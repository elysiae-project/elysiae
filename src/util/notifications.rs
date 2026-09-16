use gtk::gio::{Notification, prelude::ApplicationExt};

use crate::util::settings::get_option;

pub fn broadcast_notification(body: &str) {
    if get_option("allow-notifications").try_into().unwrap()
        && let Some(app) = gtk::gio::Application::default()
    {
        let n = Notification::new("Elysiae");
        n.set_body(Some(body));
        n.set_priority(gtk::gio::NotificationPriority::Normal);
        app.send_notification(None, &n);
    }
}
