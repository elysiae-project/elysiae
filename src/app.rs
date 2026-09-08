use gtk::{
    CssProvider,
    gio::prelude::ApplicationExt,
    prelude::{GtkApplicationExt, GtkWindowExt, ObjectExt},
    style_context_add_provider_for_display,
};

use crate::fonts;

const APP_ID: &'static str = "app.elysiae.Elysiae";
const OVERRIDE_EVERY_OTHER_THEME_THAT_COULD_BE_DEFINED_BY_A_USER_PRIORITY: u32 = u32::MAX; // lol
const APP_FONT: &[u8] = include_bytes!("../data/PretendardVariable.woff2");

pub fn build_app() -> gtk::Application {
    let app = gtk::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        fonts::load_app_fonts(&[("PretendardVariable.woff2", APP_FONT)]);
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
    style_context_add_provider_for_display(
        &display,
        &provider,
        OVERRIDE_EVERY_OTHER_THEME_THAT_COULD_BE_DEFINED_BY_A_USER_PRIORITY,
    );
}

fn build_ui(app: &gtk::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    let window = crate::window::build_window(app);
    window.present();
}
