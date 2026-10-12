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
    gio::resources_register_include!("elysiae.gresource").expect("Failed to register resources.");

    let app = app::build_app();
    app.run()
}
