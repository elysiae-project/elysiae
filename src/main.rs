use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::env::set_var;

use crate::core::fs::{BaseDirectory, full_path};

mod app;
mod core;
mod util;
mod widgets;
mod window;
mod fonts;

fn main() -> glib::ExitCode {
    // Set environment variables required to get proton working
    unsafe {
        let compat_path = full_path(None, Some(BaseDirectory::Compat)).unwrap();

        set_var("STEAM_COMPAT_DATA_PATH", compat_path);
        set_var("STEAM_COMPAT_CLIENT_INSTALL_PATH", "");
    }
    gio::resources_register_include!("elysiae.gresource")
        .expect("Failed to register resources.");

    let app = app::build_app();
    app.run()
}
