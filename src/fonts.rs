use std::ffi::CString;
use std::os::fd::AsRawFd;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::OnceLock;

use memfd::MemfdOptions;

// Keeps the memfd-backed Files alive for the whole process lifetime.
// If these are dropped, the fds close and fontconfig/FreeType will fail
// to reopen the font later (e.g. on a font cache reload).
static FONT_HANDLES: OnceLock<Vec<std::fs::File>> = OnceLock::new();

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcInitLoadConfigAndFonts() -> *mut c_void;
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const c_char) -> c_int;
}

#[link(name = "pangoft2-1.0")]
unsafe extern "C" {
    fn pango_fc_font_map_set_config(fcfontmap: *mut c_void, config: *mut c_void);
}

// pango_cairo_font_map_new_for_font_type / pango_cairo_font_map_set_default
// are already satisfied by libpangocairo-1.0, which gtk4-rs links for you.
unsafe extern "C" {
    fn pango_cairo_font_map_new_for_font_type(fonttype: c_int) -> *mut c_void;
    fn pango_cairo_font_map_set_default(fontmap: *mut c_void);
}

const CAIRO_FONT_TYPE_FT: c_int = 1;

/// Writes `bytes` into an anonymous in-memory file (memfd) and returns
/// the `/proc/self/fd/N` path fontconfig can open it through.
/// The backing File is kept alive in `FONT_HANDLES` for the process lifetime.
fn memfd_path_for(
    bytes: &[u8],
    name: &str,
    handles: &mut Vec<std::fs::File>,
) -> anyhow::Result<String> {
    let mfd = MemfdOptions::default().create(name)?;

    {
        use std::io::Write;
        mfd.as_file().write_all(bytes)?;
    }

    let file = mfd.into_file();
    let path = format!("/proc/self/fd/{}", file.as_raw_fd());
    handles.push(file); // keep fd open — do NOT drop before fontconfig reads it
    Ok(path)
}

/// Registers the given (name, bytes) font pairs as application-private
/// fonts via fontconfig, backed by memfds instead of real temp files,
/// then makes them Pango's default font map for the whole process.
///
/// Must be called before any CSS provider is loaded or widgets are realized.
pub fn load_app_fonts(fonts: &[(&str, &'static [u8])]) -> anyhow::Result<()> {
    if FONT_HANDLES.get().is_some() {
        return Ok(());
    }
    let mut handles = Vec::with_capacity(fonts.len());

    // SAFETY: FcInitLoadConfigAndFonts / FcConfigAppFontAddFile /
    // pango_cairo_font_map_new_for_font_type / pango_fc_font_map_set_config /
    // pango_cairo_font_map_set_default are all valid C functions from
    // fontconfig/pango that are safe to call with well-formed, non-null
    // arguments as constructed below (paths are valid CStrings from
    // /proc/self/fd, config/fontmap pointers are non-null on success).
    unsafe {
        let config = FcInitLoadConfigAndFonts();
        anyhow::ensure!(!config.is_null(), "failed to init fontconfig config");

        for (name, bytes) in fonts {
            let path = memfd_path_for(bytes, name, &mut handles)?;
            let c_path = CString::new(path)?;
            anyhow::ensure!(
                FcConfigAppFontAddFile(config, c_path.as_ptr()) != 0,
                "failed to register font: {name}"
            );
        }

        let fontmap = pango_cairo_font_map_new_for_font_type(CAIRO_FONT_TYPE_FT);
        anyhow::ensure!(!fontmap.is_null(), "failed to create FT-backed font map");

        pango_fc_font_map_set_config(fontmap, config);
        pango_cairo_font_map_set_default(fontmap);
    }

    let _ = FONT_HANDLES.set(handles);
    Ok(())
}
