use gtk::glib;

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/titlebar.ui")]
    pub struct Titlebar;

    #[glib::object_subclass]
    impl ObjectSubclass for Titlebar {
        const NAME: &'static str = "ElysiaeTitlebar";
        type Type = super::Titlebar;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Titlebar {}
    impl WidgetImpl for Titlebar {}
    impl BoxImpl for Titlebar {}

    #[gtk::template_callbacks]
    impl Titlebar {
        #[template_callback]
        fn on_close_clicked(&self, _button: &gtk::Button) {
            if let Some(window) = self.obj().root().and_then(|r| r.downcast::<gtk::Window>().ok())
            {
                window.close();
            }
        }
    }
}

glib::wrapper! {
    pub struct Titlebar(ObjectSubclass<imp::Titlebar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Titlebar {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }
}

impl Default for Titlebar {
    fn default() -> Self {
        Self::new()
    }
}
