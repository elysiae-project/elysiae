use gtk::glib;

use crate::core::game::Game;

mod imp {
    use gtk::CompositeTemplate;
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/sidebar.ui")]
    pub struct Sidebar;

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "ElysiaeSidebar";
        type Type = super::Sidebar;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Sidebar {}
    impl WidgetImpl for Sidebar {}
    impl BoxImpl for Sidebar {}
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn process_event<F>(game: Game, on_game_changed: F)
    where
        F: FnOnce(Game),
    {
        on_game_changed(game);
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}
