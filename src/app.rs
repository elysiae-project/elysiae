use gtk::gio::prelude::ApplicationCommandLineExt;
use gtk::prelude::Cast;
use gtk::{
    glib,
    CssProvider,
    gio::prelude::{ApplicationExt, ApplicationExtManual},
    prelude::{GtkApplicationExt, GtkWindowExt, ObjectExt},
    style_context_add_provider_for_display,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    core::game::Game,
    fonts,
    util::{cache, settings::{SettingValue, get_option}, runtime},
};

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

    let pending_game = Rc::new(RefCell::new(None::<Game>));
    let pending_game_for_command = pending_game.clone();
    app.connect_command_line(move |app, command_line| {
        if let Some(uri) = command_line.arguments().get(1)
            && let Some(uri) = uri.to_str()
            && let Some(game) = parse_game_uri(uri)
        {
            *pending_game_for_command.borrow_mut() = Some(game);
        }
        app.activate();
        glib::ExitCode::SUCCESS
    });

    let pending_game_for_activate = pending_game.clone();
    app.connect_activate(move |app| {
        build_ui(app);
        if let Some(game) = pending_game_for_activate.borrow_mut().take()
            && let Some(window) = app.active_window()
            && let Ok(window) = window.downcast::<crate::window::ElysiaeWindow>()
        {
            window.change_game(game);
        }
    });

    app.connect_startup(|_| {
        runtime::spawn(async {
            let locale = match get_option("display-language") {
                Ok(SettingValue::Str(value)) => value,
                _ => "en-us".to_string(),
            };
            if let Err(error) = cache::update_cache().await.and_then(|_| {
                if cache::cache_available(&locale)? {
                    Ok(())
                } else {
                    anyhow::bail!("asset cache is incomplete")
                }
            }) {
                log::error!("Asset cache unavailable: {error:#}");
            }
        });
    });
    app
}

fn parse_game_uri(uri: &str) -> Option<Game> {
    let url = url::Url::parse(uri).ok()?;
    if url.scheme() != "elysiae" || url.host_str()? != "open-game" {
        return None;
    }
    Game::try_from(url.path().trim_matches('/')).ok()
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
