use super::*;

#[test]
fn editable_fields_cover_schema_without_migration_metadata() {
    let source = gtk::gio::SettingsSchemaSource::from_directory(
        env!("OUT_DIR"),
        None::<&gtk::gio::SettingsSchemaSource>,
        false,
    )
    .unwrap();
    let schema = source.lookup("app.elysiae.Elysiae", false).unwrap();
    let mut expected: Vec<_> = schema
        .list_keys()
        .into_iter()
        .map(|key| key.to_string())
        .filter(|key| {
            !["is-first-launch", "settings-version", "last-used-version"].contains(&key.as_str())
        })
        .collect();
    expected.sort();
    let mut actual: Vec<_> = FIELDS.iter().map(|spec| spec.key.to_owned()).collect();
    actual.sort();

    assert_eq!(actual, expected);
    assert_eq!(FIELDS.len(), 6);
    assert_eq!(FIELDS.last().unwrap().choices, Some(SUPPORTED_LOCALES));
    assert_eq!(VOICE_LANGUAGES, &["en", "cn", "jp", "kr"]);
}

#[test]
fn every_editable_preference_round_trips_in_memory() {
    for spec in FIELDS {
        let original = get_option(spec.key).unwrap();
        let values: Vec<SettingValue> = match spec.choices {
            Some(choices) => choices.iter().map(|choice| (*choice).into()).collect(),
            None => vec![false.into(), true.into()],
        };
        for value in values {
            set_option(spec.key, value.clone()).unwrap();
            assert_eq!(get_option(spec.key).unwrap(), value);
        }

        set_option(spec.key, original).unwrap();
    }
}

/// Run explicitly with a display and --ignored --test-threads=1.
#[test]
#[ignore = "requires a GTK display; run explicitly on one test thread"]
fn gtk_settings_template_persists_syncs_and_restores() {
    gtk::init().expect("This test requires a working GTK display");
    gtk::gio::resources_register_include!("elysiae.gresource").unwrap();
    Settings::static_type();

    let builder = gtk::Builder::from_string(
        "<interface><object class='ElysiaeSettings' id='settings'/></interface>",
    );
    let settings: Settings = builder.object("settings").unwrap();
    assert_eq!(settings.imp().fields.get().unwrap().len(), 6);
    assert!(settings.close_button().has_css_class("settings-close"));

    for field in settings.imp().fields.get().unwrap() {
        assert_eq!(
            field.control.value().unwrap(),
            get_option(field.spec.key).unwrap()
        );
        let value = match field.spec.choices {
            Some(choices) => (*choices.last().unwrap()).into(),
            None => (!bool::try_from(get_option(field.spec.key).unwrap()).unwrap()).into(),
        };
        field.control.set_value(&value).unwrap();
        assert_eq!(get_option(field.spec.key).unwrap(), value);

        let external: SettingValue = match field.spec.choices {
            Some(choices) => choices[0].into(),
            None => (!bool::try_from(value).unwrap()).into(),
        };
        set_option(field.spec.key, external.clone()).unwrap();
        assert_eq!(field.control.value().unwrap(), external);
    }

    let dropdown = settings.dropdown("vo-lang");
    let row = dropdown.imp().list.row_at_index(2).unwrap();
    dropdown
        .imp()
        .list
        .emit_by_name::<()>("row-activated", &[&row]);
    assert_eq!(get_option("vo-lang").unwrap(), "jp".into());
    drop(row);
    drop(dropdown);

    set_option("vo-lang", "cn".into()).unwrap();
    assert_eq!(
        settings.field("vo-lang").unwrap().control.value().unwrap(),
        "cn".into()
    );

    settings
        .set_property_value("display-language", "fr_FR")
        .unwrap();
    assert_eq!(
        settings.get_property_value("display-language").unwrap(),
        "fr-fr"
    );

    assert!(settings.set_property_value("vo-lang", "invalid").is_err());
    assert!(settings.imp().error.is_visible());
    assert_eq!(
        settings.field("vo-lang").unwrap().control.value().unwrap(),
        "cn".into()
    );

    settings.set_property_value("vo-lang", "jp").unwrap();
    assert!(!settings.imp().error.is_visible());

    // Simulate an unsaved control value before a rejected write.
    let Control::Dropdown(dropdown, _) = &settings.field("vo-lang").unwrap().control else {
        panic!("Voice language must use a dropdown");
    };
    settings.imp().refreshing.set(true);
    dropdown.set_selected(0);
    settings.imp().refreshing.set(false);
    assert!(settings.set_property_value("vo-lang", "invalid").is_err());
    assert!(settings.imp().error.is_visible());
    assert_eq!(dropdown.selected(), 2);
    assert_eq!(settings.get_property_value("vo-lang").unwrap(), "jp");

    let weak = settings.downgrade();
    drop(settings);
    drop(builder);
    assert!(
        weak.upgrade().is_none(),
        "Preference listeners must not retain the panel"
    );
    set_option("vo-lang", "en".into()).unwrap();
}
