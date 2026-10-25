use super::*;

#[track_caller]
fn pump_until(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let context = glib::MainContext::default();

    while !ready() && std::time::Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert!(ready(), "GTK did not reach the expected state");
}

fn screenshot(window: &ElysiaeWindow, name: &str) {
    if let Ok(directory) = std::env::var("ELYSIAE_TEST_SCREENSHOTS") {
        std::fs::create_dir_all(&directory).unwrap();
        let paintable = gtk::WidgetPaintable::new(Some(window));
        let mut node = None;
        pump_until(|| {
            if node.is_none() {
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
                node = snapshot.to_node();
            }

            node.is_some()
        });
        let texture = window
            .renderer()
            .unwrap()
            .render_texture(node.unwrap(), None);
        texture
            .save_to_png(std::path::Path::new(&directory).join(name))
            .unwrap();
    }
}

fn assert_footer_layout(window: &ElysiaeWindow) {
    let footer = window.imp().progress.parent().unwrap();
    let row = footer.parent().unwrap();
    let actions = window.imp().settings_button.parent().unwrap();
    pump_until(|| {
        let row = row.compute_bounds(window).unwrap();
        let actions = actions.compute_bounds(window).unwrap();
        (row.width() - (window.width() as f32 - 64.0)).abs() <= 2.0
            && actions.x() + actions.width() <= window.width() as f32
            && (!window.imp().progress.is_visible() || window.imp().progress.height() > 0)
    });

    let area = footer.compute_bounds(window).unwrap();
    let row = row.compute_bounds(window).unwrap();
    let controls = actions.compute_bounds(window).unwrap();

    assert!(
        (area.width() - row.width() * 0.4).abs() <= 2.0,
        "Expected a 40% progress area: {area:?} in {row:?}"
    );
    assert!(
        controls.x() >= area.x() + area.width() + 23.0,
        "Progress and controls must not overlap"
    );
    assert!(
        (controls.y() + controls.height() - area.y() - area.height()).abs() <= 2.0,
        "Progress and controls must share a bottom edge: {area:?}, {controls:?}"
    );
    assert!(controls.y() + controls.height() <= window.height() as f32);
    assert!(!window.imp().settings_button.is_ancestor(&footer));
    assert!(!window.imp().operation_button.is_ancestor(&footer));
}

#[test]
#[ignore = "requires a GTK display; run explicitly on one test thread"]
fn gtk_launcher_interactions_and_layout() {
    gtk::init().unwrap();
    gtk::gio::resources_register_include!("elysiae.gresource").unwrap();
    crate::util::settings::set_option("check-for-updates", false.into()).unwrap();

    let provider = gtk::CssProvider::new();
    let css_errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let errors = css_errors.clone();
    provider.connect_parsing_error(move |_, _, error| errors.borrow_mut().push(error.to_string()));
    provider.load_from_resource("/app/elysiae/Elysiae/style.css");
    assert!(
        css_errors.borrow().is_empty(),
        "CSS errors: {:?}",
        css_errors.borrow()
    );
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let app = gtk::Application::builder()
        .application_id("app.elysiae.Elysiae.UiTests")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();

    let window = ElysiaeWindow::new(&app);
    window.present();
    pump_until(|| window.is_mapped() && window.width() >= 1100 && window.imp().sidebar.width() > 0);
    assert!(!window.imp().progress.is_visible());
    assert!(window.imp().background.is_visible());

    let initial_selector = window.imp().sidebar.compute_bounds(&window).unwrap();
    assert!((initial_selector.center().x() - window.width() as f32 / 2.0).abs() < 2.0);
    assert!(
        initial_selector.width() >= Game::all().len() as f32 * 44.0
            && (64.0..=120.0).contains(&initial_selector.height()),
        "Selector should have comfortably spaced game buttons: {initial_selector:?}"
    );
    let settings_bounds = window
        .imp()
        .settings_button
        .compute_bounds(&window)
        .unwrap();
    let operation_bounds = window
        .imp()
        .operation_button
        .compute_bounds(&window)
        .unwrap();
    assert!(
        (62.0..=68.0).contains(&settings_bounds.width())
            && (62.0..=68.0).contains(&settings_bounds.height())
    );
    assert!(
        (228.0..=240.0).contains(&operation_bounds.width())
            && (62.0..=68.0).contains(&operation_bounds.height())
    );
    assert_footer_layout(&window);
    screenshot(&window, "launcher-desktop.png");

    let mut buttons = Vec::new();
    let mut child = window.imp().sidebar.imp().games.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        buttons.push(widget.downcast::<gtk::ToggleButton>().unwrap());
    }

    assert_eq!(buttons.len(), Game::all().len());
    for pair in buttons.windows(2) {
        let left = pair[0].compute_bounds(&window).unwrap();
        let right = pair[1].compute_bounds(&window).unwrap();
        assert!(left.width() >= 44.0 && left.height() >= 44.0);
        assert!(right.x() - left.x() - left.width() >= 9.0);
    }

    // Real artwork must not override the configured CSS geometry.
    let artwork = gtk::gdk::MemoryTexture::new(
        512,
        512,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(vec![192u8; 512 * 512 * 4]),
        512 * 4,
    );
    for button in &buttons {
        let icon = button
            .child()
            .unwrap()
            .downcast::<crate::widgets::sidebar::GameIcon>()
            .unwrap();
        let natural_width = icon.measure(gtk::Orientation::Horizontal, -1).1;
        let picture = icon.imp().picture.borrow();
        let picture = picture.as_ref().unwrap();
        picture.set_paintable(Some(&artwork));
        picture.set_visible(true);
        icon.imp()
            .fallback
            .borrow()
            .as_ref()
            .unwrap()
            .set_visible(false);
        assert!(
            icon.measure(gtk::Orientation::Horizontal, -1).1 <= natural_width,
            "Icon natural size must ignore artwork resolution: {:?}",
            icon.measure(gtk::Orientation::Horizontal, -1)
        );
    }

    buttons[2].emit_clicked();
    assert_eq!(window.get_current_game(), Game::Hkrpg);
    assert!(buttons[2].has_css_class("is-selected"));

    window.imp().settings_button.emit_clicked();
    pump_until(|| {
        window.imp().settings_layer.is_visible() && window.imp().settings_panel.width() > 0
    });
    assert!(!window.imp().content.is_sensitive());
    let panel = window
        .imp()
        .settings_panel
        .compute_bounds(&window.imp().settings_layer.get())
        .unwrap();
    let (_, natural, _, _) = window
        .imp()
        .settings_panel
        .measure(gtk::Orientation::Horizontal, -1);
    let margins =
        window.imp().settings_panel.margin_start() + window.imp().settings_panel.margin_end();
    let expected = ((natural - margins) * 2).min(window.imp().settings_layer.width() - margins);
    assert!(
        (panel.width() - expected as f32).abs() <= 2.0,
        "Settings should be double natural width, capped to the window: {panel:?}, natural={natural}, expected={expected}"
    );
    let picked = window
        .imp()
        .settings_layer
        .pick(
            panel.center().x() as f64,
            panel.center().y() as f64,
            gtk::PickFlags::DEFAULT,
        )
        .unwrap();
    assert!(
        picked != window.imp().settings_backdrop.get().upcast::<gtk::Widget>()
            && !picked.is_ancestor(&window.imp().settings_backdrop.get())
    );
    let focus = gtk::prelude::GtkWindowExt::focus(&window).unwrap();
    assert!(focus.is_ancestor(&window.imp().settings_panel.get()));
    screenshot(&window, "settings-desktop.png");

    window.imp().settings_panel.close_button().emit_clicked();
    assert!(!window.imp().settings_layer.is_visible());

    window.imp().settings_button.emit_clicked();
    window.imp().settings_backdrop.emit_clicked();
    assert!(!window.imp().settings_layer.is_visible());

    window.imp().settings_button.emit_clicked();
    let controllers = window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|i| {
            controllers
                .item(i)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .next()
        .unwrap();
    let dropdown = window.imp().settings_panel.dropdown("vo-lang");
    dropdown.popup();
    pump_until(|| dropdown.imp().popover.is_mapped());
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Escape,
            &0u32,
            &gtk::gdk::ModifierType::empty()
        ]
    ));
    pump_until(|| !dropdown.is_open());
    assert!(
        window.imp().settings_layer.is_visible(),
        "Escape closes the choices before the modal"
    );

    dropdown.popup();
    pump_until(|| dropdown.imp().popover.is_mapped());
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gtk::gdk::Key::Tab, &0u32, &gtk::gdk::ModifierType::empty()]
    ));
    pump_until(|| !dropdown.is_open());
    let focus = gtk::prelude::GtkWindowExt::focus(&window).unwrap();
    assert!(!focus.is_ancestor(&dropdown), "Tab must leave the dropdown");

    for _ in 0..16 {
        assert!(keys.emit_by_name::<bool>(
            "key-pressed",
            &[&gtk::gdk::Key::Tab, &0u32, &gtk::gdk::ModifierType::empty()]
        ));
        let focus = gtk::prelude::GtkWindowExt::focus(&window).unwrap();
        assert!(focus.is_ancestor(&window.imp().settings_panel.get()));
    }

    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Escape,
            &0u32,
            &gtk::gdk::ModifierType::empty()
        ]
    ));
    assert!(!window.imp().settings_layer.is_visible());

    window.change_game(Game::Hk4e);
    let transfer = operations()
        .begin(OperationKind::Install, Some(Game::Hkrpg))
        .unwrap();
    window.imp().status.set(Some(UpdateAvailability::Updated));
    window.imp().runtime_ready.set(true);
    window.refresh_controls();
    assert_eq!(window.imp().operation_button.label().unwrap(), "Launch");
    assert!(window.imp().operation_button.is_sensitive());

    let reporter = transfer.reporter().unwrap();
    reporter.report(ProgressEvent::Sophon(SophonProgress::Downloading {
        downloaded_bytes: 50 * 1024 * 1024 * 1024,
        total_bytes: 100 * 1024 * 1024 * 1024,
        speed_bps: 20.0 * 1024.0 * 1024.0,
        eta_seconds: 2560.0,
    }));
    pump_until(|| window.imp().progress.is_visible());
    assert_eq!(window.get_current_game(), Game::Hk4e);
    assert!(window.imp().progress.imp().verification_row.is_visible());
    let selector = window.imp().sidebar.compute_bounds(&window).unwrap();
    assert!((selector.width() - initial_selector.width()).abs() <= 1.0);
    assert!((selector.height() - initial_selector.height()).abs() <= 1.0);
    assert_footer_layout(&window);
    let icon = buttons[2]
        .child()
        .unwrap()
        .downcast::<crate::widgets::sidebar::GameIcon>()
        .unwrap();
    assert!((icon.imp().target.get() - 0.5).abs() < 0.001);
    screenshot(&window, "download-desktop.png");

    window.set_default_size(720, 540);
    pump_until(|| window.width() <= 760);
    assert_footer_layout(&window);
    screenshot(&window, "download-compact.png");

    // Pin this synthetic worst-case layout while the 250 ms status refresh
    // runs; otherwise it can hide the extra button before GTK allocates it.
    let keep_preinstall = window
        .imp()
        .preinstall_button
        .connect_visible_notify(|button| {
            if !button.is_visible() {
                button.set_visible(true);
            }
        });
    let keep_label = window
        .imp()
        .operation_button
        .connect_label_notify(|button| {
            if button.label().as_deref() != Some("Resume installation") {
                button.set_label("Resume installation");
            }
        });
    window.imp().preinstall_button.set_visible(true);
    window
        .imp()
        .operation_button
        .set_label("Resume installation");

    // Exercise a refresh during the fixture rather than relying on timing.
    window.refresh_controls();
    assert!(window.imp().preinstall_button.is_visible());
    assert_eq!(
        window.imp().operation_button.label().as_deref(),
        Some("Resume installation")
    );
    pump_until(|| window.imp().preinstall_button.height() > 0);
    assert_footer_layout(&window);
    window.imp().preinstall_button.disconnect(keep_preinstall);
    window.imp().operation_button.disconnect(keep_label);
    window.imp().preinstall_button.set_visible(false);
    window.refresh_controls();

    window.imp().settings_button.emit_clicked();
    pump_until(|| {
        window
            .imp()
            .settings_panel
            .compute_bounds(&window)
            .is_some_and(|bounds| {
                bounds.x() >= 0.0
                    && bounds.y() >= 0.0
                    && bounds.x() + bounds.width() <= window.width() as f32 + 1.0
                    && bounds.y() + bounds.height() <= window.height() as f32 + 1.0
            })
    });
    let bounds = window.imp().settings_panel.compute_bounds(&window).unwrap();
    assert!(bounds.x() >= 0.0 && bounds.y() >= 0.0);
    assert!(bounds.x() + bounds.width() <= window.width() as f32 + 1.0);
    assert!(bounds.y() + bounds.height() <= window.height() as f32 + 1.0);
    screenshot(&window, "settings-compact.png");
    window.show_settings(false);
    transfer.finish(Ok(())).unwrap();

    let component = operations().begin(OperationKind::Components, None).unwrap();
    let reporter = component.reporter().unwrap();
    reporter.report(ProgressEvent::File(crate::util::web::DownloadProgress {
        download_id: Uuid::new_v4(),
        downloaded: 25,
        total: 100,
    }));
    pump_until(|| window.imp().displayed_operation.get() == Some(reporter.0.id));
    assert!(window.imp().progress.imp().bars.is_visible());
    assert!(!window.imp().progress.imp().verification_row.is_visible());

    reporter.report(ProgressEvent::Extracting("download.tar.gz".into()));
    pump_until(|| !window.imp().progress.imp().bars.is_visible());
    assert!(!window.imp().progress.imp().heading.is_visible());
    assert_eq!(
        window.imp().progress.imp().detail.label(),
        "Extracting download.tar.gz…"
    );
    component.finish(Ok(())).unwrap();
    window.close();
}

#[test]
fn preinstall_does_not_replace_launch_action() {
    assert_eq!(
        action_kind(UpdateAvailability::Preinstall),
        OperationKind::Launch
    );
    assert_eq!(action_label(UpdateAvailability::Preinstall), "Launch");
    assert!(is_installed(UpdateAvailability::Outdated));
    assert!(!is_installed(UpdateAvailability::Incomplete));
    assert_eq!(
        action_kind(UpdateAvailability::Outdated),
        OperationKind::Update
    );
}
