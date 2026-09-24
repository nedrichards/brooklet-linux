use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use adw::gio;
use adw::prelude::*;

use brooklet::{
    config,
    controller::AppController,
    error::BrookletError,
    model::{Entry, ReaderPosition},
    services::secret_store::Oo7SecretStore,
    setup::{AccountSetupService, MinifluxIdentityValidator},
    storage::sqlite::SqliteRepository,
    sync::{AccountSyncService, ReqwestMinifluxApiFactory},
};

use crate::ui;

pub struct BrookletApplication {
    application: adw::Application,
    controller: Arc<AppController>,
}

#[derive(Clone)]
struct InboxUi {
    model: ui::inbox::InboxModel,
    list: gtk::ListView,
    status: adw::StatusPage,
    scroller: gtk::ScrolledWindow,
    spinner: adw::Spinner,
    new_button: gtk::Button,
    pending_entries: Rc<RefCell<Option<Vec<Entry>>>>,
    read_in_flight: Rc<RefCell<HashSet<i64>>>,
    suppress_read: Rc<RefCell<HashSet<i64>>>,
    pinned_read: Rc<RefCell<Option<Entry>>>,
    rebuilding: Rc<Cell<bool>>,
}

#[derive(Clone)]
struct ReaderUi {
    split: adw::NavigationSplitView,
    title: adw::WindowTitle,
    placeholder: adw::StatusPage,
    scroller: gtk::ScrolledWindow,
    content: gtk::Box,
    controller: Arc<AppController>,
    active_id: Rc<Cell<Option<i64>>>,
    restoring: Rc<Cell<bool>>,
    origin_set: Rc<RefCell<Vec<Entry>>>,
    origin_index: Rc<Cell<usize>>,
    origin_inbox: Rc<Cell<bool>>,
}

#[derive(Clone)]
struct OtherViews {
    saved: ui::inbox::InboxModel,
    saved_status: adw::StatusPage,
    saved_scroller: gtk::ScrolledWindow,
    library_all: ui::inbox::InboxModel,
    library_unread: ui::inbox::InboxModel,
    library_read: ui::inbox::InboxModel,
    library_all_status: adw::StatusPage,
    library_unread_status: adw::StatusPage,
    library_read_status: adw::StatusPage,
    library_all_scroller: gtk::ScrolledWindow,
    library_unread_scroller: gtk::ScrolledWindow,
    library_read_scroller: gtk::ScrolledWindow,
    categories: gtk::ListBox,
    categories_status: adw::StatusPage,
    categories_scroller: gtk::ScrolledWindow,
}

#[derive(Clone)]
struct WindowTools {
    controller: Arc<AppController>,
    reader: ReaderUi,
    current: Rc<RefCell<Option<Entry>>>,
    inbox: InboxUi,
    views: OtherViews,
    toast: adw::ToastOverlay,
    undo: Rc<RefCell<Vec<Entry>>>,
}

#[derive(Clone)]
struct ReadContext {
    controller: Arc<AppController>,
    inbox: InboxUi,
    toast: adw::ToastOverlay,
    undo: Rc<RefCell<Vec<Entry>>>,
    reader: ReaderUi,
    current: Rc<RefCell<Option<Entry>>>,
}

#[derive(Clone)]
struct SearchState {
    controller: Arc<AppController>,
    query: gtk::SearchEntry,
    feed: gtk::DropDown,
    category: gtk::DropDown,
    read: gtk::DropDown,
    feed_ids: Rc<RefCell<Vec<Option<i64>>>>,
    category_ids: Rc<RefCell<Vec<Option<i64>>>>,
    model: ui::inbox::InboxModel,
    status: adw::StatusPage,
    scroller: gtk::ScrolledWindow,
    generation: Rc<Cell<u64>>,
    toast: adw::ToastOverlay,
}

impl SearchState {
    fn refresh(&self) {
        let token = self.generation.get().wrapping_add(1);
        self.generation.set(token);
        let feed = self
            .feed_ids
            .borrow()
            .get(self.feed.selected() as usize)
            .copied()
            .flatten();
        let category = self
            .category_ids
            .borrow()
            .get(self.category.selected() as usize)
            .copied()
            .flatten();
        let read = match self.read.selected() {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        };
        let text = self.query.text().to_string();
        let model = self.model.clone();
        let generation = self.generation.clone();
        let toast = self.toast.clone();
        let status = self.status.clone();
        let scroller = self.scroller.clone();
        let controller = self.controller.clone();
        adw::glib::timeout_add_local_once(Duration::from_millis(180), move || {
            if generation.get() != token {
                return;
            }
            controller.search_entries(text, feed, category, read, move |result| {
                if generation.get() != token {
                    return;
                }
                match result {
                    Ok(entries) => {
                        let empty = entries.is_empty();
                        ui::inbox::replace(&model, entries);
                        status.set_visible(empty);
                        scroller.set_visible(!empty);
                    }
                    Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                }
            });
        });
    }
}

impl BrookletApplication {
    pub fn new() -> Result<Self, BrookletError> {
        adw::glib::set_application_name(config::APP_NAME);
        register_resources();

        let database_path = adw::glib::user_data_dir()
            .join(config::APP_ID)
            .join("brooklet.db");
        let repository = Arc::new(SqliteRepository::open(database_path)?);
        let secrets = Arc::new(Oo7SecretStore::new(config::APP_ID));
        let setup_service = Arc::new(AccountSetupService::new(
            Arc::new(MinifluxIdentityValidator),
            repository.clone(),
            secrets.clone(),
        ));
        let sync_service = Arc::new(AccountSyncService::new(
            repository,
            secrets,
            Arc::new(ReqwestMinifluxApiFactory),
        ));
        let controller = Arc::new(AppController::new(setup_service, sync_service)?);
        let application = adw::Application::builder()
            .application_id(config::APP_ID)
            .build();
        let this = Self {
            application,
            controller,
        };
        this.install_actions();
        this.connect_lifecycle();
        Ok(this)
    }

    pub fn run(self) -> adw::glib::ExitCode {
        self.application.run()
    }

    fn connect_lifecycle(&self) {
        let controller = self.controller.clone();
        self.application.connect_activate(move |application| {
            if let Some(window) = application.active_window() {
                window.present();
                return;
            }

            let style = gtk::CssProvider::new();
            style.load_from_resource("/com/nedrichards/brooklet/style.css");
            if let Some(display) = gtk::gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &style,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }

            let builder = gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/window.ui");
            let window: adw::ApplicationWindow = builder
                .object("window")
                .expect("window.ui must define the application window");
            window.set_application(Some(application));
            let inbox_status: adw::StatusPage = builder
                .object("inbox_status")
                .expect("window.ui must define inbox_status");
            let setup_button: gtk::Button = builder
                .object("setup_button")
                .expect("window.ui must define setup_button");
            let toast_overlay: adw::ToastOverlay = builder
                .object("toast_overlay")
                .expect("window.ui must define toast_overlay");
            let inbox_scroller: gtk::ScrolledWindow = builder
                .object("inbox_scroller")
                .expect("window.ui must define inbox_scroller");
            let inbox_list: gtk::ListView = builder
                .object("inbox_list")
                .expect("window.ui must define inbox_list");
            let sync_spinner: adw::Spinner = builder
                .object("sync_spinner")
                .expect("window.ui must define sync_spinner");
            let inbox_split: adw::NavigationSplitView = builder
                .object("inbox_split")
                .expect("window.ui must define inbox_split");
            let reader_title: adw::WindowTitle = builder
                .object("reader_title")
                .expect("window.ui must define reader_title");
            let reader_placeholder: adw::StatusPage = builder
                .object("reader_placeholder")
                .expect("window.ui must define reader_placeholder");
            let reader_scroller: gtk::ScrolledWindow = builder
                .object("reader_scroller")
                .expect("window.ui must define reader_scroller");
            let reader_content: gtk::Box = builder
                .object("reader_content")
                .expect("window.ui must define reader_content");
            let library_navigation: adw::NavigationView = builder
                .object("library_navigation")
                .expect("window.ui must define library_navigation");
            let saved_list: gtk::ListView = builder.object("saved_list").expect("saved_list");
            let other_views = OtherViews {
                saved: ui::inbox::configure_with_action(&saved_list, false),
                saved_status: builder.object("saved_status").expect("saved_status"),
                saved_scroller: builder.object("saved_scroller").expect("saved_scroller"),
                library_all: ui::inbox::configure_with_action(
                    &builder
                        .object::<gtk::ListView>("library_all_list")
                        .expect("library_all_list"),
                    false,
                ),
                library_unread: ui::inbox::configure_with_action(
                    &builder
                        .object::<gtk::ListView>("library_unread_list")
                        .expect("library_unread_list"),
                    false,
                ),
                library_read: ui::inbox::configure_with_action(
                    &builder
                        .object::<gtk::ListView>("library_read_list")
                        .expect("library_read_list"),
                    false,
                ),
                library_all_status: builder
                    .object("library_all_status")
                    .expect("library_all_status"),
                library_unread_status: builder
                    .object("library_unread_status")
                    .expect("library_unread_status"),
                library_read_status: builder
                    .object("library_read_status")
                    .expect("library_read_status"),
                library_all_scroller: builder
                    .object("library_all_scroller")
                    .expect("library_all_scroller"),
                library_unread_scroller: builder
                    .object("library_unread_scroller")
                    .expect("library_unread_scroller"),
                library_read_scroller: builder
                    .object("library_read_scroller")
                    .expect("library_read_scroller"),
                categories: builder
                    .object("library_categories_list")
                    .expect("library_categories_list"),
                categories_status: builder
                    .object("library_categories_status")
                    .expect("library_categories_status"),
                categories_scroller: builder
                    .object("library_categories_scroller")
                    .expect("library_categories_scroller"),
            };
            let current_entry = Rc::new(RefCell::new(None::<Entry>));
            let inbox_ui = InboxUi {
                model: ui::inbox::configure(&inbox_list),
                list: inbox_list.clone(),
                status: inbox_status.clone(),
                scroller: inbox_scroller,
                spinner: sync_spinner,
                new_button: builder
                    .object("new_articles_button")
                    .expect("new_articles_button"),
                pending_entries: Rc::new(RefCell::new(None)),
                read_in_flight: Rc::new(RefCell::new(HashSet::new())),
                suppress_read: Rc::new(RefCell::new(HashSet::new())),
                pinned_read: Rc::new(RefCell::new(None)),
                rebuilding: Rc::new(Cell::new(false)),
            };
            inbox_ui.model.selection.connect_selected_item_notify({
                let inbox = inbox_ui.clone();
                move |_| {
                    if !inbox.rebuilding.get() {
                        let inbox = inbox.clone();
                        adw::glib::idle_add_local_once(move || {
                            if !inbox.rebuilding.get() {
                                release_read_pin_unless(
                                    &inbox,
                                    ui::inbox::selected_id(&inbox.list),
                                );
                            }
                        });
                    }
                }
            });
            inbox_ui.new_button.connect_clicked({
                let inbox = inbox_ui.clone();
                move |_| {
                    let Some(entries) = inbox.pending_entries.borrow_mut().take() else {
                        return;
                    };
                    apply_inbox_entries(&inbox, entries);
                    inbox.scroller.vadjustment().set_value(0.0);
                    inbox.new_button.set_visible(false);
                }
            });
            let reader_ui = ReaderUi {
                split: inbox_split,
                title: reader_title,
                placeholder: reader_placeholder,
                scroller: reader_scroller,
                content: reader_content,
                controller: controller.clone(),
                active_id: Rc::new(Cell::new(None)),
                restoring: Rc::new(Cell::new(false)),
                origin_set: Rc::new(RefCell::new(Vec::new())),
                origin_index: Rc::new(Cell::new(0)),
                origin_inbox: Rc::new(Cell::new(false)),
            };
            reader_ui.scroller.vadjustment().connect_value_changed({
                let reader = reader_ui.clone();
                let generation = Rc::new(Cell::new(0_u64));
                move |adjustment| {
                    if reader.restoring.get() {
                        return;
                    }
                    let Some(entry_id) = reader.active_id.get() else {
                        return;
                    };
                    let token = generation.get().wrapping_add(1);
                    generation.set(token);
                    let generation = generation.clone();
                    let controller = reader.controller.clone();
                    let content = reader.content.clone();
                    let active_id = reader.active_id.clone();
                    let offset = adjustment.value() as i32;
                    adw::glib::timeout_add_local_once(Duration::from_millis(250), move || {
                        if generation.get() != token || active_id.get() != Some(entry_id) {
                            return;
                        }
                        let position = reader_position_from_offset(&content, entry_id, offset);
                        controller.save_reader_position(position, |_| {});
                    });
                }
            });
            let undo_entry = Rc::new(RefCell::new(Vec::<Entry>::new()));

            inbox_list.connect_activate({
                let model = inbox_ui.model.clone();
                let reader_ui = reader_ui.clone();
                let controller = controller.clone();
                let inbox_ui = inbox_ui.clone();
                let toast_overlay = toast_overlay.clone();
                let undo_entry = undo_entry.clone();
                let current_entry = current_entry.clone();
                move |list, position| {
                    if let Some(entry) = ui::inbox::entry_at(&model, position) {
                        ui::inbox::select_id(list, entry.id);
                        *current_entry.borrow_mut() = Some(entry.clone());
                        set_reader_origin(&reader_ui, &model, entry.id, true);
                        open_article(&reader_ui, &inbox_ui, &entry);
                        if !entry.read {
                            mark_read(
                                ReadContext {
                                    controller: controller.clone(),
                                    inbox: inbox_ui.clone(),
                                    toast: toast_overlay.clone(),
                                    undo: undo_entry.clone(),
                                    reader: reader_ui.clone(),
                                    current: current_entry.clone(),
                                },
                                entry,
                                !reader_ui.split.is_collapsed(),
                            );
                        }
                    }
                }
            });
            for (name, model) in [
                ("saved_list", other_views.saved.clone()),
                ("library_all_list", other_views.library_all.clone()),
                ("library_unread_list", other_views.library_unread.clone()),
                ("library_read_list", other_views.library_read.clone()),
            ] {
                let list: gtk::ListView = builder.object(name).expect("article list");
                list.connect_activate({
                    let controller = controller.clone();
                    let reader_ui = reader_ui.clone();
                    let current_entry = current_entry.clone();
                    let inbox_ui = inbox_ui.clone();
                    let toast_overlay = toast_overlay.clone();
                    let undo_entry = undo_entry.clone();
                    move |list, position| {
                        if let Some(entry) = ui::inbox::entry_at(&model, position) {
                            ui::inbox::select_id(list, entry.id);
                            *current_entry.borrow_mut() = Some(entry.clone());
                            set_reader_origin(&reader_ui, &model, entry.id, false);
                            open_article(&reader_ui, &inbox_ui, &entry);
                            if !entry.read {
                                mark_read(
                                    ReadContext {
                                        controller: controller.clone(),
                                        inbox: inbox_ui.clone(),
                                        toast: toast_overlay.clone(),
                                        undo: undo_entry.clone(),
                                        reader: reader_ui.clone(),
                                        current: current_entry.clone(),
                                    },
                                    entry,
                                    false,
                                );
                            }
                        }
                    }
                });
            }
            let mark_read_action =
                gio::SimpleAction::new("mark-read", Some(&i64::static_variant_type()));
            mark_read_action.connect_activate({
                let controller = controller.clone();
                let inbox_ui = inbox_ui.clone();
                let toast_overlay = toast_overlay.clone();
                let undo_entry = undo_entry.clone();
                let reader_ui = reader_ui.clone();
                let current = current_entry.clone();
                move |_, parameter| {
                    let Some(entry_id) = parameter.and_then(|value| value.get::<i64>()) else {
                        return;
                    };
                    if let Some(entry) = ui::inbox::entry_by_id(&inbox_ui.model, entry_id) {
                        let retain = !reader_ui.split.is_collapsed();
                        if retain {
                            ui::inbox::select_id(&inbox_ui.list, entry_id);
                        }
                        mark_read(
                            ReadContext {
                                controller: controller.clone(),
                                inbox: inbox_ui.clone(),
                                toast: toast_overlay.clone(),
                                undo: undo_entry.clone(),
                                reader: reader_ui.clone(),
                                current: current.clone(),
                            },
                            entry,
                            retain,
                        );
                    }
                }
            });
            window.add_action(&mark_read_action);
            let mark_unread_action =
                gio::SimpleAction::new("mark-unread", Some(&i64::static_variant_type()));
            mark_unread_action.connect_activate({
                let controller = controller.clone();
                let inbox = inbox_ui.clone();
                let reader = reader_ui.clone();
                let current = current_entry.clone();
                let undo = undo_entry.clone();
                let toast = toast_overlay.clone();
                move |_, parameter| {
                    let Some(entry_id) = parameter.and_then(|value| value.get::<i64>()) else {
                        return;
                    };
                    if let Some(entry) = ui::inbox::entry_by_id(&inbox.model, entry_id) {
                        mark_unread(
                            controller.clone(),
                            inbox.clone(),
                            reader.clone(),
                            current.clone(),
                            undo.clone(),
                            toast.clone(),
                            entry,
                        );
                    }
                }
            });
            window.add_action(&mark_unread_action);
            let mark_all_action = gio::SimpleAction::new("mark-all-read", None);
            mark_all_action.connect_activate({
                let controller = controller.clone();
                let inbox = inbox_ui.clone();
                let undo = undo_entry.clone();
                let toast = toast_overlay.clone();
                move |_, _| {
                    let entries = (0..inbox.model.store.n_items())
                        .filter_map(|position| ui::inbox::entry_at(&inbox.model, position))
                        .filter(|entry| !entry.read)
                        .collect::<Vec<_>>();
                    if entries.is_empty() {
                        return;
                    }
                    let ids = entries.iter().map(|entry| entry.id).collect::<Vec<_>>();
                    controller.set_read_many_local(ids, true, {
                        let inbox = inbox.clone();
                        let undo = undo.clone();
                        let toast = toast.clone();
                        move |result| match result {
                            Ok(()) => {
                                let count = entries.len();
                                *undo.borrow_mut() = entries;
                                inbox.pinned_read.borrow_mut().take();
                                inbox.rebuilding.set(true);
                                ui::inbox::replace(&inbox.model, Vec::new());
                                inbox.rebuilding.set(false);
                                update_inbox_visibility(&inbox);
                                let message = format!("Marked {count} articles read");
                                let notification = adw::Toast::new(&message);
                                notification.set_button_label(Some("Undo"));
                                notification.set_action_name(Some("win.undo"));
                                toast.add_toast(notification);
                            }
                            Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                        }
                    });
                }
            });
            window.add_action(&mark_all_action);
            let undo_action = gio::SimpleAction::new("undo", None);
            undo_action.connect_activate({
                let controller = controller.clone();
                let inbox_ui = inbox_ui.clone();
                let toast_overlay = toast_overlay.clone();
                let undo_entry = undo_entry.clone();
                let reader = reader_ui.clone();
                let current = current_entry.clone();
                move |_, _| {
                    undo_mark_read(
                        controller.clone(),
                        inbox_ui.clone(),
                        toast_overlay.clone(),
                        undo_entry.clone(),
                        reader.clone(),
                        current.clone(),
                    );
                }
            });
            window.add_action(&undo_action);
            let reader_menu: gtk::MenuButton = builder.object("reader_menu").expect("reader_menu");
            let menu = gio::Menu::new();
            menu.append(Some("Previous Article"), Some("win.previous-article"));
            menu.append(Some("Next Article"), Some("win.next-article"));
            for (label, action) in [
                ("Keep Unread", "win.keep-unread"),
                ("Save / Unsave", "win.toggle-star"),
                ("Send to Karakeep", "win.send-karakeep"),
                ("Open in Browser", "win.open-browser"),
                ("Copy Link", "win.copy-link"),
            ] {
                menu.append(Some(label), Some(action));
            }
            reader_menu.set_menu_model(Some(&menu));
            let tools = WindowTools {
                controller: controller.clone(),
                reader: reader_ui.clone(),
                current: current_entry.clone(),
                inbox: inbox_ui.clone(),
                views: other_views.clone(),
                toast: toast_overlay.clone(),
                undo: undo_entry.clone(),
            };
            install_reader_actions(&window, tools.clone());
            install_window_tools(&window, application, &builder, tools);
            let destinations: adw::ViewStack =
                builder.object("destinations").expect("destinations");
            install_article_cursor_keys(&window, &destinations, &reader_ui.scroller);
            for (name, tag) in [
                ("library-all", "library-all"),
                ("library-unread", "library-unread"),
                ("library-read", "library-read"),
                ("library-categories", "library-categories"),
            ] {
                let action = gio::SimpleAction::new(name, None);
                action.connect_activate({
                    let navigation = library_navigation.clone();
                    let controller = controller.clone();
                    let views = other_views.clone();
                    let toast = toast_overlay.clone();
                    move |_, _| {
                        navigation.push_by_tag(tag);
                        load_other_views(controller.clone(), views.clone(), toast.clone());
                    }
                });
                window.add_action(&action);
            }
            let category_action =
                gio::SimpleAction::new("library-category", Some(&i64::static_variant_type()));
            category_action.connect_activate({
                let controller = controller.clone();
                let navigation = library_navigation.clone();
                let toast = toast_overlay.clone();
                move |_, value| {
                    let Some(category_id) = value.and_then(|value| value.get::<i64>()) else {
                        return;
                    };
                    controller.feeds_cached(Some(category_id), {
                        let navigation = navigation.clone();
                        let toast = toast.clone();
                        move |result| match result {
                            Ok(feeds) => {
                                let empty = feeds.is_empty();
                                let list = gtk::ListBox::new();
                                list.add_css_class("boxed-list");
                                list.set_selection_mode(gtk::SelectionMode::None);
                                for feed in feeds {
                                    let row = adw::ActionRow::builder()
                                        .title(&feed.title)
                                        .use_markup(false)
                                        .activatable(true)
                                        .build();
                                    row.set_action_name(Some("win.library-feed"));
                                    row.set_action_target_value(Some(&feed.id.to_variant()));
                                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                                    list.append(&row);
                                }
                                let scroller = gtk::ScrolledWindow::new();
                                scroller.set_child(Some(&list));
                                let status = adw::StatusPage::new();
                                status.set_vexpand(true);
                                status.set_icon_name(Some("folder-symbolic"));
                                status.set_title("No feeds in this category");
                                status.set_description(Some(
                                    "Feeds added in Miniflux will appear after sync.",
                                ));
                                let toolbar = adw::ToolbarView::new();
                                toolbar.add_top_bar(&adw::HeaderBar::new());
                                if empty {
                                    toolbar.set_content(Some(&status));
                                } else {
                                    toolbar.set_content(Some(&scroller));
                                }
                                navigation.push(&adw::NavigationPage::new(&toolbar, "Feeds"));
                            }
                            Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                        }
                    });
                }
            });
            window.add_action(&category_action);
            let feed_action =
                gio::SimpleAction::new("library-feed", Some(&i64::static_variant_type()));
            feed_action.connect_activate({
                let controller = controller.clone();
                let navigation = library_navigation.clone();
                let reader = reader_ui.clone();
                let current = current_entry.clone();
                let inbox = inbox_ui.clone();
                let toast = toast_overlay.clone();
                let undo = undo_entry.clone();
                move |_, value| {
                    let Some(feed_id) = value.and_then(|value| value.get::<i64>()) else {
                        return;
                    };
                    let list = gtk::ListView::new(
                        None::<gtk::SelectionModel>,
                        None::<gtk::ListItemFactory>,
                    );
                    let model = ui::inbox::configure_with_action(&list, false);
                    let scroller = gtk::ScrolledWindow::new();
                    scroller.set_vexpand(true);
                    scroller.set_child(Some(&list));
                    scroller.set_visible(false);
                    let status = adw::StatusPage::new();
                    status.set_vexpand(true);
                    status.set_icon_name(Some("folder-documents-symbolic"));
                    status.set_title("No cached articles");
                    status.set_description(Some("Articles from this feed will appear after sync."));
                    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    content.append(&status);
                    content.append(&scroller);
                    controller.entries_for_view(format!("feed:{feed_id}"), {
                        let model = model.clone();
                        let toast = toast.clone();
                        let status = status.clone();
                        let scroller = scroller.clone();
                        move |result| match result {
                            Ok(entries) => {
                                let empty = entries.is_empty();
                                ui::inbox::replace(&model, entries);
                                status.set_visible(empty);
                                scroller.set_visible(!empty);
                            }
                            Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                        }
                    });
                    list.connect_activate({
                        let controller = controller.clone();
                        let reader = reader.clone();
                        let current = current.clone();
                        let inbox = inbox.clone();
                        let toast = toast.clone();
                        let undo = undo.clone();
                        move |_, position| {
                            if let Some(entry) = ui::inbox::entry_at(&model, position) {
                                *current.borrow_mut() = Some(entry.clone());
                                set_reader_origin(&reader, &model, entry.id, false);
                                open_article(&reader, &inbox, &entry);
                                if !entry.read {
                                    mark_read(
                                        ReadContext {
                                            controller: controller.clone(),
                                            inbox: inbox.clone(),
                                            toast: toast.clone(),
                                            undo: undo.clone(),
                                            reader: reader.clone(),
                                            current: current.clone(),
                                        },
                                        entry,
                                        false,
                                    );
                                }
                            }
                        }
                    });
                    let toolbar = adw::ToolbarView::new();
                    toolbar.add_top_bar(&adw::HeaderBar::new());
                    toolbar.set_content(Some(&content));
                    navigation.push(&adw::NavigationPage::new(&toolbar, "Feed articles"));
                }
            });
            window.add_action(&feed_action);
            let back_action = gio::SimpleAction::new("back", None);
            back_action.connect_activate({
                let reader = reader_ui.clone();
                let navigation = library_navigation.clone();
                move |_, _| {
                    if reader.split.is_collapsed() && reader.split.shows_content() {
                        reader.split.set_show_content(false);
                    } else {
                        navigation.pop();
                    }
                }
            });
            window.add_action(&back_action);

            let sync_action = gio::SimpleAction::new("sync", None);
            sync_action.connect_activate({
                let controller = controller.clone();
                let inbox_ui = inbox_ui.clone();
                let toast_overlay = toast_overlay.clone();
                let views = other_views.clone();
                move |action, _| {
                    begin_sync(
                        controller.clone(),
                        inbox_ui.clone(),
                        toast_overlay.clone(),
                        action.clone(),
                        views.clone(),
                    );
                }
            });
            application.add_action(&sync_action);

            let setup_action = gio::SimpleAction::new("setup", None);
            setup_action.connect_activate({
                let window = window.downgrade();
                let controller = controller.clone();
                let inbox_status = inbox_status.clone();
                let setup_button = setup_button.clone();
                let toast_overlay = toast_overlay.clone();
                let inbox_ui = inbox_ui.clone();
                let sync_action = sync_action.clone();
                let views = other_views.clone();
                move |_, _| {
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    ui::setup::present(&window, controller.clone(), {
                        let inbox_status = inbox_status.clone();
                        let setup_button = setup_button.clone();
                        let toast_overlay = toast_overlay.clone();
                        let controller = controller.clone();
                        let inbox_ui = inbox_ui.clone();
                        let sync_action = sync_action.clone();
                        let views = views.clone();
                        move |_account| {
                            show_account(&inbox_status, &setup_button);
                            toast_overlay.add_toast(adw::Toast::new("Miniflux account connected"));
                            begin_sync(
                                controller.clone(),
                                inbox_ui.clone(),
                                toast_overlay.clone(),
                                sync_action.clone(),
                                views.clone(),
                            );
                        }
                    });
                }
            });
            window.add_action(&setup_action);

            window.present();

            controller.existing_account({
                let setup_action = setup_action.clone();
                let controller = controller.clone();
                let inbox_ui = inbox_ui.clone();
                let toast_overlay = toast_overlay.clone();
                let sync_action = sync_action.clone();
                let views = other_views.clone();
                move |result| match result {
                    Ok(Some(_account)) => {
                        show_account(&inbox_status, &setup_button);
                        controller.cached_inbox({
                            let controller = controller.clone();
                            let inbox_ui = inbox_ui.clone();
                            let toast_overlay = toast_overlay.clone();
                            let sync_action = sync_action.clone();
                            let views = views.clone();
                            move |result| {
                                match result {
                                    Ok(entries) => show_entries(&inbox_ui, entries),
                                    Err(error) => toast_overlay
                                        .add_toast(adw::Toast::new(&error.sync_message())),
                                }
                                load_other_views(
                                    controller.clone(),
                                    views.clone(),
                                    toast_overlay.clone(),
                                );
                                begin_sync(controller, inbox_ui, toast_overlay, sync_action, views);
                            }
                        });
                    }
                    Ok(None) => {
                        setup_action.activate(None);
                    }
                    Err(error) => {
                        toast_overlay.add_toast(adw::Toast::new(&error.setup_message()));
                    }
                }
            });
        });
    }

    fn install_actions(&self) {
        let quit = gio::ActionEntry::builder("quit")
            .activate(|application: &adw::Application, _, _| application.quit())
            .build();
        self.application.add_action_entries([quit]);
        self.application
            .set_accels_for_action("app.quit", &["<primary>q"]);
        self.application
            .set_accels_for_action("app.sync", &["<primary>r"]);
        self.application
            .set_accels_for_action("app.search", &["<primary>f"]);
        self.application
            .set_accels_for_action("win.undo", &["<primary>z"]);
        self.application
            .set_accels_for_action("win.back", &["<alt>Left", "Escape"]);
    }
}

fn opens_shortcuts(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    if key == gtk::gdk::Key::F1 && modifiers.is_empty() {
        return true;
    }
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers.intersects(
            gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK
                | gtk::gdk::ModifierType::META_MASK,
        )
        && (key == gtk::gdk::Key::question
            || (key == gtk::gdk::Key::slash
                && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)))
}

fn install_article_cursor_keys(
    window: &adw::ApplicationWindow,
    destinations: &adw::ViewStack,
    reader_scroller: &gtk::ScrolledWindow,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let cursor_mode = Rc::new(Cell::new(false));
    keys.connect_key_pressed({
        let window = window.downgrade();
        let destinations = destinations.downgrade();
        let reader_scroller = reader_scroller.downgrade();
        let cursor_mode = cursor_mode.clone();
        move |_, key, _, modifiers| {
            let (Some(window), Some(destinations), Some(reader_scroller)) = (
                window.upgrade(),
                destinations.upgrade(),
                reader_scroller.upgrade(),
            ) else {
                return adw::glib::Propagation::Proceed;
            };
            if opens_shortcuts(key, modifiers) {
                return if gtk::prelude::WidgetExt::activate_action(
                    &window,
                    "win.show-shortcuts",
                    None,
                )
                .is_ok()
                {
                    adw::glib::Propagation::Stop
                } else {
                    adw::glib::Propagation::Proceed
                };
            }
            if key == gtk::gdk::Key::Tab || key == gtk::gdk::Key::ISO_Left_Tab {
                cursor_mode.set(false);
                return adw::glib::Propagation::Proceed;
            }
            if key == gtk::gdk::Key::Escape {
                cursor_mode.set(false);
                return adw::glib::Propagation::Proceed;
            }
            if !modifiers.is_empty() {
                return adw::glib::Propagation::Proceed;
            }
            if key == gtk::gdk::Key::u && reader_scroller.is_visible() {
                let mut focus = gtk::prelude::GtkWindowExt::focus(&window);
                while let Some(widget) = focus {
                    if widget.is::<gtk::Editable>()
                        || widget.is::<gtk::TextView>()
                        || widget.is::<adw::Dialog>()
                        || widget.is::<gtk::Popover>()
                    {
                        return adw::glib::Propagation::Proceed;
                    }
                    focus = widget.parent();
                }
                if gtk::prelude::WidgetExt::activate_action(&window, "win.keep-unread", None)
                    .is_ok()
                {
                    return adw::glib::Propagation::Stop;
                }
            }
            if (key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter) && cursor_mode.get()
            {
                let list = destinations
                    .visible_child()
                    .and_then(|child| mapped_article_list(&child));
                if let Some(list) = list
                    && let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>()
                    && selection.selected() != gtk::INVALID_LIST_POSITION
                {
                    cursor_mode.set(false);
                    list.emit_by_name::<()>("activate", &[&selection.selected()]);
                    return adw::glib::Propagation::Stop;
                }
            }
            let Some(direction) = ui::inbox::cursor_direction(key) else {
                return adw::glib::Propagation::Proceed;
            };
            if let Some(focus) = gtk::prelude::GtkWindowExt::focus(&window) {
                let mut current = Some(focus);
                while let Some(widget) = current {
                    if widget.is::<gtk::ListBox>()
                        || widget.is::<gtk::Editable>()
                        || widget.is::<gtk::TextView>()
                        || widget.is::<gtk::DropDown>()
                        || ((key == gtk::gdk::Key::Up || key == gtk::gdk::Key::Down)
                            && widget == reader_scroller)
                    {
                        return adw::glib::Propagation::Proceed;
                    }
                    current = widget.parent();
                }
            }
            let Some(list) = destinations
                .visible_child()
                .and_then(|child| mapped_article_list(&child))
            else {
                return adw::glib::Propagation::Proceed;
            };
            if ui::inbox::move_cursor(&list, direction) {
                cursor_mode.set(true);
                adw::glib::Propagation::Stop
            } else {
                adw::glib::Propagation::Proceed
            }
        }
    });
    window.add_controller(keys);
    window.connect_focus_widget_notify({
        let destinations = destinations.downgrade();
        move |window| {
            if !cursor_mode.get() {
                return;
            }
            let Some(destinations) = destinations.upgrade() else {
                cursor_mode.set(false);
                return;
            };
            let focus = gtk::prelude::GtkWindowExt::focus(window);
            let list = destinations
                .visible_child()
                .and_then(|child| mapped_article_list(&child));
            if !focus.zip(list).is_some_and(|(focus, list)| {
                focus == list.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&list)
            }) {
                cursor_mode.set(false);
            }
        }
    });
}

fn mapped_article_list(widget: &gtk::Widget) -> Option<gtk::ListView> {
    if !widget.is_mapped() {
        return None;
    }
    if let Ok(list) = widget.clone().downcast::<gtk::ListView>() {
        return Some(list);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(list) = mapped_article_list(&widget) {
            return Some(list);
        }
        child = widget.next_sibling();
    }
    None
}

fn begin_sync(
    controller: Arc<AppController>,
    inbox_ui: InboxUi,
    toast_overlay: adw::ToastOverlay,
    sync_action: gio::SimpleAction,
    views: OtherViews,
) {
    if !sync_action.is_enabled() {
        return;
    }
    sync_action.set_enabled(false);
    inbox_ui.spinner.set_visible(true);
    let reload_controller = controller.clone();
    controller.sync(move |result| {
        sync_action.set_enabled(true);
        inbox_ui.spinner.set_visible(false);
        match result {
            Ok(result) => {
                let before = inbox_snapshot(&inbox_ui);
                let changes = ui::inbox::InboxChanges::between(&before, &result.inbox);
                show_entries(&inbox_ui, result.inbox);
                load_other_views(reload_controller, views, toast_overlay.clone());
                if let Some(message) = changes.toast_message() {
                    toast_overlay.add_toast(adw::Toast::new(&message));
                }
            }
            Err(error) => {
                toast_overlay.add_toast(adw::Toast::new(&error.sync_message()));
            }
        }
    });
}

fn inbox_snapshot(inbox_ui: &InboxUi) -> Vec<Entry> {
    if let Some(entries) = inbox_ui.pending_entries.borrow().as_ref() {
        return entries.clone();
    }
    (0..inbox_ui.model.store.n_items())
        .filter_map(|position| ui::inbox::entry_at(&inbox_ui.model, position))
        .collect()
}

fn show_entries(inbox_ui: &InboxUi, entries: Vec<Entry>) {
    if inbox_ui.scroller.vadjustment().value() > 40.0 && inbox_ui.model.store.n_items() > 0 {
        let changes = ui::inbox::InboxChanges::between(&inbox_snapshot(inbox_ui), &entries);
        if changes.added > 0 || changes.updated > 0 {
            let new_count = changes.added;
            inbox_ui.new_button.set_label(if new_count == 0 {
                "Inbox updated"
            } else if new_count == 1 {
                "1 new article"
            } else {
                "New articles"
            });
            if new_count > 1 {
                inbox_ui
                    .new_button
                    .set_label(&format!("{new_count} new articles"));
            }
            *inbox_ui.pending_entries.borrow_mut() = Some(entries);
            inbox_ui.new_button.set_visible(true);
            return;
        }
    }
    inbox_ui.new_button.set_visible(false);
    inbox_ui.pending_entries.borrow_mut().take();
    apply_inbox_entries(inbox_ui, entries);
}

fn apply_inbox_entries(inbox_ui: &InboxUi, mut entries: Vec<Entry>) {
    if let Some(pinned) = inbox_ui.pinned_read.borrow().as_ref() {
        if let Some(position) = entries.iter().position(|entry| entry.id == pinned.id) {
            entries[position] = pinned.clone();
        } else {
            let position = entries
                .iter()
                .position(|entry| {
                    (entry.published_at_ms, entry.id) < (pinned.published_at_ms, pinned.id)
                })
                .unwrap_or(entries.len());
            entries.insert(position, pinned.clone());
        }
    }
    let selected_id = ui::inbox::selected_id(&inbox_ui.list);
    inbox_ui.rebuilding.set(true);
    ui::inbox::replace(&inbox_ui.model, entries);
    if let Some(id) = selected_id {
        ui::inbox::select_id(&inbox_ui.list, id);
    }
    inbox_ui.rebuilding.set(false);
    update_inbox_visibility(inbox_ui);
}

fn release_read_pin_unless(inbox_ui: &InboxUi, keep_id: Option<i64>) {
    let removed_id = {
        let mut pinned = inbox_ui.pinned_read.borrow_mut();
        if pinned
            .as_ref()
            .is_some_and(|entry| Some(entry.id) == keep_id)
        {
            return;
        }
        pinned.take().map(|entry| entry.id)
    };
    if let Some(id) = removed_id {
        ui::inbox::remove_entry(&inbox_ui.model, id);
        update_inbox_visibility(inbox_ui);
    }
}

fn pin_read(inbox_ui: &InboxUi, entry: Entry) -> bool {
    if ui::inbox::entry_by_id(&inbox_ui.model, entry.id).is_none() {
        return false;
    }
    release_read_pin_unless(inbox_ui, Some(entry.id));
    let entry = Entry {
        read: true,
        ..entry
    };
    *inbox_ui.pinned_read.borrow_mut() = Some(entry.clone());
    inbox_ui.rebuilding.set(true);
    ui::inbox::update_entry(&inbox_ui.model, entry.clone());
    ui::inbox::select_id(&inbox_ui.list, entry.id);
    inbox_ui.rebuilding.set(false);
    true
}

fn update_inbox_visibility(inbox_ui: &InboxUi) {
    let empty = inbox_ui.model.store.n_items() == 0;
    inbox_ui.status.set_visible(empty);
    inbox_ui.scroller.set_visible(!empty);
    if empty {
        inbox_ui.status.set_title("You’re all caught up");
        inbox_ui
            .status
            .set_description(Some("There are no unread articles in Miniflux."));
    }
}

fn update_reader_read_state(
    reader: &ReaderUi,
    current: &Rc<RefCell<Option<Entry>>>,
    entry_id: i64,
    read: bool,
) {
    if let Some(entry) = current.borrow_mut().as_mut()
        && entry.id == entry_id
    {
        entry.read = read;
    }
    for entry in reader.origin_set.borrow_mut().iter_mut() {
        if entry.id == entry_id {
            entry.read = read;
        }
    }
}

fn restore_unread_row(inbox: &InboxUi, entry: Entry) {
    let entry = Entry {
        read: false,
        ..entry
    };
    let was_pinned = inbox
        .pinned_read
        .borrow()
        .as_ref()
        .is_some_and(|pinned| pinned.id == entry.id);
    if was_pinned {
        inbox.pinned_read.borrow_mut().take();
    }
    let selected_id = ui::inbox::selected_id(&inbox.list);
    inbox.rebuilding.set(true);
    if !ui::inbox::update_entry(&inbox.model, entry.clone()) {
        ui::inbox::insert_entry(&inbox.model, entry);
    }
    if let Some(id) = selected_id {
        ui::inbox::select_id(&inbox.list, id);
    }
    inbox.rebuilding.set(false);
    update_inbox_visibility(inbox);
}

fn mark_unread(
    controller: Arc<AppController>,
    inbox: InboxUi,
    reader: ReaderUi,
    current: Rc<RefCell<Option<Entry>>>,
    undo: Rc<RefCell<Vec<Entry>>>,
    toast: adw::ToastOverlay,
    entry: Entry,
) {
    let entry_id = entry.id;
    if !entry.read && !inbox.read_in_flight.borrow().contains(&entry_id) {
        return;
    }
    if inbox.read_in_flight.borrow().contains(&entry_id) {
        inbox.suppress_read.borrow_mut().insert(entry_id);
        restore_unread_row(&inbox, entry);
        update_reader_read_state(&reader, &current, entry_id, false);
        undo.borrow_mut().retain(|item| item.id != entry_id);
        toast.add_toast(adw::Toast::new("Kept unread"));
        return;
    }
    controller.set_read_local(entry_id, false, move |result| match result {
        Ok(()) => {
            restore_unread_row(&inbox, entry);
            update_reader_read_state(&reader, &current, entry_id, false);
            undo.borrow_mut().retain(|item| item.id != entry_id);
            toast.add_toast(adw::Toast::new("Kept unread"));
        }
        Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
    });
}

fn mark_read(context: ReadContext, entry: Entry, retain_current: bool) {
    let ReadContext {
        controller,
        inbox,
        toast,
        undo,
        reader,
        current,
    } = context;
    if !inbox.read_in_flight.borrow_mut().insert(entry.id) {
        return;
    }
    let entry_id = entry.id;
    let restore_controller = controller.clone();
    controller.set_read_local(entry_id, true, move |result| {
        inbox.read_in_flight.borrow_mut().remove(&entry_id);
        if inbox.suppress_read.borrow_mut().remove(&entry_id) {
            if result.is_ok() {
                let toast = toast.clone();
                restore_controller.set_read_local(entry_id, false, move |result| {
                    if let Err(error) = result {
                        toast.add_toast(adw::Toast::new(&error.sync_message()));
                    }
                });
            } else if let Err(error) = result {
                toast.add_toast(adw::Toast::new(&error.sync_message()));
            }
            return;
        }
        match result {
            Ok(()) => {
                update_reader_read_state(&reader, &current, entry_id, true);
                if retain_current
                    && ui::inbox::selected_id(&inbox.list) == Some(entry_id)
                    && pin_read(&inbox, entry.clone())
                {
                    *undo.borrow_mut() = vec![entry];
                } else if let Some(removed) = ui::inbox::remove_entry(&inbox.model, entry_id) {
                    *undo.borrow_mut() = vec![removed];
                } else {
                    *undo.borrow_mut() = vec![entry];
                }
                update_inbox_visibility(&inbox);
                let notification = adw::Toast::new("Marked as read");
                notification.set_button_label(Some("Undo"));
                notification.set_action_name(Some("win.undo"));
                toast.add_toast(notification);
            }
            Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
        }
    });
}

fn undo_mark_read(
    controller: Arc<AppController>,
    inbox_ui: InboxUi,
    toast_overlay: adw::ToastOverlay,
    undo_entry: Rc<RefCell<Vec<Entry>>>,
    reader: ReaderUi,
    current: Rc<RefCell<Option<Entry>>>,
) {
    let entries = std::mem::take(&mut *undo_entry.borrow_mut());
    if entries.is_empty() {
        return;
    }
    let ids = entries.iter().map(|entry| entry.id).collect::<Vec<_>>();
    controller.set_read_many_local(ids, false, move |result| match result {
        Ok(()) => {
            for entry in entries {
                let entry_id = entry.id;
                restore_unread_row(&inbox_ui, entry);
                update_reader_read_state(&reader, &current, entry_id, false);
            }
            toast_overlay.add_toast(adw::Toast::new("Restored to Inbox"));
        }
        Err(error) => {
            *undo_entry.borrow_mut() = entries;
            toast_overlay.add_toast(adw::Toast::new(&error.sync_message()));
        }
    });
}

fn open_article(reader_ui: &ReaderUi, inbox_ui: &InboxUi, entry: &Entry) {
    release_read_pin_unless(inbox_ui, Some(entry.id));
    let entry_id = entry.id;
    reader_ui.restoring.set(true);
    reader_ui.active_id.set(Some(entry.id));
    let images = ui::reader::show(entry, &reader_ui.title, &reader_ui.content);
    for slot in images {
        let reader = reader_ui.clone();
        reader_ui.controller.fetch_image(slot.url, move |result| {
            let Ok(bytes) = result else {
                return;
            };
            let handle = reader.controller.backend_handle();
            adw::glib::MainContext::default().spawn_local(async move {
                let mut decode = Box::pin(async move {
                    let Ok(mut image) = glycin::Loader::new_vec(bytes).load().await else {
                        return;
                    };
                    let Ok(frame) = image.next_frame().await else {
                        return;
                    };
                    if reader.active_id.get() != Some(entry_id) {
                        return;
                    }
                    let picture = gtk::Picture::for_paintable(&frame.texture());
                    picture.set_alternative_text(Some(&slot.alt));
                    picture.set_content_fit(gtk::ContentFit::Contain);
                    picture.set_can_shrink(true);
                    while let Some(child) = slot.container.first_child() {
                        slot.container.remove(&child);
                    }
                    slot.container.append(&picture);
                });
                std::future::poll_fn(|context| {
                    let _entered = handle.enter();
                    std::future::Future::poll(decode.as_mut(), context)
                })
                .await;
            });
        });
    }
    reader_ui.placeholder.set_visible(false);
    reader_ui.scroller.set_visible(true);
    reader_ui.split.set_show_content(true);
    reader_ui.controller.reader_position(entry.id, {
        let reader = reader_ui.clone();
        move |result| {
            let position = result.ok().flatten();
            adw::glib::timeout_add_local_once(Duration::from_millis(100), move || {
                if reader.active_id.get() != Some(entry_id) {
                    return;
                }
                let value = position.map_or(0, |position| {
                    offset_from_reader_position(&reader.content, &position)
                });
                reader.scroller.vadjustment().set_value(f64::from(value));
                reader.restoring.set(false);
            });
        }
    });
}

fn set_reader_origin(
    reader: &ReaderUi,
    model: &ui::inbox::InboxModel,
    entry_id: i64,
    from_inbox: bool,
) {
    let entries = (0..model.store.n_items())
        .filter_map(|position| ui::inbox::entry_at(model, position))
        .collect::<Vec<_>>();
    set_reader_origin_entries(reader, entries, entry_id, from_inbox);
}

fn set_reader_origin_entries(
    reader: &ReaderUi,
    entries: Vec<Entry>,
    entry_id: i64,
    from_inbox: bool,
) {
    reader.origin_inbox.set(from_inbox);
    reader.origin_index.set(
        entries
            .iter()
            .position(|entry| entry.id == entry_id)
            .unwrap_or(0),
    );
    *reader.origin_set.borrow_mut() = entries;
}

fn reader_position_from_offset(content: &gtk::Box, entry_id: i64, scroll: i32) -> ReaderPosition {
    let mut top = 0;
    let mut index = 0;
    let mut child = content.first_child();
    while let Some(widget) = child {
        let height = widget.height() + 18;
        if top + height > scroll {
            break;
        }
        top += height;
        index += 1;
        child = widget.next_sibling();
    }
    ReaderPosition {
        entry_id,
        first_visible_block: index,
        offset_px: scroll.saturating_sub(top),
    }
}

fn offset_from_reader_position(content: &gtk::Box, position: &ReaderPosition) -> i32 {
    let mut top = 0;
    let mut child = content.first_child();
    for _ in 0..position.first_visible_block {
        let Some(widget) = child else {
            break;
        };
        top += widget.height() + 18;
        child = widget.next_sibling();
    }
    top + position.offset_px
}

fn load_other_views(controller: Arc<AppController>, views: OtherViews, toast: adw::ToastOverlay) {
    for (view, model, status, scroller) in [
        (
            "saved",
            views.saved.clone(),
            views.saved_status.clone(),
            views.saved_scroller.clone(),
        ),
        (
            "all",
            views.library_all.clone(),
            views.library_all_status.clone(),
            views.library_all_scroller.clone(),
        ),
        (
            "unread",
            views.library_unread.clone(),
            views.library_unread_status.clone(),
            views.library_unread_scroller.clone(),
        ),
        (
            "read",
            views.library_read.clone(),
            views.library_read_status.clone(),
            views.library_read_scroller.clone(),
        ),
    ] {
        let toast = toast.clone();
        controller.entries_for_view(view.into(), move |result| match result {
            Ok(entries) => {
                let empty = entries.is_empty();
                ui::inbox::replace(&model, entries);
                status.set_visible(empty);
                scroller.set_visible(!empty);
            }
            Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
        });
    }
    controller.categories_cached({
        let list = views.categories.clone();
        let status = views.categories_status.clone();
        let scroller = views.categories_scroller.clone();
        let toast = toast.clone();
        move |result| match result {
            Ok(categories) => {
                let empty = categories.is_empty();
                while let Some(row) = list.first_child() {
                    list.remove(&row);
                }
                for category in categories {
                    let row = adw::ActionRow::builder()
                        .title(&category.title)
                        .use_markup(false)
                        .activatable(true)
                        .build();
                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                    row.set_action_name(Some("win.library-category"));
                    row.set_action_target_value(Some(&category.id.to_variant()));
                    list.append(&row);
                }
                status.set_visible(empty);
                scroller.set_visible(!empty);
            }
            Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
        }
    });
}

fn install_reader_actions(window: &adw::ApplicationWindow, tools: WindowTools) {
    let WindowTools {
        controller,
        reader,
        current,
        inbox,
        views,
        toast,
        undo,
    } = tools;
    for (name, step) in [("previous-article", -1_isize), ("next-article", 1_isize)] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate({
            let reader = reader.clone();
            let current = current.clone();
            let controller = controller.clone();
            let inbox = inbox.clone();
            let toast = toast.clone();
            let undo = undo.clone();
            move |_, _| {
                let index = reader.origin_index.get();
                let next = if step < 0 {
                    index.checked_sub(1)
                } else {
                    index.checked_add(1)
                };
                let Some(next) = next else {
                    return;
                };
                let Some(entry) = reader.origin_set.borrow().get(next).cloned() else {
                    return;
                };
                reader.origin_index.set(next);
                *current.borrow_mut() = Some(entry.clone());
                if reader.origin_inbox.get() {
                    ui::inbox::select_id(&inbox.list, entry.id);
                }
                open_article(&reader, &inbox, &entry);
                if !entry.read {
                    let retain = reader.origin_inbox.get() && !reader.split.is_collapsed();
                    mark_read(
                        ReadContext {
                            controller: controller.clone(),
                            inbox: inbox.clone(),
                            toast: toast.clone(),
                            undo: undo.clone(),
                            reader: reader.clone(),
                            current: current.clone(),
                        },
                        entry,
                        retain,
                    );
                }
            }
        });
        window.add_action(&action);
    }
    let keep = gio::SimpleAction::new("keep-unread", None);
    keep.connect_activate({
        let controller = controller.clone();
        let current = current.clone();
        let reader = reader.clone();
        let inbox = inbox.clone();
        let toast = toast.clone();
        let undo = undo.clone();
        move |_, _| {
            let Some(entry) = current.borrow().clone() else {
                return;
            };
            mark_unread(
                controller.clone(),
                inbox.clone(),
                reader.clone(),
                current.clone(),
                undo.clone(),
                toast.clone(),
                entry,
            );
        }
    });
    window.add_action(&keep);
    let star = gio::SimpleAction::new("toggle-star", None);
    let star_in_flight = Rc::new(RefCell::new(HashSet::new()));
    star.connect_activate({
        let controller = controller.clone();
        let current = current.clone();
        let views = views.clone();
        let toast = toast.clone();
        let star_in_flight = star_in_flight.clone();
        move |_, _| {
            let Some(entry) = current.borrow().clone() else {
                return;
            };
            if !star_in_flight.borrow_mut().insert(entry.id) {
                return;
            }
            let entry_id = entry.id;
            let desired = !entry.starred;
            controller.set_starred_local(entry_id, desired, {
                let current = current.clone();
                let controller = controller.clone();
                let views = views.clone();
                let toast = toast.clone();
                let reader = reader.clone();
                let star_in_flight = star_in_flight.clone();
                move |result| match result {
                    Ok(()) => {
                        star_in_flight.borrow_mut().remove(&entry_id);
                        if let Some(active) = current
                            .borrow_mut()
                            .as_mut()
                            .filter(|active| active.id == entry_id)
                        {
                            active.starred = desired;
                        }
                        if let Some(origin) = reader
                            .origin_set
                            .borrow_mut()
                            .iter_mut()
                            .find(|item| item.id == entry_id)
                        {
                            origin.starred = desired;
                        }
                        load_other_views(controller, views, toast.clone());
                        toast.add_toast(adw::Toast::new(if desired {
                            "Saved"
                        } else {
                            "Removed from Saved"
                        }));
                    }
                    Err(error) => {
                        star_in_flight.borrow_mut().remove(&entry_id);
                        toast.add_toast(adw::Toast::new(&error.sync_message()));
                    }
                }
            });
        }
    });
    window.add_action(&star);
    let send = gio::SimpleAction::new("send-karakeep", None);
    send.connect_activate({
        let controller = controller.clone();
        let current = current.clone();
        let toast = toast.clone();
        move |_, _| {
            let Some(entry) = current.borrow().clone() else {
                return;
            };
            controller.queue_karakeep(entry, {
                let toast = toast.clone();
                move |result| match result {
                    Ok(()) => toast.add_toast(adw::Toast::new("Queued for Karakeep delivery")),
                    Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                }
            });
        }
    });
    window.add_action(&send);
    let copy = gio::SimpleAction::new("copy-link", None);
    copy.connect_activate({
        let current = current.clone();
        let toast = toast.clone();
        move |_, _| {
            if let (Some(entry), Some(display)) =
                (current.borrow().as_ref(), gtk::gdk::Display::default())
            {
                display.clipboard().set_text(&entry.url);
                toast.add_toast(adw::Toast::new("Link copied"));
            }
        }
    });
    window.add_action(&copy);
    let browser = gio::SimpleAction::new("open-browser", None);
    browser.connect_activate({
        let current = current.clone();
        let window = window.downgrade();
        let toast = toast.clone();
        move |_, _| {
            if let (Some(entry), Some(window)) = (current.borrow().as_ref(), window.upgrade()) {
                let launcher = gtk::UriLauncher::new(&entry.url);
                launcher.launch(Some(&window), gio::Cancellable::NONE, {
                    let toast = toast.clone();
                    move |result| {
                        if let Err(error) = result {
                            toast.add_toast(adw::Toast::new(&error.to_string()));
                        }
                    }
                });
            }
        }
    });
    window.add_action(&browser);
}

fn install_window_tools(
    window: &adw::ApplicationWindow,
    application: &adw::Application,
    builder: &gtk::Builder,
    tools: WindowTools,
) {
    let WindowTools {
        controller,
        reader,
        current,
        inbox,
        views,
        toast,
        undo,
    } = tools;
    let menu_button: gtk::MenuButton = builder.object("app_menu").expect("app_menu");
    let menu = gio::Menu::new();
    menu.append(Some("Mark All Read"), Some("win.mark-all-read"));
    for (label, action) in [
        ("Refresh Feeds", "win.refresh-feeds"),
        ("Subscribe…", "win.subscribe"),
        ("Preferences", "win.preferences"),
        ("Keyboard Shortcuts", "win.show-shortcuts"),
        ("About Brooklet", "win.about"),
    ] {
        menu.append(Some(label), Some(action));
    }
    menu_button.set_menu_model(Some(&menu));

    let search = gio::SimpleAction::new("search", None);
    search.connect_activate({
        let window = window.downgrade();
        let controller = controller.clone();
        let reader = reader.clone();
        let current = current.clone();
        let inbox = inbox.clone();
        let toast = toast.clone();
        let undo = undo.clone();
        move |_, _| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let dialog = adw::Dialog::new();
            dialog.set_title("Search Library");
            dialog.set_content_width(620);
            dialog.set_content_height(600);
            let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let query = gtk::SearchEntry::new();
            query.set_placeholder_text(Some("Search title, feed, author and content"));
            query.set_margin_start(12);
            query.set_margin_end(12);
            query.set_margin_top(12);
            body.append(&query);
            let filters = gtk::FlowBox::new();
            filters.set_selection_mode(gtk::SelectionMode::None);
            filters.set_row_spacing(8);
            filters.set_column_spacing(8);
            filters.set_margin_start(12);
            filters.set_margin_end(12);
            let feed = gtk::DropDown::from_strings(&["All feeds"]);
            feed.set_hexpand(true);
            feed.set_tooltip_text(Some("Narrow by feed"));
            filters.append(&feed);
            let category = gtk::DropDown::from_strings(&["All categories"]);
            category.set_hexpand(true);
            category.set_tooltip_text(Some("Narrow by category"));
            filters.append(&category);
            let read = gtk::DropDown::from_strings(&["Any status", "Unread", "Read"]);
            read.set_tooltip_text(Some("Narrow by read status"));
            filters.append(&read);
            body.append(&filters);
            let scroller = gtk::ScrolledWindow::new();
            scroller.set_vexpand(true);
            scroller.set_visible(false);
            let list =
                gtk::ListView::new(None::<gtk::SelectionModel>, None::<gtk::ListItemFactory>);
            let model = ui::inbox::configure_with_action(&list, false);
            scroller.set_child(Some(&list));
            let status = adw::StatusPage::new();
            status.set_vexpand(true);
            status.set_icon_name(Some("system-search-symbolic"));
            status.set_title("No matching articles");
            status.set_description(Some("Try a different search or filter."));
            body.append(&status);
            body.append(&scroller);
            dialog.set_child(Some(&body));
            let state = SearchState {
                controller: controller.clone(),
                query: query.clone(),
                feed: feed.clone(),
                category: category.clone(),
                read: read.clone(),
                feed_ids: Rc::new(RefCell::new(vec![None])),
                category_ids: Rc::new(RefCell::new(vec![None])),
                model: model.clone(),
                status,
                scroller,
                generation: Rc::new(Cell::new(0)),
                toast: toast.clone(),
            };
            query.connect_search_changed({
                let state = state.clone();
                move |_| state.refresh()
            });
            feed.connect_selected_notify({
                let state = state.clone();
                move |_| state.refresh()
            });
            category.connect_selected_notify({
                let state = state.clone();
                move |_| state.refresh()
            });
            read.connect_selected_notify({
                let state = state.clone();
                move |_| state.refresh()
            });
            controller.feeds_cached(None, {
                let state = state.clone();
                move |result| {
                    if let Ok(feeds) = result
                        && let Some(model) = state.feed.model().and_downcast::<gtk::StringList>()
                    {
                        for item in feeds {
                            model.append(&item.title);
                            state.feed_ids.borrow_mut().push(Some(item.id));
                        }
                    }
                }
            });
            controller.categories_cached({
                let state = state.clone();
                move |result| {
                    if let Ok(categories) = result
                        && let Some(model) =
                            state.category.model().and_downcast::<gtk::StringList>()
                    {
                        for item in categories {
                            model.append(&item.title);
                            state.category_ids.borrow_mut().push(Some(item.id));
                        }
                    }
                }
            });
            state.refresh();
            list.connect_activate({
                let dialog = dialog.clone();
                let controller = controller.clone();
                let reader = reader.clone();
                let current = current.clone();
                let inbox = inbox.clone();
                let toast = toast.clone();
                let undo = undo.clone();
                move |_, position| {
                    if let Some(entry) = ui::inbox::entry_at(&model, position) {
                        dialog.close();
                        *current.borrow_mut() = Some(entry.clone());
                        set_reader_origin(&reader, &model, entry.id, false);
                        open_article(&reader, &inbox, &entry);
                        if !entry.read {
                            mark_read(
                                ReadContext {
                                    controller: controller.clone(),
                                    inbox: inbox.clone(),
                                    toast: toast.clone(),
                                    undo: undo.clone(),
                                    reader: reader.clone(),
                                    current: current.clone(),
                                },
                                entry,
                                false,
                            );
                        }
                    }
                }
            });
            dialog.present(Some(&window));
            query.grab_focus();
        }
    });
    application.add_action(&search);

    let refresh = gio::SimpleAction::new("refresh-feeds", None);
    refresh.connect_activate({
        let controller = controller.clone();
        let inbox = inbox.clone();
        let views = views.clone();
        let toast = toast.clone();
        move |action, _| {
            action.set_enabled(false);
            controller.refresh_feeds({
                let action = action.clone();
                let controller = controller.clone();
                let inbox = inbox.clone();
                let views = views.clone();
                let toast = toast.clone();
                move |result| {
                    action.set_enabled(true);
                    match result {
                        Ok(result) => {
                            let before = inbox_snapshot(&inbox);
                            let changes = ui::inbox::InboxChanges::between(&before, &result.inbox);
                            show_entries(&inbox, result.inbox);
                            load_other_views(controller, views, toast.clone());
                            if let Some(message) = changes.toast_message() {
                                toast.add_toast(adw::Toast::new(&message));
                            }
                        }
                        Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                    }
                }
            });
        }
    });
    window.add_action(&refresh);

    let subscribe = gio::SimpleAction::new("subscribe", None);
    subscribe.connect_activate({
        let window = window.downgrade();
        let controller = controller.clone();
        let toast = toast.clone();
        move |_, _| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let dialog = adw::Dialog::new();
            dialog.set_title("Subscribe to a feed");
            dialog.set_content_width(430);
            let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
            body.set_margin_top(20);
            body.set_margin_bottom(20);
            body.set_margin_start(20);
            body.set_margin_end(20);
            let url = gtk::Entry::new();
            url.set_placeholder_text(Some("https://example.com/feed.xml"));
            url.set_input_purpose(gtk::InputPurpose::Url);
            body.append(&url);
            let error = gtk::Label::new(None);
            error.set_xalign(0.0);
            error.set_wrap(true);
            error.add_css_class("error");
            error.set_visible(false);
            body.append(&error);
            let button = gtk::Button::with_label("Subscribe");
            button.add_css_class("suggested-action");
            button.set_halign(gtk::Align::End);
            body.append(&button);
            url.connect_activate({
                let button = button.clone();
                move |_| button.emit_clicked()
            });
            url.connect_changed({
                let error = error.clone();
                move |_| error.set_visible(false)
            });
            let submit_button = button.clone();
            let input_url = url.clone();
            button.connect_clicked({
                let dialog = dialog.clone();
                let controller = controller.clone();
                let toast = toast.clone();
                let window = window.downgrade();
                move |_| {
                    let feed_url = input_url.text().trim().to_owned();
                    if !matches!(url::Url::parse(&feed_url), Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() && parsed.username().is_empty() && parsed.password().is_none()) {
                        error.set_label("Enter an HTTP or HTTPS feed URL without credentials.");
                        error.set_visible(true);
                        input_url.grab_focus();
                        return;
                    }
                    submit_button.set_sensitive(false);
                    error.set_visible(false);
                    controller.subscribe(feed_url, None, {
                        let dialog = dialog.clone();
                        let toast = toast.clone();
                        let button = submit_button.clone();
                        let error = error.clone();
                        let window = window.clone();
                        move |result| match result {
                            Ok(()) => {
                                dialog.close();
                                toast.add_toast(adw::Toast::new("Feed subscribed"));
                                if let Some(window) = window.upgrade() {
                                    let _ = gtk::prelude::WidgetExt::activate_action(&window, "app.sync", None);
                                }
                            }
                            Err(failure) => {
                                button.set_sensitive(true);
                                error.set_label(&failure.sync_message());
                                error.set_visible(true);
                            }
                        }
                    });
                }
            });
            dialog.set_child(Some(&body));
            dialog.present(Some(&window));
            url.grab_focus();
        }
    });
    window.add_action(&subscribe);

    let preferences = gio::SimpleAction::new("preferences", None);
    preferences.connect_activate({
        let window = window.downgrade();
        let application = application.clone();
        let controller = controller.clone();
        let toast = toast.clone();
        move |_, _| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let dialog = adw::PreferencesDialog::new();
            let page = adw::PreferencesPage::new();
            let account_group = adw::PreferencesGroup::new();
            account_group.set_title("Miniflux account");
            let account_row = adw::ActionRow::new();
            account_row.set_use_markup(false);
            account_row.set_title("Loading account…");
            account_group.add(&account_row);
            let logout_row = adw::ActionRow::new();
            logout_row.set_title("Log out and clear data");
            logout_row.set_subtitle("Remove this account, cached articles, pending changes, and saved credentials");
            let logout_button = gtk::Button::with_label("Log Out…");
            logout_button.set_valign(gtk::Align::Center);
            logout_button.add_css_class("destructive-action");
            logout_row.add_suffix(&logout_button);
            logout_row.set_activatable_widget(Some(&logout_button));
            account_group.add(&logout_row);
            logout_button.connect_clicked({
                let logout_button = logout_button.clone();
                let window = window.downgrade();
                let application = application.clone();
                let controller = controller.clone();
                let dialog = dialog.clone();
                let toast = toast.clone();
                move |_| {
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    let confirm = adw::AlertDialog::new(
                        Some("Log out and clear local data?"),
                        Some("Brooklet will remove its cached articles, reader positions, pending offline changes, and stored Miniflux and Karakeep credentials. This cannot be undone. Articles and stars already synced to Miniflux will remain there."),
                    );
                    confirm.add_responses(&[("cancel", "Cancel"), ("clear", "Log Out and Clear Data")]);
                    confirm.set_close_response("cancel");
                    confirm.set_default_response(Some("cancel"));
                    confirm.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
                    confirm.connect_response(Some("clear"), {
                        let controller = controller.clone();
                        let dialog = dialog.clone();
                        let application = application.clone();
                        let window = window.downgrade();
                        let toast = toast.clone();
                        let logout_button = logout_button.clone();
                        move |_, _| {
                            logout_button.set_sensitive(false);
                            controller.disconnect({
                                let application = application.clone();
                                let window = window.clone();
                                let dialog = dialog.clone();
                                let toast = toast.clone();
                                let logout_button = logout_button.clone();
                                move |result| match result {
                                    Ok(()) => {
                                        dialog.close();
                                        if let Some(window) = window.upgrade() {
                                            let _hold = application.hold();
                                            window.close();
                                            application.activate();
                                        }
                                    }
                                    Err(error) => {
                                        logout_button.set_sensitive(true);
                                        toast.add_toast(adw::Toast::new(&format!("Could not clear account data: {error}")));
                                    }
                                }
                            });
                        }
                    });
                    confirm.present(Some(&window));
                }
            });
            page.add(&account_group);
            let storage = adw::PreferencesGroup::new();
            storage.set_title("Local cache");
            let days = adw::SpinRow::with_range(1.0, 3650.0, 1.0);
            days.set_title("Keep read articles for days");
            storage.add(&days);
            page.add(&storage);
            let karakeep = adw::PreferencesGroup::new();
            karakeep.set_title("Karakeep delivery");
            let direct = adw::SwitchRow::new();
            direct.set_title("Send directly to Karakeep");
            direct.set_subtitle("Otherwise use Miniflux’s configured integration");
            karakeep.add(&direct);
            let endpoint = adw::EntryRow::new();
            endpoint.set_title("Karakeep endpoint (…/api/v1/bookmarks)");
            endpoint.set_sensitive(false);
            karakeep.add(&endpoint);
            let key = adw::PasswordEntryRow::new();
            key.set_title("API key (leave blank to keep saved key)");
            key.set_sensitive(false);
            karakeep.add(&key);
            direct.connect_active_notify({
                let endpoint = endpoint.clone();
                let key = key.clone();
                move |row| {
                    endpoint.set_sensitive(row.is_active());
                    key.set_sensitive(row.is_active());
                }
            });
            let save_karakeep = gtk::Button::with_label("Save Karakeep settings");
            save_karakeep.set_margin_top(8);
            karakeep.add(&save_karakeep);
            page.add(&karakeep);
            let diagnostics = adw::PreferencesGroup::new();
            diagnostics.set_title("Sync diagnostics");
            let status = adw::ActionRow::new();
            status.set_use_markup(false);
            status.set_title("Loading sync status…");
            diagnostics.add(&status);
            page.add(&diagnostics);
            dialog.add(&page);
            controller.existing_account(move |result| match result {
                Ok(Some(account)) => {
                    account_row.set_title(&account.username);
                    account_row.set_subtitle(&format!(
                        "{} · Miniflux {}",
                        account.server_url, account.server_version
                    ));
                }
                Ok(None) => {
                    account_row.set_title("No account configured");
                    logout_row.set_sensitive(false);
                }
                Err(error) => {
                    account_row.set_title(&error.setup_message());
                    logout_row.set_sensitive(false);
                }
            });
            controller.storage_policy({
                let controller = controller.clone();
                let toast = toast.clone();
                let days = days.clone();
                move |result| match result {
                    Ok(policy) => {
                        days.set_value(policy.retain_read_days.unwrap_or(30) as f64);
                        days.connect_value_notify({
                            let controller = controller.clone();
                            let toast = toast.clone();
                            move |row| {
                                controller.save_storage_policy(
                                    brooklet::model::StoragePolicy {
                                        retain_read_days: Some(row.value() as u32),
                                        keep_at_most: policy.keep_at_most,
                                    },
                                    {
                                        let toast = toast.clone();
                                        move |result| {
                                            if let Err(error) = result {
                                                toast.add_toast(adw::Toast::new(
                                                    &error.sync_message(),
                                                ));
                                            }
                                        }
                                    },
                                );
                            }
                        });
                    }
                    Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                }
            });
            controller.karakeep_config({
                let direct = direct.clone();
                let endpoint = endpoint.clone();
                let toast = toast.clone();
                move |result| match result {
                    Ok(Some(config)) => {
                        direct.set_active(config.route == brooklet::model::KarakeepRoute::Direct);
                        endpoint.set_text(config.direct_endpoint.as_deref().unwrap_or(""));
                    }
                    Ok(None) => {}
                    Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                }
            });
            save_karakeep.connect_clicked({
                let controller = controller.clone();
                let direct = direct.clone();
                let endpoint = endpoint.clone();
                let key = key.clone();
                let toast = toast.clone();
                move |_| {
                    let route = if direct.is_active() {
                        brooklet::model::KarakeepRoute::Direct
                    } else {
                        brooklet::model::KarakeepRoute::Miniflux
                    };
                    let endpoint_value = endpoint.text().trim().to_owned();
                    let secret = key.text().trim().to_owned();
                    key.set_text("");
                    controller.save_karakeep_config(
                        brooklet::model::KarakeepConfig {
                            route,
                            direct_endpoint: (!endpoint_value.is_empty()).then_some(endpoint_value),
                        },
                        (!secret.is_empty()).then_some(secret),
                        {
                            let toast = toast.clone();
                            move |result| match result {
                                Ok(()) => {
                                    toast.add_toast(adw::Toast::new("Karakeep settings saved"))
                                }
                                Err(error) => {
                                    toast.add_toast(adw::Toast::new(&error.sync_message()))
                                }
                            }
                        },
                    );
                }
            });
            controller.sync_status(move |result| match result {
                Ok(info) => {
                    status.set_title(&format!(
                        "{} pending · Last sync {}",
                        info.queued_mutations,
                        info.last_successful_sync_at_ms.map_or_else(
                            || "never".into(),
                            |value| jiff::Timestamp::from_millisecond(value)
                                .map_or_else(|_| "unknown".into(), |time| time.to_string())
                        )
                    ));
                    status.set_subtitle(info.error.as_deref().unwrap_or("No sync error"));
                }
                Err(error) => status.set_title(&error.sync_message()),
            });
            dialog.present(Some(&window));
        }
    });
    window.add_action(&preferences);

    let shortcuts = gio::SimpleAction::new("show-shortcuts", None);
    shortcuts.connect_activate({
        let window = window.downgrade();
        move |_, _| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let builder =
                gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/shortcuts-dialog.ui");
            let dialog: adw::ShortcutsDialog = builder
                .object("shortcuts_dialog")
                .expect("shortcuts-dialog.ui must define shortcuts_dialog");
            dialog.present(Some(&window));
        }
    });
    window.add_action(&shortcuts);

    let about = gio::SimpleAction::new("about", None);
    about.connect_activate({
        let window = window.downgrade();
        move |_, _| {
            if let Some(window) = window.upgrade() {
                let dialog = adw::AboutDialog::new();
                dialog.set_application_name("Brooklet");
                dialog.set_version(env!("CARGO_PKG_VERSION"));
                dialog.set_developer_name("Nick Richards");
                dialog.set_website("https://github.com/nedrichards/brooklet-linux");
                dialog.set_license_type(gtk::License::Gpl30);
                dialog.present(Some(&window));
            }
        }
    });
    window.add_action(&about);
}

fn show_account(inbox_status: &adw::StatusPage, setup_button: &gtk::Button) {
    inbox_status.set_description(Some(
        "Your account is connected. Run a sync to fetch your Inbox.",
    ));
    setup_button.set_visible(false);
}

pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    adw::init()?;
    register_resources();
    let builder = gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/window.ui");
    let _: adw::ApplicationWindow = builder
        .object("window")
        .expect("window.ui must define the application window");
    let setup = gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/setup-dialog.ui");
    let _: adw::Dialog = setup
        .object("setup_dialog")
        .expect("setup-dialog.ui must define the setup dialog");
    let shortcuts = gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/shortcuts-dialog.ui");
    let _: adw::ShortcutsDialog = shortcuts
        .object("shortcuts_dialog")
        .expect("shortcuts-dialog.ui must define the shortcuts dialog");
    Ok(())
}

fn register_resources() {
    gio::resources_register_include!("brooklet.gresource")
        .expect("Brooklet GResources must be valid");
}
