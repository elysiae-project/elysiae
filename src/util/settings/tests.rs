use super::*;

/// Checks invalid preferences return errors instead of reaching GLib assertions.
#[test]
fn invalid_keys_and_types_return_errors_without_glib_abort() {
    assert!(get_option("not-a-key").is_err());
    assert!(set_option("not-a-key", true.into()).is_err());
    assert!(set_option("vo-lang", true.into()).is_err());
    assert!(set_option("allow-notifications", "yes".into()).is_err());
    assert!(set_option("vo-lang", "invalid".into()).is_err());
}

/// Checks idempotent migration, independent language choices and future versions.
#[test]
fn initialization_preserves_preferences_and_rejects_newer_schemas() {
    let settings = settings().unwrap();
    settings.set_string("vo-lang", "en-us").unwrap();
    set_option("display-language", "fr_FR".into()).unwrap();
    set_option("use-video-background", true.into()).unwrap();

    assert!(initialize().unwrap().first_launch);
    assert!(!initialize().unwrap().first_launch);
    assert_eq!(
        String::try_from(get_option("vo-lang").unwrap()).unwrap(),
        "en"
    );
    assert_eq!(display_locale().unwrap(), "fr-fr");
    assert!(bool::try_from(get_option("use-video-background").unwrap()).unwrap());
    assert_eq!(
        settings.string("last-used-version"),
        env!("CARGO_PKG_VERSION")
    );

    settings.set_string("settings-version", "99").unwrap();
    assert!(initialize().is_err());
    assert_eq!(settings.string("settings-version"), "99");
    assert_eq!(display_locale().unwrap(), "fr-fr");
}
