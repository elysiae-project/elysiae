use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
mod app;
mod core;
mod fonts;
mod util;
mod widgets;
mod window;

/// Initializes logging, preferences and resources before running the GTK launcher.
fn main() -> glib::ExitCode {
    if let Err(error) = util::logging::init() {
        eprintln!("Could not initialize logging: {error:#}");
        return glib::ExitCode::FAILURE;
    }
    if let Err(error) = util::settings::initialize() {
        log::error!(
            "Settings initialization failed; existing preferences are unchanged: {error:#}"
        );
    }
    if is_nvidia() {
        log::info!("NVIDIA Device detected. setting the GSK renderer to Cairo");
        unsafe {
            // NVIDIA devices seem to behave oddly with gtk on linux (surprise surprise)
            // Setting the gsk renderer to cairo seems to cut it down to expected levels and
            // I personally haven't noticed any other performance issues when using cairo
            // If a better solution comes up, a PR can just be opened
            std::env::set_var("GSK_RENDERER", "cairo");
        }
    }
    gio::resources_register_include!("elysiae.gresource").expect("Failed to register resources.");

    let app = app::build_app();
    app.run()
}

fn is_nvidia() -> bool {
    // If a NVIDIA graphics card is present, one of these two paths should exist
    std::path::Path::new("/proc/driver/nvidia/version").exists()
        || std::path::Path::new("/dev/nvidia0").exists()
}
