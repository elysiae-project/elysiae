use anyhow::Result;
use gtk::glib;

use crate::util::settings::{SettingValue, get_option, set_option};

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

    pub fn get_property_value(&self, key: &str) -> Result<String> {
        match get_option(key)? {
            SettingValue::Bool(value) => Ok(value.to_string()),
            SettingValue::Str(value) => Ok(value),
        }
    }

    pub fn set_property_value(&self, key: &str, value: &str) -> Result<()> {
        let current = get_option(key)?;
        let setting = match current {
            SettingValue::Bool(_) => SettingValue::Bool(value.parse()? ),
            SettingValue::Str(_) => SettingValue::Str(value.to_owned()),
        };
        set_option(key, setting)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}
