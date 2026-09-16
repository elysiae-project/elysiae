use gtk::glib;

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/progressbar.ui")]
    pub struct Progressbar;

    #[glib::object_subclass]
    impl ObjectSubclass for Progressbar {
        const NAME: &'static str = "ElysiaeProgressbar";
        type Type = super::Progressbar;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Progressbar {}
    impl WidgetImpl for Progressbar {}
    impl BoxImpl for Progressbar {}
}

glib::wrapper! {
    pub struct Progressbar(ObjectSubclass<imp::Progressbar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Progressbar {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }
}

impl Default for Progressbar {
    fn default() -> Self {
        Self::new()
    }
}
