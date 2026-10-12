use anyhow::Result;
use async_channel::Sender;
use gtk::{glib, prelude::*, subclass::prelude::ObjectSubclassIsExt};
use irmin::SophonProgress;

use crate::{
    core::{
        game::Game,
        game_downloader::{self, UpdateAvailability},
        operations::operations,
        proton_manager,
    },
    util::{runtime, settings::get_option},
    widgets::progressbar::{DownloadEvent, Progressbar},
};

mod imp {
    use crate::{
        core::{game::Game, game_downloader::UpdateAvailability},
        widgets::{background::Background, titlebar::Titlebar},
    };
    use gtk::{CompositeTemplate, glib, prelude::*, subclass::prelude::*};
    use std::cell::Cell;

    /// Template controls and request state for the main launcher window.
    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/window.ui")]
    pub struct ElysiaeWindow {
        #[template_child]
        pub settings_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub operation_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub background: TemplateChild<Background>,
        pub current_game: Cell<Game>,
        pub status: Cell<Option<UpdateAvailability>>,
        pub request: Cell<u64>,
        pub busy: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ElysiaeWindow {
        const NAME: &'static str = "ElysiaeWindow";
        type Type = super::ElysiaeWindow;
        type ParentType = gtk::ApplicationWindow;
        /// Registers child widget types and binds the window template.
        fn class_init(klass: &mut Self::Class) {
            Titlebar::static_type();
            Background::static_type();
            klass.bind_template();
        }
        /// Instantiates the template children for this window.
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }
    impl ObjectImpl for ElysiaeWindow {
        /// Connects the primary button and keeps it in sync with backend operations.
        fn constructed(&self) {
            self.parent_constructed();
            let weak = self.obj().downgrade();
            self.operation_button.connect_clicked(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.start_operation();
                }
            });
            let weak = self.obj().downgrade();
            glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
                let Some(window) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                window.refresh_controls();
                glib::ControlFlow::Continue
            });
        }
    }
    impl WidgetImpl for ElysiaeWindow {}
    impl WindowImpl for ElysiaeWindow {}
    impl ApplicationWindowImpl for ElysiaeWindow {}
}

glib::wrapper! {
    /// Main window for game selection, installation status and backend actions.
    pub struct ElysiaeWindow(ObjectSubclass<imp::ElysiaeWindow>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
            gtk::ShortcutManager, gtk::gio::ActionGroup, gtk::gio::ActionMap;
}

impl ElysiaeWindow {
    /// Creates an application window with the initial game selected.
    pub fn new(app: &gtk::Application) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();
        window.change_game(Game::Bh3);
        window
    }
    /// Returns the game currently selected in this window.
    pub fn get_current_game(&self) -> Game {
        self.imp().current_game.get()
    }
    /// Selects a game and refreshes its background and installation status.
    pub fn change_game(&self, game: Game) {
        log::info!("Selected game: {}", game.code());
        self.imp().current_game.set(game);
        self.refresh_media();
        self.refresh_status();
    }
    /// Reloads the selected game's background using current preferences.
    pub fn refresh_media(&self) {
        self.imp().background.set_media(self.get_current_game());
    }

    /// Updates the primary button while respecting active backend operations.
    fn refresh_controls(&self) {
        let busy = self.imp().busy.get() || operations().is_busy().unwrap_or(true);
        self.imp()
            .operation_button
            .set_sensitive(!busy && self.imp().status.get().is_some());
        if !busy && let Some(status) = self.imp().status.get() {
            self.imp().operation_button.set_label(action_label(status));
        }
    }

    /// Refreshes local status and optionally checks for online updates.
    /// Responses from superseded requests are ignored.
    pub fn refresh_status(&self) {
        let game = self.get_current_game();
        let request = self.imp().request.get().wrapping_add(1);
        self.imp().request.set(request);
        self.imp().status.set(None);
        self.imp().operation_button.set_label("Checking…");
        self.imp().operation_button.set_sensitive(false);
        let lang = get_option("vo-lang")
            .and_then(String::try_from)
            .unwrap_or_else(|_| "en".into());
        let check = get_option("check-for-updates")
            .and_then(bool::try_from)
            .unwrap_or(false);
        let (sender, receiver) = async_channel::bounded(2);
        runtime::spawn(async move {
            let local =
                tokio::task::spawn_blocking(move || game_downloader::local_update_status(game))
                    .await;
            match local {
                Ok(Ok(status)) => {
                    let _ = sender.send(Ok(status)).await;
                    if status == UpdateAvailability::Updated
                        && check
                        && !operations().is_busy().unwrap_or(true)
                    {
                        match game_downloader::update_status(game, &lang).await {
                            Ok(status) => {
                                let _ = sender.send(Ok(status)).await;
                            }
                            Err(error) => log::warn!(
                                "Game update check unavailable; offline launch remains available: {error:#}"
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
                    Ok(status) => {
                        window.imp().status.set(Some(status));
                        if !window.imp().busy.get() {
                            window
                                .imp()
                                .operation_button
                                .set_label(action_label(status));
                            window
                                .imp()
                                .operation_button
                                .set_sensitive(!operations().is_busy().unwrap_or(true));
                            window.imp().operation_button.set_tooltip_text(None);
                        }
                    }
                    Err(error) => {
                        window.imp().operation_button.set_label("Unavailable");
                        window.imp().operation_button.set_tooltip_text(Some(&error));
                    }
                }
            }
        });
    }

    /// Starts the selected game's current action and forwards its progress.
    /// Keeps the application alive until the backend operation finishes.
    fn start_operation(&self) {
        if self.imp().busy.get() || operations().is_busy().unwrap_or(true) {
            return;
        }
        let Some(status) = self.imp().status.get() else {
            return;
        };
        let game = self.get_current_game();
        let lang = get_option("vo-lang")
            .and_then(String::try_from)
            .unwrap_or_else(|_| "en".into());
        self.imp().busy.set(true);
        self.imp().operation_button.set_sensitive(false);
        self.imp().operation_button.set_label(
            if matches!(
                status,
                UpdateAvailability::Updated | UpdateAvailability::Untracked
            ) {
                "Running…"
            } else {
                "Working…"
            },
        );
        let (sender, receiver) = async_channel::bounded(32);
        let (done, completion) = async_channel::bounded(1);
        runtime::spawn(async move {
            let result = match status {
                UpdateAvailability::NotInstalled | UpdateAvailability::Incomplete => {
                    game_downloader::download_game(game, &lang, sender).await
                }
                UpdateAvailability::Outdated | UpdateAvailability::Preinstall => {
                    game_downloader::download_update(game, &lang, sender).await
                }
                UpdateAvailability::Updated | UpdateAvailability::Untracked => {
                    game_downloader::launch_game(game).await
                }
            };
            // Dedicated single terminal slot: ordinary progress cannot crowd out
            // the operation result, including failures before irmin starts.
            let _ = done.send(result).await;
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(progress) = receiver.recv().await {
                let Some(window) = weak.upgrade() else {
                    break;
                };
                let text = Progressbar::handle_download_event(DownloadEvent::Sophon(progress));
                window.imp().operation_button.set_tooltip_text(Some(&text));
            }
        });
        let weak = self.downgrade();
        let hold = self.application().map(|app| app.hold());
        glib::MainContext::default().spawn_local(async move {
            let _hold = hold;
            let result = completion.recv().await;
            let Some(window) = weak.upgrade() else {
                return;
            };
            window.imp().busy.set(false);
            if let Ok(Err(error)) = result {
                log::error!("Operation failed: {error:#}");
                gtk::AlertDialog::builder()
                    .message("Operation failed")
                    .detail(format!("{error:#}"))
                    .build()
                    .show(Some(&window));
            }
            window.refresh_status();
        });
    }

    /// Runs the shared runtime update through the supervised backend.
    pub async fn download_components(&self) -> Result<()> {
        proton_manager::update_all_modules().await
    }
    /// Installs the game with the requested voice language and progress channel.
    pub async fn download_game(
        &self,
        game: Game,
        lang: &str,
        sender: Sender<SophonProgress>,
    ) -> Result<()> {
        game_downloader::download_game(game, lang, sender).await
    }
    /// Applies the game's available update or downloads its preinstall data.
    pub async fn update_game(
        &self,
        game: Game,
        lang: &str,
        sender: Sender<SophonProgress>,
    ) -> Result<()> {
        game_downloader::download_update(game, lang, sender).await
    }
    /// Launches the game and waits for the supervised session to finish.
    pub async fn launch_game(&self, game: Game) -> Result<()> {
        game_downloader::launch_game(game).await
    }
}

/// Chooses the primary button label for the current installation state.
fn action_label(status: UpdateAvailability) -> &'static str {
    match status {
        UpdateAvailability::Updated | UpdateAvailability::Untracked => "Launch",
        UpdateAvailability::Outdated => "Update",
        UpdateAvailability::Preinstall => "Preinstall",
        UpdateAvailability::NotInstalled => "Download",
        UpdateAvailability::Incomplete => "Resume installation",
    }
}

/// Creates the main launcher window for an application instance.
pub fn build_window(app: &gtk::Application) -> ElysiaeWindow {
    ElysiaeWindow::new(app)
}
