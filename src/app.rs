use gtk::{CssProvider, gio, glib, prelude::*, style_context_add_provider_for_display};
use std::{cell::RefCell, rc::Rc, time::Duration};

use crate::{
    core::{game::Game, game_downloader, installations, operations::operations, proton_manager},
    fonts,
    util::{
        cache, runtime,
        settings::{display_locale, get_option},
    },
};

const APP_ID: &str = "app.elysiae.Elysiae";
const APP_STYLE_PRIORITY: u32 = gtk::STYLE_PROVIDER_PRIORITY_APPLICATION;
const APP_FONT: &[u8] = include_bytes!("../data/fonts/PretendardVariable.woff2");

pub fn build_app() -> gtk::Application {
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|app| {
        if let Err(error) = fonts::load_app_fonts(&[("PretendardVariable.woff2", APP_FONT)]) {
            log::error!("Could not load application fonts: {error:#}");
        }
        load_css();
        let weak = app.downgrade();
        if let Err(error) = crate::util::settings::connect_changed(move |key| {
            let Some(app) = weak.upgrade() else { return; };
            if matches!(key, "display-language" | "use-video-background") {
                if let Some(window) = app.active_window().and_downcast::<crate::window::ElysiaeWindow>() {
                    window.refresh_media();
                }
                refresh_assets(&app);
            } else if key == "check-for-updates"
                && let Some(window) = app.active_window().and_downcast::<crate::window::ElysiaeWindow>() {
                window.refresh_status();
            }
        }) { log::warn!("Could not observe preference changes: {error:#}"); }
        refresh_assets(app);
        let (ready, recovered) = async_channel::bounded(1);
        let hold = app.hold();
        glib::MainContext::default().spawn_local(async move {
            let _hold = hold;
            let _ = recovered.recv().await;
        });
        runtime::spawn(async move {
            if let Err(error) = proton_manager::recover_runtime().await {
                log::error!("Runtime recovery needs attention: {error:#}");
            }
            for game in Game::all() {
                match installations::pending(game) {
                    Ok(true) => if let Err(error) = game_downloader::recover_maintenance(game).await {
                        log::error!("Maintenance recovery for {} needs attention: {error:#}", game.code());
                    },
                    Ok(false) => {},
                    Err(error) => log::error!("Could not inspect maintenance recovery: {error:#}"),
                }
            }
            let _ = ready.send(()).await;
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                if !get_option("check-for-updates").and_then(bool::try_from).unwrap_or(false)
                    || operations().is_busy().unwrap_or(true) { continue; }
                match proton_manager::check_runtime_update().await {
                    Ok(true) => log::info!("A runtime update is available; use the check-runtime-update action to review it"),
                    Ok(false) => {},
                    Err(error) => log::warn!("Runtime update check unavailable; installed runtime remains usable: {error:#}"),
                }
            }
        });
    });

    let pending_game = Rc::new(RefCell::new(None::<Game>));
    let pending = pending_game.clone();
    app.connect_command_line(move |app, command_line| {
        for argument in command_line.arguments().iter().skip(1) {
            let Some(game) = argument.to_str().and_then(parse_game_uri) else {
                command_line.printerr_literal("Expected elysiae://open-game/<game-code>\n");
                return glib::ExitCode::FAILURE;
            };
            *pending.borrow_mut() = Some(game);
        }
        app.activate();
        glib::ExitCode::SUCCESS
    });
    let pending = pending_game.clone();
    app.connect_open(move |app, files, _| {
        for file in files {
            if let Some(game) = parse_game_uri(file.uri().as_str()) {
                *pending.borrow_mut() = Some(game);
            }
        }
        app.activate();
    });
    app.connect_activate(move |app| {
        build_ui(app);
        if let Some(game) = pending_game.borrow_mut().take()
            && let Some(window) = app
                .active_window()
                .and_downcast::<crate::window::ElysiaeWindow>()
        {
            window.change_game(game);
        }
    });
    add_actions(&app);
    app
}

pub fn refresh_assets(app: &gtk::Application) {
    let locale = display_locale().unwrap_or_else(|_| "en-us".into());
    let (sender, receiver) = async_channel::bounded(1);
    runtime::spawn(async move {
        match cache::refresh_cache(&locale).await {
            Ok(report) => {
                for (game, error) in report.failures {
                    log::warn!(
                        "Asset refresh for {} failed; retaining usable cached assets: {error}",
                        game.code()
                    );
                }
            }
            Err(error) => log::warn!("Asset refresh unavailable: {error:#}"),
        }
        let _ = sender.send(()).await;
    });
    let weak = app.downgrade();
    // Unlike MainContext::invoke, this cannot execute GTK code immediately on
    // an arbitrary worker while the main context happens to be unowned.
    glib::MainContext::default().spawn_local(async move {
        if receiver.recv().await.is_ok()
            && let Some(app) = weak.upgrade()
            && let Some(window) = app
                .active_window()
                .and_downcast::<crate::window::ElysiaeWindow>()
        {
            window.refresh_media();
        }
    });
}

fn add_actions(app: &gtk::Application) {
    for (name, control) in [
        (
            "pause-operation",
            game_downloader::toggle_downloading as fn() -> anyhow::Result<()>,
        ),
        ("cancel-operation", game_downloader::cancel_download),
    ] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| {
            if let Err(error) = control() {
                log::warn!("{name}: {error:#}");
            }
        });
        app.add_action(&action);
    }
    let action = gio::SimpleAction::new("check-runtime-update", None);
    let weak = app.downgrade();
    action.connect_activate(move |_, _| {
        let Some(app) = weak.upgrade() else { return; };
        let Some(window) = app.active_window() else { return; };
        glib::MainContext::default().spawn_local(async move {
            let result = runtime::spawn(proton_manager::check_runtime_update()).await;
            match result {
                Ok(Ok(true)) => {
                    let dialog = gtk::AlertDialog::builder().message("Install the available runtime update?")
                        .detail("The working runtime is retained until the replacement is validated. Running games and other operations must finish first.")
                        .buttons(["Cancel", "Update"]).cancel_button(0).default_button(0).build();
                    if dialog.choose_future(Some(&window)).await == Ok(1) {
                        let _hold = window.application().map(|app| app.hold());
                        match runtime::spawn(proton_manager::update_all_modules()).await {
                            Ok(Ok(())) => gtk::AlertDialog::builder().message("Runtime updated").build().show(Some(&window)),
                            result => gtk::AlertDialog::builder().message("Runtime update failed")
                                .detail(format!("{result:?}")).build().show(Some(&window)),
                        }
                    }
                }
                Ok(Ok(false)) => gtk::AlertDialog::builder().message("The runtime is current").build().show(Some(&window)),
                result => gtk::AlertDialog::builder().message("Could not check runtime updates")
                    .detail(format!("{result:?}")).build().show(Some(&window)),
            }
        });
    });
    app.add_action(&action);
}

fn parse_game_uri(uri: &str) -> Option<Game> {
    let url = url::Url::parse(uri).ok()?;
    if url.scheme() != "elysiae"
        || url.host_str()? != "open-game"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
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
    crate::window::build_window(app).present();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn game_uri_is_navigation_only_and_rejects_foreign_inputs() {
        assert_eq!(parse_game_uri("elysiae://open-game/hk4e"), Some(Game::Hk4e));
        for uri in [
            "https://open-game/hk4e",
            "elysiae://execute/hk4e",
            "elysiae://open-game/unknown",
            "elysiae://user@open-game/hk4e",
            "elysiae://open-game/hk4e?run=true",
            "elysiae://open-game/hk4e#run",
            "elysiae://open-game/hk4e/other",
        ] {
            assert_eq!(parse_game_uri(uri), None, "{uri}");
        }
    }
}
