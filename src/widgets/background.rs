use gtk::glib;
use gtk::subclass::prelude::ObjectSubclassIsExt;

use crate::{
    core::game::Game,
    util::{cache::media_paths, settings::{SettingValue, get_option}},
};

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/background.ui")]
    pub struct Background {
        #[template_child]
        pub background: TemplateChild<gtk::Picture>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Background {
        const NAME: &'static str = "ElysiaeBackground";
        type Type = super::Background;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Background {}
    impl WidgetImpl for Background {}
    impl BoxImpl for Background {}
}

glib::wrapper! {
    pub struct Background(ObjectSubclass<imp::Background>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Background {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_media(&self, game: Game) {
        let use_video = matches!(get_option("use-video-background"), Ok(SettingValue::Bool(true)));
        if let Ok((path, _overlay)) = media_paths(game, "en-us", use_video) {
            self.imp().background.set_filename(path.as_deref());
        } else {
            self.imp().background.set_filename(None::<&std::path::Path>);
        }
    }
}

impl Default for Background {
    fn default() -> Self {
        Self::new()
    }
}
