use super::*;

#[track_caller]
fn pump_for(timeout: std::time::Duration, mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + timeout;
    let context = glib::MainContext::default();

    while !ready() && std::time::Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert!(ready(), "GTK behavior did not settle");
}

#[track_caller]
fn pump_until(ready: impl FnMut() -> bool) {
    pump_for(std::time::Duration::from_secs(8), ready);
}

#[test]
fn preinstall_does_not_replace_launch_action() {
    assert_eq!(
        action_kind(UpdateAvailability::Preinstall),
        OperationKind::Launch
    );
    assert!(is_installed(UpdateAvailability::Outdated));
    assert!(!is_installed(UpdateAvailability::Incomplete));
    assert_eq!(
        action_kind(UpdateAvailability::Outdated),
        OperationKind::Update
    );
}

#[test]
#[ignore = "requires a GTK display"]
fn game_selection_and_progress_operations() {
    gtk::init().unwrap();
    gtk::gio::resources_register_include!("elysiae.gresource").unwrap();
    crate::util::settings::set_option("check-for-updates", false.into()).unwrap();

    let app = gtk::Application::builder()
        .application_id("app.elysiae.Elysiae.BehaviorTests")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();

    let window = ElysiaeWindow::new(&app);
    window.present();
    pump_until(|| window.is_mapped());

    let selected_games = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let selected_games_sink = selected_games.clone();
    window
        .imp()
        .sidebar
        .connect_game_selected(move |game| selected_games_sink.borrow_mut().push(game));
    let game_code = Game::Hkrpg.code();
    window
        .imp()
        .sidebar
        .emit_by_name::<()>("game-selected", &[&game_code]);
    assert_eq!(window.get_current_game(), Game::Hkrpg);
    assert_eq!(*selected_games.borrow(), [Game::Hkrpg]);

    window.imp().settings_button.emit_clicked();
    pump_until(|| window.imp().settings_layer.is_visible());
    assert!(!window.imp().content.is_sensitive());
    assert!(
        gtk::prelude::GtkWindowExt::focus(&window)
            .is_some_and(|focus| focus.is_ancestor(&window.imp().settings_panel.get()))
    );
    window.imp().settings_panel.close_button().emit_clicked();
    pump_until(|| !window.imp().settings_layer.is_visible());
    assert!(window.imp().content.is_sensitive());

    window.imp().settings_button.emit_clicked();
    pump_until(|| window.imp().settings_layer.is_visible());
    window.imp().settings_backdrop.emit_clicked();
    pump_until(|| !window.imp().settings_layer.is_visible());

    window.change_game(Game::Hk4e);
    let transfer = operations()
        .begin(OperationKind::Install, Some(Game::Hkrpg))
        .unwrap();
    let transfer_id = transfer.reporter().unwrap().0.id;
    window.imp().status.set(Some(UpdateAvailability::Updated));
    window.imp().runtime_ready.set(true);
    window.refresh_controls();
    assert!(window.imp().operation_button.is_sensitive());

    let reporter = transfer.reporter().unwrap();
    reporter.report(ProgressEvent::Sophon(SophonProgress::Downloading {
        downloaded_bytes: 50 * 1024 * 1024 * 1024,
        total_bytes: 100 * 1024 * 1024 * 1024,
        speed_bps: 20.0 * 1024.0 * 1024.0,
        eta_seconds: 2560.0,
    }));
    pump_until(|| window.imp().displayed_operation.get() == Some(transfer_id));
    assert_eq!(window.get_current_game(), Game::Hk4e);
    assert!(window.imp().progress.is_presenting());
    transfer.finish(Ok(())).unwrap();
    pump_for(std::time::Duration::from_secs(8), || {
        window.imp().displayed_operation.get().is_none()
    });
    assert!(!window.imp().progress.is_presenting());

    let component = operations().begin(OperationKind::Components, None).unwrap();
    let reporter = component.reporter().unwrap();
    let component_id = reporter.0.id;
    reporter.report(ProgressEvent::File(crate::util::web::DownloadProgress {
        download_id: Uuid::new_v4(),
        downloaded: 25,
        total: 100,
    }));
    pump_until(|| window.imp().displayed_operation.get() == Some(component_id));
    assert!(window.imp().progress.is_presenting());
    reporter.report(ProgressEvent::Extracting("component.tar.gz".into()));
    component.finish(Ok(())).unwrap();
    pump_for(std::time::Duration::from_secs(8), || {
        window.imp().displayed_operation.get().is_none()
    });
    assert!(!window.imp().progress.is_presenting());

    window.close();
}

#[test]
#[ignore = "requires a GTK display"]
fn stale_terminal_event_does_not_reset_active_progress() {
    use crate::core::operations::{OperationStatus, Phase};

    gtk::init().unwrap();
    gtk::gio::resources_register_include!("elysiae.gresource").unwrap();

    let app = gtk::Application::builder()
        .application_id("app.elysiae.Elysiae.StaleProgressBehaviorTests")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();

    let window = ElysiaeWindow::new(&app);
    window.present();
    pump_until(|| window.is_mapped());

    let superseded = Uuid::new_v4();
    let launched = Uuid::new_v4();
    let in_flight = Uuid::new_v4();
    let make_status = |id, game, phase, error| OperationStatus {
        id,
        kind: OperationKind::Install,
        game: Some(game),
        phase,
        error,
    };

    progress::publish(ProgressUpdate {
        operation: make_status(superseded, Game::Bh3, Phase::Running, None),
        event: ProgressEvent::Finished { error: None },
    });
    progress::publish(ProgressUpdate {
        operation: make_status(launched, Game::Hk4e, Phase::Running, None),
        event: ProgressEvent::Started,
    });
    progress::publish(ProgressUpdate {
        operation: make_status(in_flight, Game::Bh3, Phase::Running, None),
        event: ProgressEvent::Sophon(SophonProgress::Downloading {
            downloaded_bytes: 9 * 1024 * 1024 * 1024,
            total_bytes: 24 * 1024 * 1024 * 1024,
            speed_bps: 16.0 * 1024.0 * 1024.0,
            eta_seconds: 940.0,
        }),
    });

    pump_until(|| window.imp().displayed_operation.get() == Some(in_flight));
    assert!(window.imp().progress.is_presenting());

    progress::publish(ProgressUpdate {
        operation: make_status(
            launched,
            Game::Hk4e,
            Phase::Failed,
            Some("stale failure".to_string()),
        ),
        event: ProgressEvent::Finished {
            error: Some("stale failure".to_string()),
        },
    });

    let stale_since = std::time::Instant::now();
    pump_until(|| stale_since.elapsed() > std::time::Duration::from_secs(5));
    assert_eq!(window.imp().displayed_operation.get(), Some(in_flight));
    assert!(window.imp().progress.is_presenting());

    progress::publish(ProgressUpdate {
        operation: make_status(
            in_flight,
            Game::Bh3,
            Phase::Failed,
            Some("active failure".to_string()),
        ),
        event: ProgressEvent::Finished {
            error: Some("active failure".to_string()),
        },
    });
    pump_until(|| window.imp().displayed_operation.get().is_none());
    assert!(!window.imp().progress.is_presenting());
    window.close();
}
