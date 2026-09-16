use crate::core::game::Game;
use anyhow::Result;
use gtk::glib::{self};

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::glib::clone;
    use gtk::glib::types::StaticType;
    use gtk::prelude::ButtonExt;
    use gtk::subclass::prelude::*;

    use crate::widgets;
    use crate::widgets::background;
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
                move |_| println!("Operation button clicked")
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
        glib::Object::builder().property("application", app).build()
    }

    pub fn change_game(&self, game: Game) {}

    pub async fn download_components(&self) -> Result<()> {
        todo!()
    }

    pub async fn download_game(&self, game: Game) -> Result<()> {
        todo!()
    }
}

pub fn build_window(app: &gtk::Application) -> ElysiaeWindow {
    ElysiaeWindow::new(app)
}
