use gtk::gio::{Settings, prelude::SettingsExt};

#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Str(String),
}

impl From<bool> for SettingValue {
    fn from(value: bool) -> Self {
        SettingValue::Bool(value)
    }
}

impl From<String> for SettingValue {
    fn from(value: String) -> Self {
        SettingValue::Str(value)
    }
}

impl From<&str> for SettingValue {
    fn from(value: &str) -> Self {
        SettingValue::Str(value.to_string())
    }
}

impl TryFrom<SettingValue> for bool {
    type Error = &'static str;
    fn try_from(value: SettingValue) -> Result<Self, Self::Error> {
        match value {
            SettingValue::Bool(b) => Ok(b),
            _ => Err("Expected boolean"),
        }
    }
}

impl TryFrom<SettingValue> for String {
    type Error = &'static str;
    fn try_from(value: SettingValue) -> Result<Self, Self::Error> {
        match value {
            SettingValue::Str(s) => Ok(s),
            _ => Err("Expected String"),
        }
    }
}

pub fn get_option(key: &str) -> SettingValue {
    let settings = Settings::new("app.elysiae.Elysiae");
    let setting_value = settings.value(key);
    if let Some(b) = setting_value.get::<bool>() {
        SettingValue::Bool(b)
    } else if let Some(s) = setting_value.get::<String>() {
        SettingValue::Str(s)
    } else {
        panic!("Unsupported setting type for key {key}")
    }
}

pub fn set_option(key: &str, value: SettingValue) {
    let settings = Settings::new("app.elysiae.Elysiae");
    match value {
        SettingValue::Bool(b) => settings.set_boolean(key, b).unwrap(),
        SettingValue::Str(s) => settings.set_string(key, &s).unwrap(),
    }
}
