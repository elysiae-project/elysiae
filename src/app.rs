use gtk::{
    CssProvider,
    gio::prelude::ApplicationExt,
    prelude::{GtkApplicationExt, GtkWindowExt, ObjectExt},
    style_context_add_provider_for_display,
};

use crate::fonts;

const APP_ID: &str = "app.elysiae.Elysiae";
const APP_STYLE_PRIORITY: u32 = gtk::STYLE_PROVIDER_PRIORITY_APPLICATION;
const APP_FONT: &[u8] = include_bytes!("../data/fonts/PretendardVariable.woff2");

pub fn build_app() -> gtk::Application {
    let app = gtk::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        if let Err(error) = fonts::load_app_fonts(&[("PretendardVariable.woff2", APP_FONT)]) {
            log::error!("Could not load application fonts: {error:#}");
        }
        load_css();
    });

    app.connect_activate(build_ui);
    app
}

fn load_css() {
    let display = gtk::gdk::Display::default().expect("Could not connect to a display");
    let provider = CssProvider::new();

    gtk::Settings::for_display(&display)
        .bind_property(
            "gtk-interface-color-scheme",
            &provider,
            "prefers-color-scheme",
        )
        .sync_create()
        .build();

    provider.load_from_resource("/app/elysiae/Elysiae/style.css");
    style_context_add_provider_for_display(&display, &provider, APP_STYLE_PRIORITY);
}

fn build_ui(app: &gtk::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    let window = crate::window::build_window(app);
    window.present();
}
