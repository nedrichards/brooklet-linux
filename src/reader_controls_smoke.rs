//! Synthetic reader layout and interaction checks; no installed account is accessed.
use super::*;

pub(super) fn run(
    window: &adw::ApplicationWindow,
    builder: &gtk::Builder,
    inbox: &InboxUi,
    reader: &ReaderUi,
    repository: &SqliteRepository,
    template: &Entry,
) -> Result<(), adw::glib::BoolError> {
    use brooklet::services::traits::Repository;
    fn settle() {
        let deadline = std::time::Instant::now() + Duration::from_millis(350);
        while std::time::Instant::now() < deadline {
            while adw::glib::MainContext::default().pending() {
                adw::glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn capture(
        window: &adw::ApplicationWindow,
        widget: &impl IsA<gtk::Widget>,
        name: &str,
    ) -> Result<(), adw::glib::BoolError> {
        let Some(directory) = std::env::var_os("BROOKLET_READER_REVIEW_DIR") else {
            return Ok(());
        };
        std::fs::create_dir_all(&directory).map_err(|error| adw::glib::bool_error!("{error}"))?;
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(widget)).snapshot(
            &snapshot,
            widget.as_ref().width() as f64,
            widget.as_ref().height() as f64,
        );
        let node = snapshot
            .to_node()
            .ok_or_else(|| adw::glib::bool_error!("Reader screenshot has no render node"))?;
        window
            .renderer()
            .unwrap()
            .render_texture(&node, None)
            .save_to_png(std::path::Path::new(&directory).join(format!("{name}.png")))
            .map_err(|error| adw::glib::bool_error!("{error}"))
    }
    fn check(value: bool, message: &str) -> Result<(), adw::glib::BoolError> {
        if value {
            Ok(())
        } else {
            Err(adw::glib::bool_error!("{message}"))
        }
    }
    install_reader_controls(window, reader);
    update_shortcut_tooltips(window.upcast_ref());
    check(
        !reader.menu.is_sensitive()
            && reader
                .actions
                .list_actions()
                .iter()
                .all(|name| !reader.actions.is_action_enabled(name)),
        "Empty reader actions remained enabled",
    )?;
    let browser: gtk::Button = builder.object("open_browser_button").unwrap();
    let keep: gtk::Button = builder.object("keep_unread_button").unwrap();
    let previous: gtk::Button = builder.object("previous_article_button").unwrap();
    let next: gtk::Button = builder.object("next_article_button").unwrap();
    for (button, action) in [
        (&browser, "reader.open-browser"),
        (&keep, "reader.keep-unread"),
        (&previous, "reader.previous-article"),
        (&next, "reader.next-article"),
    ] {
        check(
            button.tooltip_text().is_some_and(|text| {
                text.ends_with(&format!("({})", keyboard::action_hint(action)))
            }),
            "Reader hover shortcut missing or inconsistent",
        )?;
    }
    let mut entry = Entry { read: true, title: "A long article headline that must stay complete in the body while the header gives the actions room".into(), ..template.clone() };
    let other = Entry {
        id: 9876,
        ..template.clone()
    };
    let current = Rc::new(RefCell::new(Some(entry.clone())));
    let context = ReadContext {
        controller: reader.controller.clone(),
        reader: reader.clone(),
        inbox: inbox.clone(),
        current: current.clone(),
        toast: builder.object("toast_overlay").unwrap(),
        undo: Rc::new(RefCell::new(Vec::new())),
    };
    install_keep_unread_action(window, builder, context.clone());
    let destinations: adw::ViewStack = builder.object("destinations").unwrap();
    let page: adw::NavigationPage = builder.object("reader_page").unwrap();
    let keys = install_article_cursor_keys(window, &destinations, &page, &reader.scroller, context);
    show_entries(inbox, vec![other.clone()]);
    *reader.source_list.borrow_mut() = Some(inbox.list.downgrade());
    set_reader_origin_entries(
        reader,
        vec![Arc::new(entry.clone()), Arc::new(other.clone())],
        entry.id,
        true,
    );
    reader.active_id.set(Some(entry.id));
    ui::reader::begin(&entry, &reader.title, &reader.content);
    reader.placeholder.set_visible(false);
    reader.scroller.set_visible(true);
    reader.split.set_show_content(true);
    update_reader_controls(reader, Some(&entry));
    check(
        !reader.actions.is_action_enabled("previous-article")
            && reader.actions.is_action_enabled("next-article"),
        "First article navigation boundaries incorrect",
    )?;
    reader.origin_index.set(1);
    update_reader_controls(reader, Some(&other));
    check(
        reader.actions.is_action_enabled("previous-article")
            && !reader.actions.is_action_enabled("next-article"),
        "Last article navigation boundaries incorrect",
    )?;
    reader.origin_index.set(0);
    update_reader_controls(reader, Some(&entry));
    let layout: adw::BreakpointBin = builder.object("reader_layout").unwrap();
    let navigation: gtk::Box = builder.object("reader_navigation").unwrap();
    for width in [360, 550, 799, 801, 811, 1080, 1440] {
        window.set_default_size(width, 720);
        reader.split.set_show_content(true);
        settle();
        check(
            browser.is_mapped()
                && keep.is_mapped()
                && reader.menu.is_mapped()
                && browser.is_sensitive()
                && keep.is_sensitive(),
            "Primary reader actions disappeared or became disabled on resize",
        )?;
        check(
            navigation.is_visible() == layout.current_breakpoint().is_none(),
            "Navigation ignored the reader-pane breakpoint",
        )?;
        for button in [&browser, &keep] {
            let bounds = button
                .compute_bounds(&layout)
                .ok_or_else(|| adw::glib::bool_error!("Reader button has no bounds"))?;
            check(
                bounds.x() >= 0.0 && bounds.x() + bounds.width() <= layout.width() as f32 + 1.0,
                "Primary reader action clipped on resize",
            )?;
        }
        check(
            layout
                .child()
                .unwrap()
                .measure(gtk::Orientation::Horizontal, -1)
                .0
                <= layout.width(),
            "Reader toolbar minimum width exceeds its pane",
        )?;
        check(
            layout.width() <= window.width(),
            "Reader forced the window wider",
        )?;
        capture(window, window, &format!("reader-{width}"))?;
        println!(
            "Reader layout: window {}sp, pane {}sp, navigation {}",
            window.width(),
            layout.width(),
            navigation.is_visible()
        );
    }
    let settings = gtk::Settings::default().unwrap();
    let original_font = settings.gtk_font_name();
    settings.set_gtk_font_name(Some("Sans 16"));
    window.set_default_size(360, 720);
    reader.split.set_show_content(true);
    settle();
    for button in [&browser, &keep] {
        let bounds = button.compute_bounds(&layout).unwrap();
        check(
            bounds.x() >= 0.0 && bounds.x() + bounds.width() <= layout.width() as f32 + 1.0,
            "Reader primary action clipped with increased text size",
        )?;
    }
    check(
        layout
            .child()
            .unwrap()
            .measure(gtk::Orientation::Horizontal, -1)
            .0
            <= layout.width(),
        "Reader toolbar minimum width exceeds its pane with larger text",
    )?;
    capture(window, window, "reader-large-text")?;
    settings.set_gtk_font_name(original_font.as_deref());
    settle();
    // The menu's displayed accelerators use the same definitions as the hover hints.
    let menu = reader_menu_model(false);
    check(menu.n_items() == 3, "Reader menu lost its action groups")?;
    let primary = menu.item_link(0, "section").unwrap();
    check(
        primary
            .item_attribute_value(0, "action", None)
            .and_then(|value| value.get::<String>())
            .as_deref()
            == Some("reader.open-browser"),
        "Reader menu no longer prioritises browser",
    )?;
    check(
        primary
            .item_attribute_value(1, "accel", None)
            .and_then(|value| value.get::<String>())
            .as_deref()
            == Some("r"),
        "Unread menu shortcut diverged",
    )?;
    for (starred, label) in [(false, "Save"), (true, "Unsave")] {
        entry.starred = starred;
        update_reader_controls(reader, Some(&entry));
        let section = reader
            .menu
            .menu_model()
            .unwrap()
            .item_link(1, "section")
            .unwrap();
        check(
            section
                .item_attribute_value(0, "label", None)
                .and_then(|value| value.get::<String>())
                .as_deref()
                == Some(label),
            "Reader save label failed to follow article state",
        )?;
    }
    // Header actions must target the open article even while a different row is selected.
    let copied = Rc::new(Cell::new(None));
    let copy = gio::SimpleAction::new("copy-link", None);
    copy.connect_activate({
        let window = window.downgrade();
        let page = page.clone();
        let destinations = destinations.clone();
        let scroller = reader.scroller.clone();
        let current = current.clone();
        let copied = copied.clone();
        move |_, _| {
            if let Some(window) = window.upgrade() {
                copied.set(
                    article_action_target(&window, &page, &scroller, &destinations, &current)
                        .map(|entry| entry.id),
                );
            }
        }
    });
    window.add_action(&copy);
    window.set_default_size(1080, 720);
    reader.split.set_show_content(true);
    settle();
    inbox.list.grab_focus();
    reader.actions.activate_action("copy-link", None);
    check(
        copied.get() == Some(entry.id),
        "Reader action targeted a different selected article",
    )?;
    for collapsed in [false, true] {
        window.set_default_size(if collapsed { 550 } else { 1080 }, 720);
        settle();
        for mode in ["button", "menu", "key", "already-unread", "pending-read"] {
            entry.read = mode != "already-unread";
            reader
                .controller
                .backend_handle()
                .block_on(repository.merge_changed_page(1, std::slice::from_ref(&entry), &[]))
                .map_err(|error| adw::glib::bool_error!("{error}"))?;
            *current.borrow_mut() = Some(entry.clone());
            show_entries(
                inbox,
                if entry.read {
                    vec![other.clone()]
                } else {
                    vec![entry.clone(), other.clone()]
                },
            );
            if mode == "pending-read" {
                inbox.read_in_flight.borrow_mut().insert(entry.id);
            }
            reader.split.set_show_content(true);
            settle();
            reader.scroller.grab_focus();
            if mode == "button" {
                keep.emit_clicked();
            } else if mode == "menu" {
                reader.menu.popup();
                settle();
                let popover = reader.menu.popover().unwrap();
                check(popover.is_visible(), "Reader menu did not open")?;
                capture(
                    window,
                    &popover,
                    if collapsed {
                        "reader-menu-narrow"
                    } else {
                        "reader-menu-wide"
                    },
                )?;
                reader.actions.activate_action("keep-unread", None);
                reader.menu.popdown();
            } else if mode == "key" && std::env::var_os("BROOKLET_KEYBOARD_DRIVER").is_some() {
                let status = std::process::Command::new("python3")
                    .arg(std::env::var_os("BROOKLET_KEYBOARD_DRIVER").unwrap())
                    .arg(window.title().unwrap())
                    .args(["r", "both"])
                    .status()
                    .map_err(|error| adw::glib::bool_error!("{error}"))?;
                check(status.success(), "Physical reader R failed")?;
            } else {
                check(
                    keys.emit_by_name::<bool>(
                        "key-pressed",
                        &[&gtk::gdk::Key::r, &0_u32, &gtk::gdk::ModifierType::empty()],
                    ),
                    "Reader R was not handled",
                )?;
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                settle();
                let focused = gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| {
                    focus == inbox.list.clone().upcast::<gtk::Widget>()
                        || focus.is_ancestor(&inbox.list)
                });
                if focused && !current.borrow().as_ref().unwrap().read {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    return Err(adw::glib::bool_error!(
                        "Unread return failed: {mode}, collapsed={collapsed}"
                    ));
                }
            }
            check(
                !collapsed || !reader.split.shows_content(),
                "Unread return left the narrow reader open",
            )?;
            check(
                ui::inbox::selected_id(&inbox.list) == Some(entry.id),
                "Unread return lost source selection",
            )?;
            inbox.read_in_flight.borrow_mut().remove(&entry.id);
            inbox.suppress_read.borrow_mut().remove(&entry.id);
        }
    }
    window.remove_controller(&keys);
    window.remove_action("copy-link");
    window.remove_action("keep-unread");
    reader.active_id.set(None);
    update_reader_controls(reader, None);
    window.set_default_size(1080, 720);
    reader.split.set_show_content(false);
    settle();
    println!("Reader toolbar, menu, shortcut and unread-return regression passed");
    Ok(())
}
