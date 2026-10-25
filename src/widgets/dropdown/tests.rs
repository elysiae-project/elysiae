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
