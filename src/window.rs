use anyhow::Result;
use async_channel::Sender;
use gtk::{glib, prelude::*, subclass::prelude::ObjectSubclassIsExt};
use irmin::SophonProgress;
use uuid::Uuid;

use crate::{
    core::{
        game::Game,
        game_downloader::{self, UpdateAvailability},
        operations::{OperationKind, operations},
        proton_manager,
    },
    util::{
        progress::{self, ProgressEvent, ProgressUpdate},
        runtime,
        settings::get_option,
    },
};

mod imp {
    use super::*;
    use crate::widgets::{
        background::Background, progressbar::Progressbar, settings::Settings, sidebar::Sidebar,
        titlebar::Titlebar,
    };
    use gtk::{CompositeTemplate, subclass::prelude::*};
    use std::cell::{Cell, RefCell};

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/window.ui")]
    pub struct ElysiaeWindow {
        #[template_child]
        pub settings_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub operation_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub preinstall_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub background: TemplateChild<Background>,
        #[template_child]
        pub sidebar: TemplateChild<Sidebar>,
        #[template_child]
        pub progress: TemplateChild<Progressbar>,
        #[template_child]
        pub game_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub activity_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub content: TemplateChild<gtk::Box>,
        #[template_child]
        pub settings_layer: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub settings_backdrop: TemplateChild<gtk::Button>,
        #[template_child]
        pub settings_panel: TemplateChild<Settings>,
        pub current_game: Cell<Game>,
        pub status: Cell<Option<UpdateAvailability>>,
        pub request: Cell<u64>,
        pub installation_request: Cell<u64>,
        pub runtime_ready: Cell<bool>,
        pub transfer_pending: Cell<Option<Game>>,
        pub launch_pending: Cell<Option<Game>>,
        pub displayed_operation: Cell<Option<Uuid>>,
        pub completed: RefCell<Vec<Uuid>>,
        pub previous_focus: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
        pub active_ids: RefCell<Vec<(Uuid, OperationKind)>>,
        pub progress_receiver: RefCell<Option<async_channel::Receiver<ProgressUpdate>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ElysiaeWindow {
        const NAME: &'static str = "ElysiaeWindow";
        type Type = super::ElysiaeWindow;
        type ParentType = gtk::ApplicationWindow;
        fn class_init(klass: &mut Self::Class) {
            Titlebar::static_type();
            Background::static_type();
            Sidebar::static_type();
            Progressbar::static_type();
            Settings::static_type();
            klass.bind_template();
        }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }
    impl ObjectImpl for ElysiaeWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let window = self.obj();
            let weak = window.downgrade();
            self.operation_button.connect_clicked(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.start_operation(false);
                }
            });
            let weak = window.downgrade();
            self.preinstall_button.connect_clicked(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.start_operation(true);
                }
            });
            let weak = window.downgrade();
            self.sidebar.connect_game_selected(move |game| {
                if let Some(window) = weak.upgrade() {
                    window.change_game(game);
                }
            });
            let weak = window.downgrade();
            self.settings_button.connect_clicked(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.show_settings(true);
                }
            });
            for button in [
                self.settings_backdrop.get(),
                self.settings_panel.close_button(),
            ] {
                let weak = window.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(window) = weak.upgrade() {
                        window.show_settings(false);
                    }
                });
            }
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = window.downgrade();
            keys.connect_key_pressed(move |_, key, _, modifiers| {
                let Some(window) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if !window.imp().settings_layer.is_visible() {
                    return glib::Propagation::Proceed;
                }
                if key == gtk::gdk::Key::Escape {
                    window.show_settings(false);
                    return glib::Propagation::Stop;
                }
                if key == gtk::gdk::Key::Tab || key == gtk::gdk::Key::ISO_Left_Tab {
                    let direction = if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
                        || key == gtk::gdk::Key::ISO_Left_Tab
                    {
                        gtk::DirectionType::TabBackward
                    } else {
                        gtk::DirectionType::TabForward
                    };
                    if !window.imp().settings_panel.child_focus(direction) {
                        gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
                        window.imp().settings_panel.child_focus(direction);
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(keys);
            let receiver = progress::subscribe();
            *self.progress_receiver.borrow_mut() = Some(receiver.clone());
            let weak = window.downgrade();
            glib::MainContext::default().spawn_local(async move {
                while let Ok(update) = receiver.recv().await {
                    let Some(window) = weak.upgrade() else {
                        break;
                    };
                    window.handle_progress(update);
                }
            });
            let weak = window.downgrade();
            glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                let Some(window) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let ids = operations()
                    .active()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| (s.id, s.kind))
                    .collect::<Vec<_>>();
                let changed = *window.imp().active_ids.borrow() != ids;
                if changed {
                    *window.imp().active_ids.borrow_mut() = ids;
                    window.refresh_status();
                }
                window.refresh_controls();
                glib::ControlFlow::Continue
            });
            self.progress.reset();
        }
        fn dispose(&self) {
            if let Some(receiver) = self.progress_receiver.borrow_mut().take() {
                receiver.close();
            }
            self.dispose_template();
        }
    }
    impl WidgetImpl for ElysiaeWindow {}
    impl WindowImpl for ElysiaeWindow {}
    impl ApplicationWindowImpl for ElysiaeWindow {}
}

glib::wrapper! {
    pub struct ElysiaeWindow(ObjectSubclass<imp::ElysiaeWindow>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
            gtk::ShortcutManager, gtk::gio::ActionGroup, gtk::gio::ActionMap;
}

impl ElysiaeWindow {
    pub fn new(app: &gtk::Application) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();
        window.change_game(Game::default());
        window.imp().sidebar.refresh_icons();
        window.refresh_installations();
        window
    }
    pub fn get_current_game(&self) -> Game {
        self.imp().current_game.get()
    }

    pub fn change_game(&self, game: Game) {
        self.imp().current_game.set(game);
        self.imp().sidebar.select(game);
        self.imp().game_title.set_label(game.display_name());
        self.imp().status.set(None);
        self.refresh_media();
        self.refresh_status();
    }

    pub fn refresh_media(&self) {
        self.imp().background.set_media(self.get_current_game());
    }

    pub fn refresh_artwork(&self) {
        self.imp().sidebar.refresh_icons();
        self.refresh_media();
    }

    /// An incremental cache publication need not reload unrelated artwork.
    pub fn assets_ready(&self, game: Game) {
        self.imp().sidebar.refresh_icon(game);
        if self.get_current_game() == game {
            self.refresh_media();
        }
    }

    fn show_settings(&self, visible: bool) {
        if visible == self.imp().settings_layer.is_visible() {
            return;
        }
        if visible {
            *self.imp().previous_focus.borrow_mut() =
                gtk::prelude::GtkWindowExt::focus(self).map(|w| w.downgrade());
        }
        self.imp().content.set_sensitive(!visible);
        self.imp().content.set_can_target(!visible);
        self.imp().settings_layer.set_visible(visible);
        if visible {
            self.imp().settings_panel.focus_first();
        } else if let Some(widget) = self
            .imp()
            .previous_focus
            .borrow_mut()
            .take()
            .and_then(|w| w.upgrade())
            .filter(|w| w.is_sensitive() && w.is_visible())
        {
            widget.grab_focus();
        } else {
            self.imp().settings_button.grab_focus();
        }
    }

    fn refresh_controls(&self) {
        let imp = self.imp();
        let game = self.get_current_game();
        let active = operations().active().unwrap_or_default();
        let running = active
            .iter()
            .find(|s| s.kind == OperationKind::Launch)
            .and_then(|s| s.game);
        let transfer = active.iter().any(|s| s.kind != OperationKind::Launch)
            || imp.transfer_pending.get().is_some();
        let Some(status) = imp.status.get() else {
            imp.operation_button.set_sensitive(false);
            imp.preinstall_button.set_visible(false);
            return;
        };
        let kind = action_kind(status);
        let launching = kind == OperationKind::Launch;
        let pending = if launching {
            imp.launch_pending.get().is_some()
        } else {
            imp.transfer_pending.get().is_some()
        };
        let ready = !launching || !transfer || imp.runtime_ready.get();
        let allowed =
            !pending && ready && operations().can_begin(kind, Some(game)).unwrap_or(false);
        imp.operation_button.set_sensitive(allowed);
        imp.operation_button.set_label(if running == Some(game) {
            "Running…"
        } else if imp.launch_pending.get() == Some(game) {
            "Launching…"
        } else if !launching
            && (imp.transfer_pending.get() == Some(game)
                || active
                    .iter()
                    .any(|s| s.game == Some(game) && s.kind != OperationKind::Launch))
        {
            "Working…"
        } else {
            action_label(status)
        });
        imp.operation_button.set_tooltip_text(if !ready { Some("The shared runtime needs setup. It can be prepared when the current download finishes.") } else { None });
        imp.preinstall_button
            .set_visible(status == UpdateAvailability::Preinstall);
        imp.preinstall_button.set_sensitive(
            imp.transfer_pending.get().is_none()
                && operations()
                    .can_begin(OperationKind::Preinstall, Some(game))
                    .unwrap_or(false),
        );
        imp.activity_label
            .set_label(&if let Some(running) = running {
                format!("{} is running", running.display_name())
            } else {
                match status {
                    UpdateAvailability::Updated | UpdateAvailability::Untracked => {
                        "Ready to play".into()
                    }
                    UpdateAvailability::Preinstall => "Ready to play · preinstall available".into(),
                    UpdateAvailability::Outdated => "An update is available".into(),
                    UpdateAvailability::Incomplete => "Installation incomplete".into(),
                    UpdateAvailability::NotInstalled => "Not installed".into(),
                }
            });
    }

    /// Local status appears first; remote checks cannot replace a newer selection.
    pub fn refresh_status(&self) {
        let game = self.get_current_game();
        let previous = self.imp().status.get();
        let request = self.imp().request.get().wrapping_add(1);
        self.imp().request.set(request);
        if previous.is_none() {
            self.imp().operation_button.set_label("Checking…");
        }
        let lang = get_option("vo-lang")
            .and_then(String::try_from)
            .unwrap_or_else(|_| "en".into());
        let check = get_option("check-for-updates")
            .and_then(bool::try_from)
            .unwrap_or(false);
        let (sender, receiver) = async_channel::bounded(2);
        runtime::spawn(async move {
            let local = tokio::task::spawn_blocking(move || -> Result<_> {
                Ok((
                    game_downloader::local_update_status(game)?,
                    proton_manager::runtime_ready_for_launch().unwrap_or(false),
                ))
            })
            .await;
            match local {
                Ok(Ok((status, ready))) => {
                    let shown = if status == UpdateAvailability::Updated
                        && matches!(
                            previous,
                            Some(UpdateAvailability::Outdated | UpdateAvailability::Preinstall)
                        ) {
                        previous.unwrap()
                    } else {
                        status
                    };
                    let _ = sender.send(Ok((shown, ready))).await;
                    if status == UpdateAvailability::Updated
                        && check
                        && operations()
                            .can_begin(OperationKind::Launch, Some(game))
                            .unwrap_or(false)
                    {
                        match game_downloader::update_status(game, &lang).await {
                            Ok(status) => {
                                let _ = sender.send(Ok((status, ready))).await;
                            }
                            Err(error) => log::warn!(
                                "Update check unavailable; retaining local status: {error:#}"
                            ),
                        }
                    }
                }
                Ok(Err(error)) => {
                    let _ = sender.send(Err(error.to_string())).await;
                }
                Err(error) => {
                    let _ = sender.send(Err(error.to_string())).await;
                }
            }
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(result) = receiver.recv().await {
                let Some(window) = weak.upgrade() else {
                    break;
                };
                if window.imp().request.get() != request {
                    break;
                }
                match result {
                    Ok((status, ready)) => {
                        window.imp().status.set(Some(status));
                        window.imp().runtime_ready.set(ready);
                        window
                            .imp()
                            .sidebar
                            .set_installed(game, is_installed(status));
                        window.refresh_controls();
                    }
                    Err(error) => {
                        window.imp().status.set(None);
                        window.imp().operation_button.set_label("Unavailable");
                        window.imp().operation_button.set_sensitive(false);
                        window
                            .imp()
                            .activity_label
                            .set_label("Could not read installation status");
                        window.imp().operation_button.set_tooltip_text(Some(&error));
                    }
                }
            }
        });
    }

    fn refresh_installations(&self) {
        let request = self.imp().installation_request.get().wrapping_add(1);
        self.imp().installation_request.set(request);
        let (sender, receiver) = async_channel::bounded(1);
        runtime::spawn(async move {
            let result = tokio::task::spawn_blocking(|| {
                Game::all()
                    .into_iter()
                    .map(|game| {
                        (
                            game,
                            game_downloader::local_update_status(game).is_ok_and(is_installed),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let _ = sender.send(result).await;
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            if let Ok(Ok(states)) = receiver.recv().await
                && let Some(window) = weak.upgrade()
                && window.imp().installation_request.get() == request
            {
                for (game, installed) in states {
                    window.imp().sidebar.set_installed(game, installed);
                }
            }
        });
    }

    fn start_operation(&self, preinstall: bool) {
        let Some(status) = self.imp().status.get() else {
            return;
        };
        let game = self.get_current_game();
        let kind = if preinstall {
            OperationKind::Preinstall
        } else {
            action_kind(status)
        };
        let launching = kind == OperationKind::Launch;
        if preinstall && status != UpdateAvailability::Preinstall {
            return;
        }
        if !operations().can_begin(kind, Some(game)).unwrap_or(false)
            || (if launching {
                self.imp().launch_pending.get().is_some()
            } else {
                self.imp().transfer_pending.get().is_some()
            })
        {
            return;
        }
        if launching {
            self.imp().launch_pending.set(Some(game));
        } else {
            self.imp().transfer_pending.set(Some(game));
        }
        self.refresh_controls();
        let lang = get_option("vo-lang")
            .and_then(String::try_from)
            .unwrap_or_else(|_| "en".into());
        let (done, completion) = async_channel::bounded(1);
        runtime::spawn(async move {
            // Legacy backend API remains available; the UI receives identified
            // events through the shared reporter instead of an anonymous channel.
            let (sender, _receiver) = async_channel::bounded(1);
            let result = match kind {
                OperationKind::Launch => game_downloader::launch_game(game).await,
                OperationKind::Preinstall => {
                    game_downloader::preinstall_game(game, &lang, sender).await
                }
                OperationKind::Update => {
                    game_downloader::download_update(game, &lang, sender).await
                }
                _ => game_downloader::download_game(game, &lang, sender).await,
            };
            let _ = done.send(result).await;
        });
        let weak = self.downgrade();
        let hold = self.application().map(|app| app.hold());
        glib::MainContext::default().spawn_local(async move {
            let _hold = hold;
            let result = completion.recv().await;
            let Some(window) = weak.upgrade() else {
                return;
            };
            if launching {
                window.imp().launch_pending.set(None);
            } else {
                window.imp().transfer_pending.set(None);
            }
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => window.show_error(&format!("{error:#}")),
                Err(error) => window.show_error(&format!("Operation worker disconnected: {error}")),
            }
            window.refresh_status();
            window.refresh_installations();
        });
    }

    fn show_error(&self, detail: &str) {
        log::error!("Operation failed: {detail}");
        gtk::AlertDialog::builder()
            .message("Operation failed")
            .detail(detail)
            .build()
            .show(Some(self));
    }

    fn handle_progress(&self, update: ProgressUpdate) {
        let id = update.operation.id;
        if !matches!(
            update.operation.kind,
            OperationKind::Install
                | OperationKind::Update
                | OperationKind::Preinstall
                | OperationKind::Repair
                | OperationKind::Components
        ) {
            return;
        }
        if self.imp().completed.borrow().contains(&id) {
            return;
        }
        if update.operation.kind == OperationKind::Components
            && matches!(update.event, ProgressEvent::Started)
        {
            return;
        }
        if matches!(update.event, ProgressEvent::Finished { .. })
            && self.imp().displayed_operation.get() != Some(id)
        {
            return;
        }
        if self.imp().displayed_operation.replace(Some(id)) != Some(id) {
            self.imp().progress.reset();
        }
        self.imp().progress.update(&update);
        if let Some(game) = update.operation.game {
            match &update.event {
                ProgressEvent::Started => self.imp().sidebar.set_progress(game, Some(0.0)),
                ProgressEvent::Sophon(
                    SophonProgress::Downloading {
                        downloaded_bytes,
                        total_bytes,
                        ..
                    }
                    | SophonProgress::Paused {
                        downloaded_bytes,
                        total_bytes,
                    },
                ) if *total_bytes > 0 => {
                    self.imp()
                        .sidebar
                        .set_progress(game, Some(*downloaded_bytes as f64 / *total_bytes as f64));
                }
                ProgressEvent::Finished { error } => {
                    if error.is_none() {
                        self.imp().sidebar.set_installed(game, true);
                    }
                    self.imp().sidebar.set_progress(game, None);
                    self.refresh_installations();
                }
                _ => {}
            }
        }
        if matches!(update.event, ProgressEvent::Finished { .. }) {
            let mut completed = self.imp().completed.borrow_mut();
            completed.push(id);
            if completed.len() > 32 {
                completed.remove(0);
            }
            drop(completed);
            self.refresh_status();
            if matches!(update.event, ProgressEvent::Finished { error: None }) {
                let weak = self.downgrade();
                glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                    if let Some(window) = weak.upgrade()
                        && window.imp().displayed_operation.get() == Some(id)
                    {
                        window.imp().progress.reset();
                        window.imp().displayed_operation.set(None);
                    }
                });
            }
        }
    }

    pub async fn download_components(&self) -> Result<()> {
        proton_manager::update_all_modules().await
    }
    pub async fn download_game(
        &self,
        game: Game,
        lang: &str,
        sender: Sender<SophonProgress>,
    ) -> Result<()> {
        game_downloader::download_game(game, lang, sender).await
    }
    pub async fn update_game(
        &self,
        game: Game,
        lang: &str,
        sender: Sender<SophonProgress>,
    ) -> Result<()> {
        game_downloader::download_update(game, lang, sender).await
    }
    pub async fn launch_game(&self, game: Game) -> Result<()> {
        game_downloader::launch_game(game).await
    }
}

fn is_installed(status: UpdateAvailability) -> bool {
    matches!(
        status,
        UpdateAvailability::Updated
            | UpdateAvailability::Preinstall
            | UpdateAvailability::Outdated
            | UpdateAvailability::Untracked
    )
}

fn action_kind(status: UpdateAvailability) -> OperationKind {
    match status {
        UpdateAvailability::Updated
        | UpdateAvailability::Preinstall
        | UpdateAvailability::Untracked => OperationKind::Launch,
        UpdateAvailability::Outdated => OperationKind::Update,
        UpdateAvailability::NotInstalled | UpdateAvailability::Incomplete => OperationKind::Install,
    }
}

fn action_label(status: UpdateAvailability) -> &'static str {
    match status {
        UpdateAvailability::Updated
        | UpdateAvailability::Preinstall
        | UpdateAvailability::Untracked => "Launch",
        UpdateAvailability::Outdated => "Update",
        UpdateAvailability::NotInstalled => "Download",
        UpdateAvailability::Incomplete => "Resume installation",
    }
}

pub fn build_window(app: &gtk::Application) -> ElysiaeWindow {
    ElysiaeWindow::new(app)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            let node = snapshot.to_node().expect("Window must render");
            let texture = window.renderer().unwrap().render_texture(&node, None);
            texture
                .save_to_png(std::path::Path::new(&directory).join(name))
                .unwrap();
        }
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
        provider
            .connect_parsing_error(move |_, _, error| errors.borrow_mut().push(error.to_string()));
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
        pump_until(|| {
            window.is_mapped() && window.width() >= 1100 && window.imp().sidebar.width() > 0
        });
        assert!(!window.imp().progress.is_visible());
        assert!(window.imp().background.is_visible());
        let selector = window.imp().sidebar.compute_bounds(&window).unwrap();
        assert!((selector.center().x() - window.width() as f32 / 2.0).abs() < 2.0);
        screenshot(&window, "launcher-desktop.png");
        let mut buttons = Vec::new();
        let mut child = window.imp().sidebar.imp().games.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            buttons.push(widget.downcast::<gtk::ToggleButton>().unwrap());
        }
        assert_eq!(buttons.len(), Game::all().len());
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
            downloaded_bytes: 50,
            total_bytes: 100,
            speed_bps: 10.0,
            eta_seconds: 5.0,
        }));
        pump_until(|| window.imp().progress.is_visible());
        assert_eq!(window.get_current_game(), Game::Hk4e);
        assert!(window.imp().progress.imp().verification_row.is_visible());
        let icon = buttons[2]
            .child()
            .unwrap()
            .downcast::<crate::widgets::sidebar::GameIcon>()
            .unwrap();
        assert!((icon.imp().target.get() - 0.5).abs() < 0.001);
        screenshot(&window, "download-desktop.png");
        window.set_default_size(720, 540);
        pump_until(|| window.width() <= 760);
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
}
