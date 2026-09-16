use gtk::glib;

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/settings.ui")]
    pub struct Settings;

    #[glib::object_subclass]
    impl ObjectSubclass for Settings {
        const NAME: &'static str = "ElysiaeSettings";
        type Type = super::Settings;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Settings {}
    impl WidgetImpl for Settings {}
    impl BoxImpl for Settings {}
}

glib::wrapper! {
    pub struct Settings(ObjectSubclass<imp::Settings>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Settings {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}
