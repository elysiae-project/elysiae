use anyhow::Result;
use gtk::gio::{Settings, prelude::SettingsExt};
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Str(String),
}
impl From<bool> for SettingValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<String> for SettingValue {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}
impl From<&str> for SettingValue {
    fn from(v: &str) -> Self {
        Self::Str(v.into())
    }
}
impl TryFrom<SettingValue> for bool {
    type Error = anyhow::Error;
    fn try_from(v: SettingValue) -> Result<Self> {
        match v {
            SettingValue::Bool(v) => Ok(v),
            _ => anyhow::bail!("Expected boolean"),
        }
    }
}
impl TryFrom<SettingValue> for String {
    type Error = anyhow::Error;
    fn try_from(v: SettingValue) -> Result<Self> {
        match v {
            SettingValue::Str(v) => Ok(v),
            _ => anyhow::bail!("Expected string"),
        }
    }
}
pub fn get_option(key: &str) -> Result<SettingValue> {
    let s = Settings::new("app.elysiae.Elysiae");
    let v = s.value(key);
    if let Some(b) = v.get::<bool>() {
        Ok(b.into())
    } else if let Some(v) = v.get::<String>() {
        Ok(v.into())
    } else {
        Err(anyhow::anyhow!("Unsupported setting type for key {key}"))
    }
}
pub fn set_option(key: &str, value: SettingValue) -> Result<()> {
    let s = Settings::new("app.elysiae.Elysiae");
    match value {
        SettingValue::Bool(v) => {
            s.set_boolean(key, v)?;
        }
        SettingValue::Str(v) => {
            s.set_string(key, &v)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_value_conversions_are_type_safe() {
        assert_eq!(bool::try_from(SettingValue::from(true)).unwrap(), true);
        assert_eq!(
            String::try_from(SettingValue::from("en-us")).unwrap(),
            "en-us"
        );
        assert!(bool::try_from(SettingValue::from("not a bool")).is_err());
        assert!(String::try_from(SettingValue::from(true)).is_err());
    }
}
