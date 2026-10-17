//! Shared controls for collection, navigation and search headers.
use adw::{gio, prelude::*};

// Mark dynamic labels for gettext extraction, as in the shortcut definitions.
const fn message(text: &str) -> &str {
    text
}

pub fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    for (label, action) in [
        (message("Mark Inbox Read"), "win.mark-all-read"),
        (message("Search Articles"), "app.search"),
        (message("Refresh Feeds"), "win.refresh-feeds"),
        (message("Subscribe…"), "win.subscribe"),
        (message("Preferences"), "win.preferences"),
        (message("Keyboard Shortcuts"), "app.show-shortcuts"),
        (message("About Brooklet"), "win.about"),
    ] {
        menu.append(
            Some(&adw::glib::dgettext(Some("brooklet"), label)),
            Some(action),
        );
    }
    menu
}

pub fn populate(header: &adw::HeaderBar) {
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for (icon, action, label) in [
        ("view-refresh-symbolic", "app.sync", message("Sync")),
        (
            "system-search-symbolic",
            "app.search",
            message("Search Articles"),
        ),
    ] {
        let label = adw::glib::dgettext(Some("brooklet"), label);
        let button = gtk::Button::from_icon_name(icon);
        button.set_action_name(Some(action));
        button.set_tooltip_text(Some(&format!(
            "{label} ({})",
            crate::keyboard::action_hint(action)
        )));
        button.update_property(&[gtk::accessible::Property::Label(&label)]);
        controls.append(&button);
    }
    let menu_label = adw::glib::dgettext(Some("brooklet"), "Main Menu");
    let menu = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&main_menu())
        .tooltip_text(menu_label.as_str())
        .build();
    menu.set_widget_name("app_menu");
    menu.update_property(&[gtk::accessible::Property::Label(&menu_label)]);
    controls.append(&menu);
    header.pack_end(&controls);
}

pub fn header() -> adw::HeaderBar {
    let header = adw::HeaderBar::new();
    populate(&header);
    header
}

pub fn install(builder: &gtk::Builder) {
    for name in [
        "saved_header",
        "library_header",
        "library_all_header",
        "library_unread_header",
        "library_read_header",
        "library_categories_header",
    ] {
        populate(&builder.object::<adw::HeaderBar>(name).expect(name));
    }
}
