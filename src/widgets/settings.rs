use std::cell::RefCell;

use anyhow::{Context, Result, bail};
use gtk::{glib, prelude::*, subclass::prelude::ObjectSubclassIsExt};

use crate::{
    util::settings::{SUPPORTED_LOCALES, SettingValue, connect_changed, get_option, set_option},
    widgets::dropdown::Dropdown,
};

const VOICE_LANGUAGES: &[&str] = &["en", "cn", "jp", "kr"];

struct FieldSpec {
    key: &'static str,
    label: &'static str,
    description: &'static str,
    choices: Option<&'static [&'static str]>,
}

// Migration bookkeeping deliberately has no editable control.
const FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "vo-lang",
        label: "Voice-over language",
        description: "Audio language for new game downloads.",
        choices: Some(VOICE_LANGUAGES),
    },
    FieldSpec {
        key: "generate-desktop-shortcut",
        label: "Desktop shortcuts",
        description: "Create a shortcut when a game is installed.",
        choices: None,
    },
    FieldSpec {
        key: "allow-notifications",
        label: "Desktop notifications",
        description: "Notify you when downloads and updates finish.",
        choices: None,
    },
    FieldSpec {
        key: "use-video-background",
        label: "Video backgrounds",
        description: "Play animated backgrounds when available.",
        choices: None,
    },
    FieldSpec {
        key: "check-for-updates",
        label: "Check for updates",
        description: "Check game and runtime releases without installing them.",
        choices: None,
    },
    FieldSpec {
        key: "display-language",
        label: "Display language",
        description: "Choose localized artwork. Launcher text stays in English.",
        choices: Some(SUPPORTED_LOCALES),
    },
];

/// Human-readable choices retain the schema's identifiers as their values.
fn language_label(language: &str) -> &str {
    match language {
        "en" | "en-us" => "English",
        "cn" => "Chinese",
        "zh-cn" => "Chinese (Simplified)",
        "zh-tw" => "Chinese (Traditional)",
        "jp" | "ja-jp" => "Japanese",
        "kr" | "ko-kr" => "Korean",
        "fr-fr" => "French",
        "de-de" => "German",
        "es-es" => "Spanish",
        "ru-ru" => "Russian",
        "id-id" => "Indonesian",
        "th-th" => "Thai",
        "vi-vn" => "Vietnamese",
        "pt-br" => "Portuguese (Brazil)",
        _ => language,
    }
}

enum Control {
    Switch(gtk::Switch),
    Dropdown(Dropdown, &'static [&'static str]),
}

impl Control {
    fn widget(&self) -> gtk::Widget {
        match self {
            Self::Switch(control) => control.clone().upcast(),
            Self::Dropdown(control, _) => control.clone().upcast(),
        }
    }

    fn value(&self) -> Result<SettingValue> {
        match self {
            Self::Switch(control) => Ok(control.is_active().into()),
            Self::Dropdown(control, choices) => choices
                .get(control.selected() as usize)
                .map(|value| SettingValue::from(*value))
                .context("Choose a supported language"),
        }
    }

    fn set_value(&self, value: &SettingValue) -> Result<()> {
        match (self, value) {
            (Self::Switch(control), SettingValue::Bool(value)) => control.set_active(*value),
            (Self::Dropdown(control, choices), SettingValue::Str(value)) => {
                let index = choices
                    .iter()
                    .position(|choice| *choice == value)
                    .context("The saved language is not supported")?;
                control.set_selected(index as u32);
            }
            _ => bail!("The saved preference has an unexpected type"),
        }
        Ok(())
    }
}

struct Field {
    spec: &'static FieldSpec,
    control: Control,
    previous: RefCell<Option<SettingValue>>,
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::collections::BTreeMap;

    use gtk::{CompositeTemplate, glib, subclass::prelude::*};

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/settings.ui")]
    pub struct Settings {
        #[template_child]
        pub close: TemplateChild<gtk::Button>,
        #[template_child]
        pub content: TemplateChild<gtk::Box>,
        #[template_child]
        pub error: TemplateChild<gtk::Label>,
        pub(super) fields: OnceCell<Vec<super::Field>>,
        pub(super) refreshing: Cell<bool>,
        pub(super) errors: RefCell<BTreeMap<&'static str, String>>,
    }

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

    impl ObjectImpl for Settings {
        fn constructed(&self) {
            self.parent_constructed();
            // Runs for both Settings::new() and instances made by a parent template.
            self.obj().initialize();
        }
    }
    impl WidgetImpl for Settings {}
    impl BoxImpl for Settings {}
}

glib::wrapper! {
    /// Settings content; the parent owns modal dismissal and focus containment.
    pub struct Settings(ObjectSubclass<imp::Settings>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Settings {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Connect this button to the parent modal's dismissal handler.
    pub fn close_button(&self) -> gtk::Button {
        self.imp().close.get()
    }

    /// Call after showing the modal; focus the first available preference.
    pub fn focus_first(&self) {
        if let Some(fields) = self.imp().fields.get() {
            for field in fields {
                if field.control.widget().grab_focus() {
                    return;
                }
            }
        }
        self.imp().close.grab_focus();
    }

    /// Close nested choices before dismissing or moving focus around the modal.
    pub fn dismiss_dropdowns(&self, restore_focus: bool) -> bool {
        let mut dismissed = false;
        for field in self.imp().fields.get().into_iter().flatten() {
            if let Control::Dropdown(control, _) = &field.control {
                dismissed |= control.dismiss(restore_focus);
            }
        }
        dismissed
    }

    #[cfg(test)]
    pub(crate) fn dropdown(&self, key: &str) -> Dropdown {
        let Control::Dropdown(control, _) = &self.field(key).unwrap().control else {
            panic!("Expected a dropdown preference");
        };
        control.clone()
    }

    #[cfg(test)]
    pub fn get_property_value(&self, key: &str) -> Result<String> {
        match get_option(key)? {
            SettingValue::Bool(value) => Ok(value.to_string()),
            SettingValue::Str(value) => Ok(value),
        }
    }

    #[cfg(test)]
    pub fn set_property_value(&self, key: &str, value: &str) -> Result<()> {
        let result = (|| {
            let setting = match get_option(key)? {
                SettingValue::Bool(_) => SettingValue::Bool(value.parse()?),
                SettingValue::Str(_) => SettingValue::Str(value.to_owned()),
            };
            set_option(key, setting)
        })();
        self.finish_change(key, &result);
        result
    }

    fn initialize(&self) {
        let fields = FIELDS.iter().map(|spec| self.add_field(spec)).collect();
        // Construction happens once, before any user or settings callbacks exist.
        let _ = self.imp().fields.set(fields);
        for field in self.imp().fields.get().into_iter().flatten() {
            self.refresh_field(field.spec.key);
            let weak = self.downgrade();
            let key = field.spec.key;
            match &field.control {
                Control::Switch(control) => {
                    control.connect_active_notify(move |_| {
                        if let Some(settings) = weak.upgrade() {
                            settings.field_changed(key);
                        }
                    });
                }
                Control::Dropdown(control, _) => {
                    control.connect_selected_notify(move |_| {
                        if let Some(settings) = weak.upgrade() {
                            settings.field_changed(key);
                        }
                    });
                }
            }
        }
        let weak = self.downgrade();
        if let Err(error) = connect_changed(move |key| {
            if let Some(settings) = weak.upgrade() {
                settings.refresh_field(key);
            }
        }) {
            self.show_error("", format!("Could not monitor preference changes: {error}"));
        }
    }

    fn add_field(&self, spec: &'static FieldSpec) -> Field {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(20)
            .css_classes(["settings-row"])
            .build();
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .hexpand(true)
            .css_classes(["settings-row-text"])
            .build();
        let label = gtk::Label::builder()
            .label(spec.label)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["settings-label"])
            .build();
        let description = gtk::Label::builder()
            .label(spec.description)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(36)
            .css_classes(["settings-description"])
            .build();
        text.append(&label);
        text.append(&description);
        row.append(&text);

        let control = match spec.choices {
            Some(choices) => {
                let labels: Vec<_> = choices
                    .iter()
                    .map(|choice| language_label(choice))
                    .collect();
                let dropdown = Dropdown::new(&labels);
                dropdown.set_valign(gtk::Align::Center);
                dropdown.set_accessible_labels(&label, &description);
                Control::Dropdown(dropdown, choices)
            }
            None => Control::Switch(
                gtk::Switch::builder()
                    .valign(gtk::Align::Center)
                    .css_classes(["settings-switch"])
                    .build(),
            ),
        };
        let widget = control.widget();
        widget.update_relation(&[
            gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()]),
            gtk::accessible::Relation::DescribedBy(&[description.upcast_ref()]),
        ]);
        label.set_mnemonic_widget(Some(&widget));
        row.append(&widget);
        self.imp().content.append(&row);
        Field {
            spec,
            control,
            previous: RefCell::new(None),
        }
    }

    fn field(&self, key: &str) -> Option<&Field> {
        self.imp()
            .fields
            .get()?
            .iter()
            .find(|field| field.spec.key == key)
    }

    /// Backend refreshes never write, even when GTK emits synchronous notifications.
    fn refresh_field(&self, key: &str) {
        let Some(field) = self.field(key) else { return };
        let was_refreshing = self.imp().refreshing.replace(true);
        let result = get_option(key).and_then(|value| {
            field.control.set_value(&value)?;
            *field.previous.borrow_mut() = Some(value);
            Ok(())
        });
        self.imp().refreshing.set(was_refreshing);
        field.control.widget().set_sensitive(result.is_ok());
        match result {
            Ok(()) => {
                self.imp().errors.borrow_mut().remove(field.spec.key);
                self.update_error();
            }
            Err(error) => self.show_error(
                field.spec.key,
                format!(
                    "Could not load {}: {error}. Reopen the launcher to retry.",
                    field.spec.label
                ),
            ),
        }
    }

    fn field_changed(&self, key: &'static str) {
        if self.imp().refreshing.get() {
            return;
        }
        let Some(field) = self.field(key) else { return };
        let result = match field.control.value() {
            // GTK may queue a second notify after rollback. It is not an edit
            // and must neither write again nor erase the failure message.
            Ok(value) if field.previous.borrow().as_ref() == Some(&value) => return,
            Ok(value) => set_option(key, value),
            Err(error) => Err(error),
        };
        self.finish_change(key, &result);
    }

    fn finish_change(&self, key: &str, result: &Result<()>) {
        let Some(field) = self.field(key) else { return };
        match result {
            Ok(()) => self.refresh_field(key),
            Err(error) => {
                // Prefer the current backend value; retain a known-good fallback if
                // the backend itself is unavailable. Restoring must not write again.
                let previous = field.previous.borrow().clone();
                let value = get_option(key).ok().or(previous.clone());
                let was_refreshing = self.imp().refreshing.replace(true);
                if let Some(value) = value
                    && field.control.set_value(&value).is_err()
                    && let Some(previous) = previous
                {
                    let _ = field.control.set_value(&previous);
                }
                self.imp().refreshing.set(was_refreshing);
                self.show_error(
                    field.spec.key,
                    format!(
                        "Could not save {}: {error}. Your previous choice was restored. Try again.",
                        field.spec.label
                    ),
                );
            }
        }
    }

    fn show_error(&self, key: &'static str, message: String) {
        self.imp().errors.borrow_mut().insert(key, message);
        self.update_error();
    }

    fn update_error(&self) {
        let message = self
            .imp()
            .errors
            .borrow()
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        self.imp().error.set_label(&message);
        self.imp().error.set_visible(!message.is_empty());
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
                !["is-first-launch", "settings-version", "last-used-version"]
                    .contains(&key.as_str())
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
}
