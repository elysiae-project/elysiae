use gtk::glib;

use crate::core::game::Game;

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/background.ui")]
    pub struct Background;

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
        
    }
}

impl Default for Background {
    fn default() -> Self {
        Self::new()
    }
}
