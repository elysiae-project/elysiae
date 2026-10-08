use crate::core::{
    game::Game,
    game_downloader::{download_game as install_game, download_update, launch_game as run_game},
    proton_manager::{components_installed, update_all_modules},
};
use anyhow::Result;
use async_channel::Sender;
use irmin::SophonProgress;
use gtk::glib;
use gtk::subclass::prelude::ObjectSubclassIsExt;

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::glib::clone;
    use gtk::glib::types::StaticType;
    use gtk::prelude::ButtonExt;
    use crate::core::game::Game;
    use crate::util::{runtime, settings::{SettingValue, get_option}};
    use crate::widgets::progressbar::{DownloadEvent, Progressbar};
    use crate::core::{game_downloader::download_game as install_game, proton_manager::{components_installed, update_all_modules}};
    use gtk::subclass::prelude::*;

    use crate::widgets;
    use crate::widgets::{background::Background, titlebar::Titlebar};

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/window.ui")]
    pub struct ElysiaeWindow {
        #[template_child]
        pub settings_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub operation_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub background: TemplateChild<widgets::background::Background>,

        pub current_game: std::cell::Cell<Game>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ElysiaeWindow {
        const NAME: &'static str = "ElysiaeWindow";
        type Type = super::ElysiaeWindow;
        type ParentType = gtk::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            Titlebar::static_type();
            Background::static_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }

    }

    impl ObjectImpl for ElysiaeWindow {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();

            self.settings_button.connect_clicked(clone!(
                #[weak]
                obj,
                move |_| println!("Settings button clicked")
            ));

            self.operation_button.connect_clicked(clone!(
                #[weak]
                obj,
                move |_| {
                    let game = obj.get_current_game();
                    let lang = match get_option("vo-lang") {
                        Ok(SettingValue::Str(value)) => value,
                        _ => "en-us".to_owned(),
                    };
                    let (sender, receiver) = async_channel::unbounded();
                    glib::MainContext::default().spawn_local(async move {
                        while let Ok(progress) = receiver.recv().await {
                            let _ = Progressbar::handle_download_event(DownloadEvent::Sophon(progress));
                        }
                    });
                    runtime::spawn(async move {
                        if !components_installed().unwrap_or(false) {
                            if let Err(error) = update_all_modules().await {
                                log::error!("Component installation failed: {error:#}");
                                return;
                            }
                        }
                        if let Err(error) = install_game(game, &lang, sender).await {
                            log::error!("Game download failed: {error:#}");
                        }
                    });
                }
            ));
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
        window.change_game(Game::Bh3);
        window
    }

    pub fn get_current_game(&self) -> Game {
        self.imp().current_game.get()
    }

    pub fn change_game(&self, game: Game) {
        self.imp().current_game.set(game);
        self.imp().background.set_media(game);
    }

    pub async fn download_components(&self) -> Result<()> {
        update_all_modules().await?;
        Ok(())
    }

    pub async fn download_game(
        &self,
        game: Game,
        lang: &str,
        sender: Sender<SophonProgress>,
    ) -> Result<()> {
        if !components_installed()? {
            self.download_components().await?;
        }

        install_game(game, lang, sender).await
    }

    pub async fn update_game(
        &self,
        game: Game,
        lang: &str,
        sender: Sender<SophonProgress>,
    ) -> Result<()> {
        if !components_installed()? {
            self.download_components().await?;
        }

        download_update(game, lang, sender).await
    }

    pub async fn launch_game(&self, game: Game) -> Result<()> {
        run_game(game).await
    }
}

pub fn build_window(app: &gtk::Application) -> ElysiaeWindow {
    ElysiaeWindow::new(app)
}
