use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::env::set_var;

use crate::core::fs::{BaseDirectory, full_path};

mod app;
mod core;
mod fonts;
mod util;
mod widgets;
mod window;

fn main() -> glib::ExitCode {
    // Set environment variables required to get proton working
    unsafe {
        let compat_path = full_path(None, Some(BaseDirectory::Compat)).unwrap();

        // If the compat data path doesn't exist when proton runs, proton will automatically create the directory when it first runs
        set_var("STEAM_COMPAT_DATA_PATH", compat_path);

        // Setting this environment variable to be an empty string is intended. Proton doesn't need the variable to work, it just needs it to be defined
        set_var("STEAM_COMPAT_CLIENT_INSTALL_PATH", "");
    }
    gio::resources_register_include!("elysiae.gresource").expect("Failed to register resources.");

    let app = app::build_app();
    app.run()
}
