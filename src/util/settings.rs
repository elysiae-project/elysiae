use std::cell::RefCell;

use anyhow::{Context, Result, ensure};
use gtk::gio::{self, Settings, SettingsSchemaSource, prelude::SettingsExt};

const APP_ID: &str = "app.elysiae.Elysiae";
const SETTINGS_VERSION: u32 = 1;
pub const SUPPORTED_LOCALES: &[&str] = &[
    "en-us", "zh-cn", "ko-kr", "fr-fr", "de-de", "es-es", "ru-ru", "id-id", "ja-jp", "th-th",
    "zh-tw", "vi-vn", "pt-br",
];

/// Boolean or string value supported by the launcher's preferences.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Str(String),
}

impl From<bool> for SettingValue {
    /// Wraps a boolean as a setting value.
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<String> for SettingValue {
    /// Wraps an owned string as a setting value.
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}

impl From<&str> for SettingValue {
    /// Copies a borrowed string into a setting value.
    fn from(v: &str) -> Self {
        Self::Str(v.into())
    }
}

impl TryFrom<SettingValue> for bool {
    type Error = anyhow::Error;

    /// Reads a boolean setting, rejecting string values.
    fn try_from(v: SettingValue) -> Result<Self> {
        match v {
            SettingValue::Bool(v) => Ok(v),
            _ => anyhow::bail!("Expected boolean"),
        }
    }
}

impl TryFrom<SettingValue> for String {
    type Error = anyhow::Error;

    /// Reads a string setting, rejecting boolean values.
    fn try_from(v: SettingValue) -> Result<Self> {
        match v {
            SettingValue::Str(v) => Ok(v),
            _ => anyhow::bail!("Expected string"),
        }
    }
}

thread_local! {
    // GObjects stay on the calling thread. Production objects share the normal
    // GSettings backend; each test thread instead owns a private memory backend.
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

/// Returns the calling thread's cached settings object using the app schema.
/// Tests use a private in-memory backend instead of desktop preferences.
fn settings() -> Result<Settings> {
    SETTINGS.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(settings) = slot.as_ref() { return Ok(settings.clone()); }

        let parent = SettingsSchemaSource::default();
        let schema = SettingsSchemaSource::from_directory(env!("OUT_DIR"), parent.as_ref(), false)
            .ok().and_then(|source| source.lookup(APP_ID, false))
            .or_else(|| parent.and_then(|source| source.lookup(APP_ID, true)))
            .context("Elysiae's GSettings schema is not installed; install it or set GSETTINGS_SCHEMA_DIR")?;

        #[cfg(test)]
        let backend = Some(gio::memory_settings_backend_new());
        #[cfg(not(test))]
        let backend: Option<gio::SettingsBackend> = None;
        let settings = Settings::new_full(&schema, backend.as_ref(), None);
        *slot = Some(settings.clone());

        Ok(settings)
    })
}

/// Rejects unknown preference keys before calling GSettings accessors.
fn require_key(settings: &Settings, key: &str) -> Result<()> {
    ensure!(
        settings
            .settings_schema()
            .is_some_and(|schema| schema.has_key(key)),
        "Unknown setting: {key}"
    );

    Ok(())
}

/// Register one application-lifetime listener on the calling thread's settings.
/// The application supplies weak GTK references, so the listener owns no windows.
pub fn connect_changed(callback: impl Fn(&str) + 'static) -> Result<()> {
    let settings = settings()?;
    let _ = settings.connect_changed(None, move |_, key| callback(key));

    Ok(())
}

/// Reads a known preference as a boolean or string value.
pub fn get_option(key: &str) -> Result<SettingValue> {
    let settings = settings()?;
    require_key(&settings, key)?;

    let value = settings.value(key);
    if let Some(value) = value.get::<bool>() {
        Ok(value.into())
    } else if let Some(value) = value.get::<String>() {
        Ok(value.into())
    } else {
        anyhow::bail!("Unsupported setting type for key {key}")
    }
}

/// Validates and saves a preference, normalizing language identifiers as needed.
pub fn set_option(key: &str, value: SettingValue) -> Result<()> {
    let settings = settings()?;
    require_key(&settings, key)?;

    let current = settings.value(key);
    match value {
        SettingValue::Bool(value) => {
            ensure!(
                current.get::<bool>().is_some(),
                "Setting {key} expects a string"
            );
            settings.set_boolean(key, value)?;
        }
        SettingValue::Str(value) => {
            ensure!(
                current.get::<String>().is_some(),
                "Setting {key} expects a boolean"
            );

            let value = match key {
                "vo-lang" => voice_language(&value)?.to_owned(),
                "display-language" => normalize_locale(&value)?,
                _ => value,
            };
            settings.set_string(key, &value)?;
        }
    }

    if settings.is_delay_apply() {
        settings.apply();
    }

    Ok(())
}

/// The installer uses short identifiers, independently of launcher locale.
pub fn voice_language(value: &str) -> Result<&'static str> {
    match value.replace('_', "-").to_ascii_lowercase().as_str() {
        "en" | "en-us" => Ok("en"),
        "cn" | "zh-cn" | "zh-tw" => Ok("cn"),
        "jp" | "ja" | "ja-jp" => Ok("jp"),
        "kr" | "ko" | "ko-kr" => Ok("kr"),
        _ => anyhow::bail!("Unsupported voice-over language: {value}"),
    }
}

/// Converts a supported launcher locale to its lowercase, hyphenated form.
pub fn normalize_locale(value: &str) -> Result<String> {
    let locale = value.replace('_', "-").to_ascii_lowercase();
    ensure!(
        SUPPORTED_LOCALES.contains(&locale.as_str()),
        "Unsupported launcher locale: {value}"
    );

    Ok(locale)
}

/// Returns the configured display language as a validated launcher locale.
pub fn display_locale() -> Result<String> {
    normalize_locale(&String::try_from(get_option("display-language")?)?)
}

/// Startup preference state captured before migrations are applied.
#[derive(Debug, PartialEq)]
pub struct Initialization {
    pub first_launch: bool,
    pub previous_version: String,
}

/// Commit migrations together, after parsing every value. An unsupported future
/// version or malformed preference leaves the stored choices untouched.
pub fn initialize() -> Result<Initialization> {
    let settings = settings()?;
    for key in [
        "settings-version",
        "is-first-launch",
        "last-used-version",
        "vo-lang",
        "display-language",
    ] {
        require_key(&settings, key)?;
    }

    let version: u32 = String::try_from(get_option("settings-version")?)?
        .parse()
        .context("Invalid settings migration version")?;
    ensure!(
        version <= SETTINGS_VERSION,
        "Settings were written by a newer Elysiae version"
    );

    let initial = Initialization {
        first_launch: bool::try_from(get_option("is-first-launch")?)?,
        previous_version: String::try_from(get_option("last-used-version")?)?,
    };
    let voice = voice_language(&String::try_from(get_option("vo-lang")?)?)?;
    let locale = display_locale()?;

    settings.delay();
    let result = (|| -> Result<()> {
        settings.set_string("vo-lang", voice)?;
        settings.set_string("display-language", &locale)?;
        settings.set_string("settings-version", &SETTINGS_VERSION.to_string())?;
        settings.set_string("last-used-version", env!("CARGO_PKG_VERSION"))?;
        settings.set_boolean("is-first-launch", false)?;

        Ok(())
    })();

    if let Err(error) = result {
        settings.revert();
        return Err(error);
    }

    settings.apply();

    Ok(initial)
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;
