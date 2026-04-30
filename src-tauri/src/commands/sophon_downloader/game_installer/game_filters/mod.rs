mod hk4e;
mod hkrpg;
mod nap;

pub use hk4e::{
    filter_hk4e_asset_list, write_audio_lang_record as write_hk4e_audio_lang_record,
    write_pkg_version_from_manifest,
};
pub use hkrpg::{
    filter_hkrpg_asset_list, write_app_info as write_hkrpg_app_info,
    write_audio_lang_record as write_hkrpg_audio_lang_record,
    write_binary_version_files as write_hkrpg_binary_version_files,
};
pub use nap::{filter_nap_asset_list, filter_nap_installers, write_nap_audio_lang_records};

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

pub(crate) fn write_lang_file(
    path: &Path,
    vo_langs: &[String],
    mapper: fn(&str) -> Option<&'static str>,
) -> std::io::Result<()> {
    let mut existing: Vec<String> = Vec::new();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    existing.push(trimmed.to_string());
                }
            }
        }
    }

    for lang in vo_langs {
        if let Some(name) = mapper(lang) {
            if !existing.iter().any(|e| e == name) {
                existing.push(name.to_string());
            }
        }
    }

    let mut content = String::new();
    for name in &existing {
        content.push_str(name);
        content.push('\n');
    }

    let mut file = File::create(path)?;
    file.write_all(content.as_bytes())?;

    Ok(())
}
