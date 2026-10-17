//! Isolated collection chrome/gesture regression and optional review snapshots.
use super::*;

fn layout() {
    let loop_ = adw::glib::MainLoop::new(None, false);
    let quit = loop_.clone();
    adw::glib::timeout_add_local_once(Duration::from_millis(180), move || quit.quit());
    loop_.run();
}

fn snapshot(window: &adw::ApplicationWindow, name: &str) {
    let Some(directory) = std::env::var_os("BROOKLET_REVIEW_DIR") else {
        return;
    };
    layout();
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, window.width().into(), window.height().into());
    let node = snapshot.to_node().unwrap();
    window
        .renderer()
        .unwrap()
        .render_texture(&node, None)
        .save_to_png(directory.join(format!("{name}.png")))
        .unwrap();
}

pub fn run() -> Result<(), adw::glib::BoolError> {
    adw::init()?;
    register_resources();
    let style = gtk::CssProvider::new();
    style.load_from_resource("/com/nedrichards/brooklet/style.css");
    let display = gtk::gdk::Display::default().unwrap();
    gtk::style_context_add_provider_for_display(
        &display,
        &style,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let _desktop = ui::test_support::Desktop::install();
    let manager = adw::StyleManager::default();
    let original_scheme = manager.color_scheme();
    manager.set_color_scheme(adw::ColorScheme::ForceLight);
    let builder = gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/window.ui");
    let window: adw::ApplicationWindow = builder.object("window").unwrap();
    // Enable the fixture's controls without connecting to accounts or services.
    for (prefix, names) in [
        ("app", vec!["sync", "search"]),
        (
            "win",
            vec![
                "library-all",
                "library-unread",
                "library-read",
                "library-categories",
            ],
        ),
    ] {
        let actions = gio::SimpleActionGroup::new();
        for name in names {
            actions.add_action(&gio::SimpleAction::new(name, None));
        }
        window.insert_action_group(prefix, Some(&actions));
    }
    ui::sidebar::install(&builder);
    let menu: gtk::MenuButton = builder.object("app_menu").unwrap();
    menu.set_menu_model(Some(&ui::sidebar::main_menu()));
    let entries: Vec<_> = (0..80)
        .map(|id| Entry {
            id,
            account_id: 1,
            feed_id: 1,
            feed_title: "Example feed".into(),
            category_title: "News".into(),
            title: format!("Article {} — a place to pause and read", id + 1),
            url: "https://example.invalid".into(),
            author: Some("Example author".into()),
            published_at_ms: 0,
            html: String::new(),
            content_revision: 0,
            read: id % 3 == 0,
            starred: true,
            reading_minutes: 3,
            delivery_state: None,
            delivery_error: None,
        })
        .collect();
    for (list, scroller, status) in [
        ("inbox_list", "inbox_scroller", "inbox_status"),
        ("saved_list", "saved_scroller", "saved_status"),
        (
            "library_all_list",
            "library_all_scroller",
            "library_all_status",
        ),
    ] {
        let list: gtk::ListView = builder.object(list).unwrap();
        let model = ui::inbox::configure(&list, Rc::new(Cell::new(None)));
        ui::inbox::replace(&model, entries.clone());
        builder
            .object::<gtk::ScrolledWindow>(scroller)
            .unwrap()
            .set_visible(true);
        builder
            .object::<adw::StatusPage>(status)
            .unwrap()
            .set_visible(false);
    }
    let destinations: adw::ViewStack = builder.object("destinations").unwrap();
    let navigation: adw::NavigationView = builder.object("library_navigation").unwrap();
    let switcher: adw::ViewSwitcherBar = builder.object("destination_switcher").unwrap();
    install_destination_navigation(&builder);
    let controllers = switcher.observe_controllers();
    let gesture = (0..controllers.n_items())
        .filter_map(|i| controllers.item(i).and_downcast::<gtk::GestureClick>())
        .find(|g| g.propagation_phase() == gtk::PropagationPhase::Capture)
        .unwrap();
    window.present();
    layout();
    smoke_test_destination_top_shortcut(&builder)?;
    let reader: gtk::ScrolledWindow = builder.object("reader_scroller").unwrap();
    let content: gtk::Box = builder.object("reader_content").unwrap();
    let title = gtk::Label::new(Some("A quiet place for your reading"));
    title.add_css_class("title-1");
    title.set_xalign(0.0);
    content.append(&title);
    for _ in 0..24 {
        let paragraph = gtk::Label::new(Some(
            "An example article, shown here to check the relationship between the collection and reader. Your reading position stays independent of the list.",
        ));
        paragraph.set_wrap(true);
        paragraph.set_xalign(0.0);
        content.append(&paragraph);
    }
    builder
        .object::<adw::StatusPage>("reader_placeholder")
        .unwrap()
        .set_visible(false);
    reader.set_visible(true);
    // Exercise the production resolver, including a nested Library page.
    let mut targets = Vec::new();
    fn buttons(widget: &gtk::Widget, targets: &mut Vec<gtk::ToggleButton>) {
        if let Some(button) = widget.downcast_ref::<gtk::ToggleButton>() {
            targets.push(button.clone());
            return;
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            buttons(&widget, targets);
            child = widget.next_sibling();
        }
    }
    buttons(switcher.upcast_ref(), &mut targets);
    for (name, scroller, tag, index) in [
        ("inbox", "inbox_scroller", None, 0),
        ("saved", "saved_scroller", None, 1),
        ("library", "library_all_scroller", Some("library-all"), 2),
    ] {
        destinations.set_visible_child_name(name);
        if let Some(tag) = tag {
            navigation.push_by_tag(tag);
        }
        layout();
        reader.vadjustment().set_value(300.0);
        let scroller: gtk::ScrolledWindow = builder.object(scroller).unwrap();
        let adjustment = scroller.vadjustment();
        adjustment.set_value(400.0);
        if adjustment.value() < 100.0 {
            return Err(adw::glib::bool_error!("Sidebar fixture did not scroll"));
        }
        let bounds = targets[index].compute_bounds(&switcher).unwrap();
        let x = f64::from(bounds.x() + bounds.width() / 2.0);
        let y = f64::from(bounds.y() + bounds.height() / 2.0);
        gesture.emit_by_name::<()>("pressed", &[&1_i32, &x, &y]);
        gesture.emit_by_name::<()>("pressed", &[&2_i32, &x, &y]);
        if adjustment.value() != adjustment.lower()
            || reader.vadjustment().value() != 300.0
            || tag
                .is_some_and(|tag| navigation.visible_page().unwrap().tag().as_deref() != Some(tag))
        {
            return Err(adw::glib::bool_error!(
                "Destination gesture lost list or reader context: {name}, list={}, reader={}",
                adjustment.value(),
                reader.vadjustment().value()
            ));
        }
        snapshot(&window, name);
    }
    navigation.pop_to_tag("library-root");
    layout();
    snapshot(&window, "library-home");
    manager.set_color_scheme(adw::ColorScheme::ForceDark);
    layout();
    snapshot(&window, "library-dark");
    manager.set_color_scheme(adw::ColorScheme::ForceLight);
    window.set_default_size(360, 720);
    layout();
    snapshot(&window, "library-narrow");
    window.destroy();
    manager.set_color_scheme(original_scheme);
    gtk::style_context_remove_provider_for_display(&display, &style);
    println!("Sidebar gestures preserved active-page depth and reader position");
    Ok(())
}
