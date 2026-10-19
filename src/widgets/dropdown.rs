use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
};

use gtk::{glib, prelude::*, subclass::prelude::*};

mod option_imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/dropdown-option.ui")]
    pub struct DropdownOption {
        #[template_child]
        pub label: TemplateChild<gtk::Label>,
        #[template_child]
        pub check: TemplateChild<gtk::Image>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DropdownOption {
        const NAME: &'static str = "ElysiaeDropdownOption";
        type Type = super::DropdownOption;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }
    impl ObjectImpl for DropdownOption {
        fn dispose(&self) {
            self.dispose_template();
        }
    }
    impl WidgetImpl for DropdownOption {}
    impl ListBoxRowImpl for DropdownOption {}
}

glib::wrapper! {
    pub struct DropdownOption(ObjectSubclass<option_imp::DropdownOption>)
        @extends gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Actionable;
}

impl DropdownOption {
    fn new(text: &str) -> Self {
        let row: Self = glib::Object::new();
        row.imp().label.set_label(text);
        row.set_tooltip_text(Some(text));
        row
    }

    fn mark_selected(&self, selected: bool) {
        self.imp()
            .check
            .set_opacity(if selected { 1.0 } else { 0.0 });
        if selected {
            self.add_css_class("is-selected");
        } else {
            self.remove_css_class("is-selected");
        }
    }
}

mod imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/dropdown.ui")]
    pub struct Dropdown {
        #[template_child]
        pub trigger: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub label: TemplateChild<gtk::Label>,
        #[template_child]
        pub popover: TemplateChild<gtk::Popover>,
        #[template_child]
        pub list: TemplateChild<gtk::ListBox>,
        pub selected: Cell<u32>,
        pub restore_focus: Cell<bool>,
        pub options: RefCell<Vec<super::DropdownOption>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Dropdown {
        const NAME: &'static str = "ElysiaeDropdown";
        type Type = super::Dropdown;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Dropdown {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecUInt::builder("selected")
                        .default_value(gtk::INVALID_LIST_POSITION)
                        .read_only()
                        .build(),
                ]
            })
        }

        fn property(&self, _: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "selected" => self.selected.get().to_value(),
                _ => unreachable!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            self.selected.set(gtk::INVALID_LIST_POSITION);
            self.obj().connect_controls();
        }

        fn dispose(&self) {
            self.options.borrow_mut().clear();
            self.dispose_template();
        }
    }

    impl WidgetImpl for Dropdown {
        fn grab_focus(&self) -> bool {
            self.trigger.child_focus(gtk::DirectionType::TabForward)
        }
    }
    impl BoxImpl for Dropdown {}
}

glib::wrapper! {
    /// XML-backed single-choice control, independent of GtkDropDown.
    pub struct Dropdown(ObjectSubclass<imp::Dropdown>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Dropdown {
    pub fn new(labels: &[&str]) -> Self {
        let obj: Self = glib::Object::new();
        for label in labels {
            let row = DropdownOption::new(label);
            obj.imp().list.append(&row);
            obj.imp().options.borrow_mut().push(row);
        }
        obj.set_sensitive(!labels.is_empty());
        obj.imp().trigger.set_sensitive(!labels.is_empty());
        obj.set_selected(0);
        obj
    }

    fn connect_controls(&self) {
        let weak = self.downgrade();
        self.imp().list.connect_row_activated(move |_, row| {
            if let Some(obj) = weak.upgrade() {
                obj.set_selected(row.index() as u32);
                obj.dismiss(true);
            }
        });
        let weak = self.downgrade();
        self.imp().popover.connect_visible_notify(move |popover| {
            if popover.is_visible()
                && let Some(obj) = weak.upgrade()
                && let Some(row) = obj.imp().list.row_at_index(obj.selected() as i32)
            {
                obj.imp().restore_focus.set(true);
                obj.imp().list.select_row(Some(&row));
                row.grab_focus();
            }
        });
        let weak = self.downgrade();
        self.imp().popover.connect_closed(move |_| {
            if let Some(obj) = weak.upgrade()
                && obj.imp().restore_focus.get()
                && obj.is_mapped()
                && obj.is_sensitive()
            {
                obj.grab_focus();
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |obj| obj.handle_key(key))
        });
        self.add_controller(keys);
    }

    pub fn selected(&self) -> u32 {
        self.imp().selected.get()
    }

    /// Invalid indices leave the last valid selection untouched.
    pub fn set_selected(&self, index: u32) {
        let Some(row) = self.imp().options.borrow().get(index as usize).cloned() else {
            return;
        };
        if self.imp().selected.replace(index) == index {
            return;
        }
        let text = row.imp().label.label();
        self.imp().label.set_label(&text);
        self.imp().trigger.set_tooltip_text(Some(&text));
        for option in self.imp().options.borrow().iter() {
            option.mark_selected(option.index() as u32 == index);
        }
        self.imp().list.select_row(Some(&row));
        self.notify("selected");
    }

    pub fn connect_selected_notify(&self, f: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_notify_local(Some("selected"), move |obj, _| f(obj))
    }

    /// Label the actual focus target as well as the containing control.
    pub fn set_accessible_labels(&self, label: &gtk::Label, description: &gtk::Label) {
        if let Some(button) = self.imp().trigger.first_child() {
            button.update_relation(&[
                gtk::accessible::Relation::LabelledBy(&[
                    label.upcast_ref(),
                    self.imp().label.upcast_ref(),
                ]),
                gtk::accessible::Relation::DescribedBy(&[description.upcast_ref()]),
                gtk::accessible::Relation::Controls(&[self.imp().list.upcast_ref()]),
            ]);
        }
    }

    pub fn popup(&self) {
        if self.is_sensitive() && !self.imp().options.borrow().is_empty() {
            self.imp().trigger.popup();
        }
    }

    pub fn is_open(&self) -> bool {
        self.imp().popover.is_visible()
    }

    pub fn dismiss(&self, restore_focus: bool) -> bool {
        let open = self.is_open();
        if open {
            // Tab and modal dismissal already choose the next focus target.
            self.imp().restore_focus.set(restore_focus);
            self.imp().trigger.popdown();
        }
        open
    }

    fn handle_key(&self, key: gtk::gdk::Key) -> glib::Propagation {
        use gtk::gdk::Key;
        if !self.is_open() {
            if matches!(key, Key::Down | Key::Up) && !self.imp().options.borrow().is_empty() {
                self.popup();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        match key {
            Key::Escape => {
                self.dismiss(true);
            }
            Key::Return | Key::KP_Enter | Key::space => {
                if let Some(row) = self.imp().list.selected_row() {
                    self.imp().list.emit_by_name::<()>("row-activated", &[&row]);
                }
            }
            Key::Down | Key::Up | Key::Home | Key::End => {
                let last = self.imp().options.borrow().len() as i32 - 1;
                let current = self.imp().list.selected_row().map_or(0, |row| row.index());
                let index = match key {
                    Key::Down => (current + 1).min(last),
                    Key::Up => (current - 1).max(0),
                    Key::Home => 0,
                    _ => last,
                };
                if let Some(row) = self.imp().list.row_at_index(index) {
                    self.imp().list.select_row(Some(&row));
                    row.grab_focus();
                }
            }
            Key::Tab | Key::ISO_Left_Tab => {
                self.dismiss(false);
                return glib::Propagation::Proceed;
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        rc::Rc,
        time::{Duration, Instant},
    };

    fn pump_until(ready: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !ready() {
            assert!(Instant::now() < deadline, "GTK dropdown did not settle");
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn key(dropdown: &Dropdown, key: gtk::gdk::Key) -> bool {
        let controllers = dropdown.observe_controllers();
        let controller = (0..controllers.n_items())
            .find_map(|i| {
                controllers
                    .item(i)
                    .and_downcast::<gtk::EventControllerKey>()
            })
            .unwrap();
        controller.emit_by_name(
            "key-pressed",
            &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
        )
    }

    #[test]
    #[ignore = "requires a GTK display; run explicitly on one test thread"]
    fn gtk_dropdown_template_selects_navigates_and_dismisses() {
        gtk::init().unwrap();
        gtk::gio::resources_register_include!("elysiae.gresource").unwrap();
        let labels = [
            "English",
            "日本語",
            "한국어",
            "Chinese (Traditional)",
            "Français",
            "Deutsch",
            "Español",
            "Русский",
            "Bahasa Indonesia",
            "ไทย",
            "Tiếng Việt",
            "Portuguese (Brazil)",
        ];
        let dropdown = Dropdown::new(&labels);
        let empty = Dropdown::new(&[]);
        assert_eq!(empty.selected(), gtk::INVALID_LIST_POSITION);
        assert!(!empty.is_sensitive());
        empty.popup();
        assert!(!empty.is_open());
        let changes = Rc::new(Cell::new(0));
        let count = changes.clone();
        dropdown.connect_selected_notify(move |_| count.set(count.get() + 1));
        assert_eq!(dropdown.selected(), 0);
        dropdown.set_selected(0);
        dropdown.set_selected(gtk::INVALID_LIST_POSITION);
        assert_eq!(changes.get(), 0);
        let window = gtk::Window::builder()
            .default_width(400)
            .default_height(420)
            .child(&dropdown)
            .build();
        dropdown.set_valign(gtk::Align::Start);
        window.present();
        pump_until(|| window.is_mapped() && dropdown.width() > 0);
        dropdown.popup();
        pump_until(|| dropdown.imp().popover.is_mapped());
        let row = dropdown.imp().list.row_at_index(2).unwrap();
        dropdown
            .imp()
            .list
            .emit_by_name::<()>("row-activated", &[&row]);
        pump_until(|| !dropdown.is_open());
        assert_eq!(dropdown.selected(), 2);
        assert_eq!(dropdown.imp().label.label(), labels[2]);
        assert_eq!(changes.get(), 1);
        assert!(row.has_css_class("is-selected"));
        assert!(key(&dropdown, gtk::gdk::Key::Down));
        pump_until(|| dropdown.imp().popover.is_mapped());
        assert!(key(&dropdown, gtk::gdk::Key::End));
        assert_eq!(
            dropdown.selected(),
            2,
            "Navigation must not commit a choice"
        );
        assert_eq!(
            dropdown.imp().list.selected_row().unwrap().index(),
            labels.len() as i32 - 1
        );
        assert!(key(&dropdown, gtk::gdk::Key::Escape));
        pump_until(|| !dropdown.is_open());
        assert_eq!(dropdown.selected(), 2);
        assert_eq!(changes.get(), 1);
        pump_until(|| {
            gtk::prelude::GtkWindowExt::focus(&window).is_some_and(|w| w.is_ancestor(&dropdown))
        });
        dropdown.popup();
        pump_until(|| dropdown.imp().popover.is_mapped());
        assert!(key(&dropdown, gtk::gdk::Key::Home));
        assert!(key(&dropdown, gtk::gdk::Key::Down));
        assert!(key(&dropdown, gtk::gdk::Key::Return));
        pump_until(|| !dropdown.is_open());
        assert_eq!(dropdown.selected(), 1);
        assert_eq!(changes.get(), 2);
        dropdown.popup();
        pump_until(|| dropdown.imp().popover.is_mapped());
        dropdown.set_selected((labels.len() - 1) as u32);
        assert_eq!(
            dropdown.imp().list.selected_row().unwrap().index(),
            labels.len() as i32 - 1
        );
        assert!(!key(&dropdown, gtk::gdk::Key::Tab));
        pump_until(|| !dropdown.is_open());
        dropdown.set_sensitive(false);
        dropdown.popup();
        assert!(!dropdown.is_open());
        dropdown.set_sensitive(true);
        dropdown.popup();
        pump_until(|| dropdown.imp().popover.is_mapped());
        dropdown.imp().popover.popdown();
        pump_until(|| !dropdown.is_open());
        window.close();
        let weak = dropdown.downgrade();
        drop(dropdown);
        drop(row);
        drop(window);
        pump_until(|| weak.upgrade().is_none());
    }
}
