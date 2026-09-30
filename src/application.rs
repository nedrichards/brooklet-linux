use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use adw::gio;
use adw::prelude::*;

use brooklet::{
    auto_refresh::AutoRefreshPolicy,
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
    emptied_place: Rc<RefCell<Option<ListPlace>>>,
    undo_toast: Rc<RefCell<Option<adw::Toast>>>,
    refresh_policy: Rc<RefCell<AutoRefreshPolicy>>,
}

#[derive(Clone)]
struct WeakInboxModel {
    store: adw::glib::WeakRef<gio::ListStore>,
    selection: adw::glib::WeakRef<gtk::SingleSelection>,
}

#[derive(Clone)]
struct WeakInboxUi {
    model: WeakInboxModel,
    list: adw::glib::WeakRef<gtk::ListView>,
    status: adw::glib::WeakRef<adw::StatusPage>,
    scroller: adw::glib::WeakRef<gtk::ScrolledWindow>,
    spinner: adw::glib::WeakRef<adw::Spinner>,
    new_button: adw::glib::WeakRef<gtk::Button>,
    pending_entries: Rc<RefCell<Option<Vec<Entry>>>>,
    read_in_flight: Rc<RefCell<HashSet<i64>>>,
    suppress_read: Rc<RefCell<HashSet<i64>>>,
    pinned_read: Rc<RefCell<Option<Entry>>>,
    rebuilding: Rc<Cell<bool>>,
    emptied_place: Rc<RefCell<Option<ListPlace>>>,
    undo_toast: Rc<RefCell<Option<adw::Toast>>>,
    refresh_policy: Rc<RefCell<AutoRefreshPolicy>>,
}
impl InboxUi {
    fn downgrade(&self) -> WeakInboxUi {
        WeakInboxUi {
            model: WeakInboxModel {
                store: self.model.store.downgrade(),
                selection: self.model.selection.downgrade(),
            },
            list: self.list.downgrade(),
            status: self.status.downgrade(),
            scroller: self.scroller.downgrade(),
            spinner: self.spinner.downgrade(),
            new_button: self.new_button.downgrade(),
            pending_entries: self.pending_entries.clone(),
            read_in_flight: self.read_in_flight.clone(),
            suppress_read: self.suppress_read.clone(),
            pinned_read: self.pinned_read.clone(),
            rebuilding: self.rebuilding.clone(),
            emptied_place: self.emptied_place.clone(),
            undo_toast: self.undo_toast.clone(),
            refresh_policy: self.refresh_policy.clone(),
        }
    }
}
impl WeakInboxUi {
    fn upgrade(&self) -> Option<InboxUi> {
        Some(InboxUi {
            model: ui::inbox::InboxModel {
                store: self.model.store.upgrade()?,
                selection: self.model.selection.upgrade()?,
            },
            list: self.list.upgrade()?,
            status: self.status.upgrade()?,
            scroller: self.scroller.upgrade()?,
            spinner: self.spinner.upgrade()?,
            new_button: self.new_button.upgrade()?,
            pending_entries: self.pending_entries.clone(),
            read_in_flight: self.read_in_flight.clone(),
            suppress_read: self.suppress_read.clone(),
            pinned_read: self.pinned_read.clone(),
            rebuilding: self.rebuilding.clone(),
            emptied_place: self.emptied_place.clone(),
            undo_toast: self.undo_toast.clone(),
            refresh_policy: self.refresh_policy.clone(),
        })
    }
}

#[derive(Clone)]
struct ListPlace {
    entry_id: Option<i64>,
    row_y: f64,
    value: f64,
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
    positions: Rc<RefCell<HashMap<i64, ReaderPosition>>>,
    open_generation: Rc<Cell<u64>>,
    image_requests: Rc<RefCell<Vec<tokio::task::AbortHandle>>>,
    images: Rc<RefCell<Option<Rc<ui::reader_images::Images>>>>,
    build_tick: Rc<RefCell<Option<gtk::TickCallbackId>>>,
    origin_set: Rc<RefCell<Vec<Arc<Entry>>>>,
    origin_index: Rc<Cell<usize>>,
    origin_inbox: Rc<Cell<bool>>,
    source_list: Rc<RefCell<Option<adw::glib::WeakRef<gtk::ListView>>>>,
}

#[derive(Clone)]
struct WeakReaderUi {
    split: adw::glib::WeakRef<adw::NavigationSplitView>,
    title: adw::glib::WeakRef<adw::WindowTitle>,
    placeholder: adw::glib::WeakRef<adw::StatusPage>,
    scroller: adw::glib::WeakRef<gtk::ScrolledWindow>,
    content: adw::glib::WeakRef<gtk::Box>,
    controller: Arc<AppController>,
    active_id: Rc<Cell<Option<i64>>>,
    restoring: Rc<Cell<bool>>,
    positions: Rc<RefCell<HashMap<i64, ReaderPosition>>>,
    open_generation: Rc<Cell<u64>>,
    image_requests: Rc<RefCell<Vec<tokio::task::AbortHandle>>>,
    images: Rc<RefCell<Option<Rc<ui::reader_images::Images>>>>,
    build_tick: Rc<RefCell<Option<gtk::TickCallbackId>>>,
    origin_set: Rc<RefCell<Vec<Arc<Entry>>>>,
    origin_index: Rc<Cell<usize>>,
    origin_inbox: Rc<Cell<bool>>,
    source_list: Rc<RefCell<Option<adw::glib::WeakRef<gtk::ListView>>>>,
}
impl ReaderUi {
    fn downgrade(&self) -> WeakReaderUi {
        WeakReaderUi {
            split: self.split.downgrade(),
            title: self.title.downgrade(),
            placeholder: self.placeholder.downgrade(),
            scroller: self.scroller.downgrade(),
            content: self.content.downgrade(),
            controller: self.controller.clone(),
            active_id: self.active_id.clone(),
            restoring: self.restoring.clone(),
            positions: self.positions.clone(),
            open_generation: self.open_generation.clone(),
            image_requests: self.image_requests.clone(),
            images: self.images.clone(),
            build_tick: self.build_tick.clone(),
            origin_set: self.origin_set.clone(),
            origin_index: self.origin_index.clone(),
            origin_inbox: self.origin_inbox.clone(),
            source_list: self.source_list.clone(),
        }
    }
}
impl WeakReaderUi {
    fn upgrade(&self) -> Option<ReaderUi> {
        Some(ReaderUi {
            split: self.split.upgrade()?,
            title: self.title.upgrade()?,
            placeholder: self.placeholder.upgrade()?,
            scroller: self.scroller.upgrade()?,
            content: self.content.upgrade()?,
            controller: self.controller.clone(),
            active_id: self.active_id.clone(),
            restoring: self.restoring.clone(),
            positions: self.positions.clone(),
            open_generation: self.open_generation.clone(),
            image_requests: self.image_requests.clone(),
            images: self.images.clone(),
            build_tick: self.build_tick.clone(),
            origin_set: self.origin_set.clone(),
            origin_index: self.origin_index.clone(),
            origin_inbox: self.origin_inbox.clone(),
            source_list: self.source_list.clone(),
        })
    }
}

#[derive(Clone)]
struct OtherViews {
    generation: Rc<Cell<u64>>,
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
    pending: Rc<RefCell<Option<adw::glib::SourceId>>>,
    request: Rc<RefCell<Option<tokio::task::AbortHandle>>>,
}

impl SearchState {
    fn refresh(&self) {
        if let Some(source) = self.pending.borrow_mut().take() {
            source.remove();
        }
        if let Some(task) = self.request.borrow_mut().take() {
            task.abort();
        }
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
        let pending = self.pending.clone();
        let request = self.request.clone();
        let source = adw::glib::timeout_add_local_once(Duration::from_millis(180), move || {
            pending.borrow_mut().take();
            if generation.get() != token {
                return;
            }
            let completed = request.clone();
            let task = controller.search_entries(text, feed, category, read, move |result| {
                if generation.get() != token {
                    return;
                }
                completed.borrow_mut().take();
                match result {
                    Ok(entries) => {
                        let empty = entries.is_empty();
                        ui::inbox::replace(&model, entries);
                        status.set_visible(empty);
                        scroller.set_visible(!empty);
                        scroller.vadjustment().set_value(0.0);
                    }
                    Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                }
            });
            *request.borrow_mut() = Some(task);
        });
        *self.pending.borrow_mut() = Some(source);
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
            reader_scroller.set_focusable(true);
            let reader_page: adw::NavigationPage = builder
                .object("reader_page")
                .expect("window.ui must define reader_page");
            let reader_content: gtk::Box = builder
                .object("reader_content")
                .expect("window.ui must define reader_content");
            let library_navigation: adw::NavigationView = builder
                .object("library_navigation")
                .expect("window.ui must define library_navigation");
            let saved_list: gtk::ListView = builder.object("saved_list").expect("saved_list");
            let open_id = Rc::new(Cell::new(None));
            let other_views = OtherViews {
                generation: Rc::new(Cell::new(0)),
                saved: ui::inbox::configure_with_action(&saved_list, false, open_id.clone()),
                saved_status: builder.object("saved_status").expect("saved_status"),
                saved_scroller: builder.object("saved_scroller").expect("saved_scroller"),
                library_all: ui::inbox::configure_with_action(
                    &builder
                        .object::<gtk::ListView>("library_all_list")
                        .expect("library_all_list"),
                    false,
                    open_id.clone(),
                ),
                library_unread: ui::inbox::configure_with_action(
                    &builder
                        .object::<gtk::ListView>("library_unread_list")
                        .expect("library_unread_list"),
                    false,
                    open_id.clone(),
                ),
                library_read: ui::inbox::configure_with_action(
                    &builder
                        .object::<gtk::ListView>("library_read_list")
                        .expect("library_read_list"),
                    false,
                    open_id.clone(),
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
            builder
                .object::<gtk::ListView>("library_unread_list")
                .expect("library_unread_list")
                .add_css_class("unread-only-list");
            builder
                .object::<gtk::ListView>("library_read_list")
                .expect("library_read_list")
                .add_css_class("read-only-list");
            let inbox_ui = InboxUi {
                model: ui::inbox::configure(&inbox_list, open_id.clone()),
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
                emptied_place: Rc::new(RefCell::new(None)),
                undo_toast: Rc::new(RefCell::new(None)),
                refresh_policy: Rc::new(RefCell::new(AutoRefreshPolicy::default())),
            };
            inbox_ui.model.selection.connect_selected_item_notify({
                let weak = inbox_ui.downgrade();
                move |_| {
                    let Some(inbox) = weak.upgrade() else {
                        return;
                    };
                    if !inbox.rebuilding.get() {
                        let weak = weak.clone();
                        adw::glib::idle_add_local_once(move || {
                            let Some(inbox) = weak.upgrade() else {
                                return;
                            };
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
                let weak = inbox_ui.downgrade();
                move |_| {
                    let Some(inbox) = weak.upgrade() else {
                        return;
                    };
                    let Some(entries) = inbox.pending_entries.borrow_mut().take() else {
                        return;
                    };
                    apply_inbox_entries(&inbox, entries);
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
                active_id: open_id,
                restoring: Rc::new(Cell::new(false)),
                positions: Rc::new(RefCell::new(HashMap::new())),
                open_generation: Rc::new(Cell::new(0)),
                image_requests: Rc::new(RefCell::new(Vec::new())),
                images: Rc::new(RefCell::new(None)),
                build_tick: Rc::new(RefCell::new(None)),
                origin_set: Rc::new(RefCell::new(Vec::new())),
                origin_index: Rc::new(Cell::new(0)),
                origin_inbox: Rc::new(Cell::new(false)),
                source_list: Rc::new(RefCell::new(None)),
            };
            install_reader_position_tracking(&reader_ui);
            let undo_entry = Rc::new(RefCell::new(Vec::<Entry>::new()));

            inbox_list.connect_activate({
                let model = inbox_ui.model.clone();
                let weak_reader = reader_ui.downgrade();
                let controller = controller.clone();
                let weak_inbox = inbox_ui.downgrade();
                let weak_toast = toast_overlay.downgrade();
                let undo_entry = undo_entry.clone();
                let current_entry = current_entry.clone();
                move |list, position| {
                    let (Some(reader_ui), Some(inbox_ui), Some(toast_overlay)) = (
                        weak_reader.upgrade(),
                        weak_inbox.upgrade(),
                        weak_toast.upgrade(),
                    ) else {
                        return;
                    };
                    if let Some(entry) = ui::inbox::entry_at(&model, position) {
                        ui::inbox::select_id(list, entry.id);
                        *current_entry.borrow_mut() = Some(entry.clone());
                        set_reader_origin(&reader_ui, &model, list, entry.id, true);
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
                                false,
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
                    let weak_reader = reader_ui.downgrade();
                    let current_entry = current_entry.clone();
                    let weak_inbox = inbox_ui.downgrade();
                    let weak_toast = toast_overlay.downgrade();
                    let undo_entry = undo_entry.clone();
                    move |list, position| {
                        let (Some(reader_ui), Some(inbox_ui), Some(toast_overlay)) = (
                            weak_reader.upgrade(),
                            weak_inbox.upgrade(),
                            weak_toast.upgrade(),
                        ) else {
                            return;
                        };
                        if let Some(entry) = ui::inbox::entry_at(&model, position) {
                            ui::inbox::select_id(list, entry.id);
                            *current_entry.borrow_mut() = Some(entry.clone());
                            set_reader_origin(&reader_ui, &model, list, entry.id, false);
                            open_article(&reader_ui, &inbox_ui, &entry);
                            if !entry.read {
                                mark_read_from(
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
                                    false,
                                    Some(list.clone()),
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
                            false,
                            true,
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
                                dismiss_undo_toast(&inbox);
                                *undo.borrow_mut() = entries;
                                *inbox.emptied_place.borrow_mut() =
                                    Some(capture_list_place(&inbox.list, &inbox.scroller));
                                inbox.pinned_read.borrow_mut().take();
                                inbox.rebuilding.set(true);
                                ui::inbox::replace(&inbox.model, Vec::new());
                                inbox.rebuilding.set(false);
                                update_inbox_visibility(&inbox);
                                let message = format!("Marked {count} articles read");
                                let notification = adw::Toast::new(&message);
                                notification.set_button_label(Some("Undo"));
                                notification.set_action_name(Some("win.undo"));
                                *inbox.undo_toast.borrow_mut() = Some(notification.clone());
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
            for navigation in [
                destinations.clone().upcast::<adw::glib::Object>(),
                library_navigation.clone().upcast::<adw::glib::Object>(),
            ] {
                let property = if navigation.is::<adw::ViewStack>() {
                    "visible-child"
                } else {
                    "visible-page"
                };
                navigation.connect_notify_local(Some(property), {
                    let controller = controller.clone();
                    let views = other_views.clone();
                    let toast = toast_overlay.downgrade();
                    move |_, _| {
                        let controller = controller.clone();
                        let views = views.clone();
                        let toast = toast.clone();
                        adw::glib::idle_add_local_once(move || {
                            if let Some(toast) = toast.upgrade() {
                                load_other_views(controller, views, toast);
                            }
                        });
                    }
                });
            }
            install_article_cursor_keys(
                &window,
                &destinations,
                &reader_page,
                &reader_ui.scroller,
                ReadContext {
                    controller: controller.clone(),
                    inbox: inbox_ui.clone(),
                    toast: toast_overlay.clone(),
                    undo: undo_entry.clone(),
                    reader: reader_ui.clone(),
                    current: current_entry.clone(),
                },
            );
            window.connect_close_request({
                let reader = reader_ui.clone();
                move |_| {
                    if let Some(entry_id) = reader.active_id.get()
                        && !reader.restoring.get()
                    {
                        let position = ui::reader::position_from_offset(
                            &reader.content,
                            entry_id,
                            reader.scroller.vadjustment().value() as i32,
                        );
                        reader.controller.save_reader_position(position, |_| {});
                    }
                    adw::glib::Propagation::Proceed
                }
            });
            window.connect_destroy({
                let reader = reader_ui.clone();
                let application = application.downgrade();
                let current = current_entry.clone();
                let undo = undo_entry.clone();
                move |_| {
                    reader
                        .open_generation
                        .set(reader.open_generation.get().wrapping_add(1));
                    for request in reader.image_requests.borrow_mut().drain(..) {
                        request.abort();
                    }
                    if let Some(tick) = reader.build_tick.borrow_mut().take() {
                        tick.remove();
                    }
                    reader.images.borrow_mut().take();
                    reader.origin_set.borrow_mut().clear();
                    reader.positions.borrow_mut().clear();
                    reader.active_id.set(None);
                    current.borrow_mut().take();
                    undo.borrow_mut().clear();
                    while let Some(child) = reader.content.first_child() {
                        reader.content.remove(&child);
                    }
                    if let Some(application) = application.upgrade() {
                        application.remove_action("sync");
                        application.remove_action("search");
                    }
                }
            });
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
                    let model =
                        ui::inbox::configure_with_action(&list, false, reader.active_id.clone());
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
                                replace_view_entries(&model, &status, &scroller, entries);
                            }
                            Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                        }
                    });
                    list.connect_activate({
                        let controller = controller.clone();
                        let reader = reader.downgrade();
                        let current = current.clone();
                        let inbox = inbox.downgrade();
                        let toast = toast.downgrade();
                        let undo = undo.clone();
                        move |list, position| {
                            let (Some(reader), Some(inbox), Some(toast)) =
                                (reader.upgrade(), inbox.upgrade(), toast.upgrade())
                            else {
                                return;
                            };
                            if let Some(entry) = ui::inbox::entry_at(&model, position) {
                                *current.borrow_mut() = Some(entry.clone());
                                set_reader_origin(&reader, &model, list, entry.id, false);
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
                let window = window.downgrade();
                let reader = reader_ui.clone();
                let reader_page = reader_page.clone();
                let destinations: adw::ViewStack =
                    builder.object("destinations").expect("destinations");
                let inbox = inbox_ui.list.clone();
                let navigation = library_navigation.clone();
                move |_, _| {
                    if let Some(window) = window.upgrade()
                        && let Some(dialog) = focused_dialog(window.upcast_ref())
                    {
                        dialog.close();
                        return;
                    }
                    let reader_focused = window.upgrade().is_some_and(|window| {
                        gtk::prelude::GtkWindowExt::focus(&window).is_some_and(|focus| {
                            focus == reader_page.clone().upcast::<gtk::Widget>()
                                || focus.is_ancestor(&reader_page)
                        })
                    });
                    if (reader.split.is_collapsed() && reader.split.shows_content())
                        || (reader_focused && reader.scroller.is_mapped())
                    {
                        if let Some(entry_id) = reader.active_id.get() {
                            return_to_article_list(&reader, &destinations, &inbox, entry_id);
                        } else if reader.split.is_collapsed() {
                            reader.split.set_show_content(false);
                        }
                        return;
                    }
                    navigation.pop();
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
                        false,
                    );
                }
            });
            application.add_action(&sync_action);
            let auto_refresh = AutoRefreshUi::install(
                &window,
                controller.clone(),
                inbox_ui.clone(),
                toast_overlay.clone(),
                sync_action.clone(),
                other_views.clone(),
            );

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
                            inbox_ui.refresh_policy.borrow_mut().account_ready = true;
                            show_account(&inbox_status, &setup_button);
                            toast_overlay.add_toast(adw::Toast::new("Miniflux account connected"));
                            begin_sync(
                                controller.clone(),
                                inbox_ui.clone(),
                                toast_overlay.clone(),
                                sync_action.clone(),
                                views.clone(),
                                false,
                            );
                        }
                    });
                }
            });
            window.add_action(&setup_action);

            window.present();
            focus_inbox_when_ready(&window, &inbox_ui.list, &inbox_ui.model.selection);

            controller.existing_account({
                let setup_action = setup_action.clone();
                let controller = controller.clone();
                let inbox_ui = inbox_ui.clone();
                let toast_overlay = toast_overlay.clone();
                let views = other_views.clone();
                let auto_refresh = auto_refresh.clone();
                move |result| match result {
                    Ok(Some(_account)) => {
                        show_account(&inbox_status, &setup_button);
                        controller.cached_inbox({
                            let controller = controller.clone();
                            let inbox_ui = inbox_ui.clone();
                            let toast_overlay = toast_overlay.clone();
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
                                controller.sync_status(move |result| {
                                    let mut policy = inbox_ui.refresh_policy.borrow_mut();
                                    if let Ok(status) = result {
                                        policy.restore_last_success(
                                            status.last_successful_sync_at_ms,
                                        );
                                    }
                                    policy.account_ready = true;
                                    drop(policy);
                                    auto_refresh.check();
                                });
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
        let shortcuts_dialog = Rc::new(RefCell::new(None::<adw::ShortcutsDialog>));
        let shortcuts = gio::SimpleAction::new("show-shortcuts", None);
        shortcuts.connect_activate({
            let application = self.application.downgrade();
            let shortcuts_dialog = shortcuts_dialog.clone();
            move |_, _| {
                let Some(application) = application.upgrade() else {
                    return;
                };
                let Some(window) = application.active_window() else {
                    return;
                };
                if shortcuts_dialog.borrow().is_some() {
                    return;
                }
                let builder =
                    gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/shortcuts-dialog.ui");
                let dialog: adw::ShortcutsDialog = builder
                    .object("shortcuts_dialog")
                    .expect("shortcuts-dialog.ui must define shortcuts_dialog");
                dialog.connect_closed({
                    let shortcuts_dialog = shortcuts_dialog.clone();
                    move |_| {
                        shortcuts_dialog.borrow_mut().take();
                    }
                });
                *shortcuts_dialog.borrow_mut() = Some(dialog.clone());
                let parent: gtk::Widget = focused_dialog(&window)
                    .map(|dialog| dialog.upcast())
                    .unwrap_or_else(|| window.clone().upcast());
                dialog.present(Some(&parent));
            }
        });
        self.application.add_action(&shortcuts);
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
        self.application.set_accels_for_action(
            "app.show-shortcuts",
            &[
                "F1",
                "<primary>question",
                "<primary><shift>slash",
                "<primary>slash",
            ],
        );
    }
}

fn install_reader_position_tracking(reader: &ReaderUi) {
    reader.scroller.vadjustment().connect_value_changed({
        let weak = reader.downgrade();
        let pending_save = Rc::new(RefCell::new(None::<adw::glib::SourceId>));
        move |adjustment| {
            let Some(reader) = weak.upgrade() else {
                return;
            };
            if reader.restoring.get() {
                return;
            }
            let Some(entry_id) = reader.active_id.get() else {
                return;
            };
            if let Some(source) = pending_save.borrow_mut().take() {
                source.remove();
            }
            let pending = pending_save.clone();
            let controller = reader.controller.clone();
            let content = reader.content.downgrade();
            let active_id = reader.active_id.clone();
            let offset = adjustment.value() as i32;
            let positions = reader.positions.clone();
            let generation = reader.open_generation.clone();
            let expected = generation.get();
            let source = adw::glib::timeout_add_local_once(Duration::from_millis(250), move || {
                pending.borrow_mut().take();
                if active_id.get() != Some(entry_id) || generation.get() != expected {
                    return;
                }
                let Some(content) = content.upgrade() else {
                    return;
                };
                let position = ui::reader::position_from_offset(&content, entry_id, offset);
                positions.borrow_mut().insert(entry_id, position.clone());
                controller.save_reader_position(position, |_| {});
            });
            *pending_save.borrow_mut() = Some(source);
        }
    });
}

fn opens_shortcuts(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    let modifiers = modifiers
        & (gtk::gdk::ModifierType::SHIFT_MASK
            | gtk::gdk::ModifierType::CONTROL_MASK
            | gtk::gdk::ModifierType::ALT_MASK
            | gtk::gdk::ModifierType::SUPER_MASK
            | gtk::gdk::ModifierType::HYPER_MASK
            | gtk::gdk::ModifierType::META_MASK);
    if key == gtk::gdk::Key::F1 && modifiers.is_empty() {
        return true;
    }
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers.intersects(
            gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK
                | gtk::gdk::ModifierType::HYPER_MASK
                | gtk::gdk::ModifierType::META_MASK,
        )
        && (key == gtk::gdk::Key::question || key == gtk::gdk::Key::slash)
}

enum ArticleKeyFocus {
    List(gtk::ListView),
    Reader,
    Other,
}

fn focus_inbox_when_ready(
    window: &adw::ApplicationWindow,
    list: &gtk::ListView,
    selection: &gtk::SingleSelection,
) {
    let initial_focus = gtk::prelude::GtkWindowExt::focus(window);
    let window = window.downgrade();
    let selection = selection.clone();
    let pending = Cell::new(true);
    list.connect_map(move |list| {
        if !pending.replace(false) {
            return;
        }
        let window = window.clone();
        let selection = selection.clone();
        let initial_focus = initial_focus.clone();
        // GtkListView needs its first layout frame before its rows can receive focus.
        list.add_tick_callback(move |list, _| {
            let Some(window) = window.upgrade() else {
                return adw::glib::ControlFlow::Break;
            };
            let focus = gtk::prelude::GtkWindowExt::focus(&window);
            // Do not steal focus from a control the user moved to during loading.
            // Hiding the initial setup control on account discovery can move
            // focus automatically; that must not prevent focusing the inbox.
            if initial_focus
                .as_ref()
                .is_some_and(|initial| initial.is_mapped())
                && focus.as_ref().is_some_and(|focus| {
                    Some(focus) != initial_focus.as_ref()
                        && focus != list.upcast_ref::<gtk::Widget>()
                        && !focus.is_ancestor(list)
                })
            {
                return adw::glib::ControlFlow::Break;
            }
            if selection.n_items() > 0 {
                if selection.selected() == gtk::INVALID_LIST_POSITION {
                    selection.set_selected(0);
                }
                list.grab_focus();
                list.scroll_to(selection.selected(), gtk::ListScrollFlags::FOCUS, None);
            }
            adw::glib::ControlFlow::Break
        });
    });
}

fn article_key_focus(
    window: &adw::ApplicationWindow,
    reader_page: &adw::NavigationPage,
    reader_scroller: &gtk::ScrolledWindow,
    destinations: &adw::ViewStack,
    cursor_key: bool,
) -> ArticleKeyFocus {
    let mut current = gtk::prelude::GtkWindowExt::focus(window);
    let no_focus = current.is_none();
    while let Some(widget) = current {
        if widget.is::<gtk::Editable>()
            || widget.is::<gtk::TextView>()
            || widget.is::<gtk::DropDown>()
            || widget.is::<gtk::Popover>()
        {
            return ArticleKeyFocus::Other;
        }
        if let Ok(list) = widget.clone().downcast::<gtk::ListView>() {
            return ArticleKeyFocus::List(list);
        }
        if widget == reader_page.clone().upcast::<gtk::Widget>() && reader_scroller.is_mapped() {
            return ArticleKeyFocus::Reader;
        }
        if widget.is::<gtk::ListBox>() || widget.is::<adw::Dialog>() {
            return ArticleKeyFocus::Other;
        }
        current = widget.parent();
    }
    // Navigation remains a window-level shortcut when header chrome has focus.
    // Editable controls, dialogs, and the reader were excluded above.
    if (no_focus || cursor_key)
        && let Some(list) = destinations
            .visible_child()
            .and_then(|child| mapped_article_list(&child))
    {
        return ArticleKeyFocus::List(list);
    }
    ArticleKeyFocus::Other
}

fn return_to_article_list(
    reader: &ReaderUi,
    destinations: &adw::ViewStack,
    inbox: &gtk::ListView,
    entry_id: i64,
) {
    if reader.split.is_collapsed() {
        reader.split.set_show_content(false);
    }
    let source = reader.source_list.borrow().clone();
    let destinations = destinations.downgrade();
    let inbox = inbox.downgrade();
    adw::glib::idle_add_local_once(move || {
        let destinations = destinations.upgrade();
        let list = source
            .and_then(|source| source.upgrade())
            .filter(|list| list.is_mapped())
            .or_else(|| {
                destinations
                    .as_ref()
                    .and_then(|destinations| destinations.visible_child())
                    .and_then(|child| mapped_article_list(&child))
            })
            .or_else(|| inbox.upgrade().filter(|list| list.is_mapped()));
        if let Some(list) = list {
            if ui::inbox::position_of_id(&list, entry_id).is_some() {
                ui::inbox::select_id(&list, entry_id);
            }
            list.grab_focus();
        } else if let Some(destinations) = destinations {
            destinations.child_focus(gtk::DirectionType::TabForward);
        }
    });
}

fn install_article_cursor_keys(
    window: &adw::ApplicationWindow,
    destinations: &adw::ViewStack,
    reader_page: &adw::NavigationPage,
    reader_scroller: &gtk::ScrolledWindow,
    read_context: ReadContext,
) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let window = window.downgrade();
        let destinations = destinations.downgrade();
        let reader_page = reader_page.downgrade();
        let reader_scroller = reader_scroller.downgrade();
        move |_, key, _, modifiers| {
            let (Some(window), Some(destinations), Some(reader_page), Some(reader_scroller)) = (
                window.upgrade(),
                destinations.upgrade(),
                reader_page.upgrade(),
                reader_scroller.upgrade(),
            ) else {
                return adw::glib::Propagation::Proceed;
            };
            if opens_shortcuts(key, modifiers) {
                return if gtk::prelude::WidgetExt::activate_action(
                    &window,
                    "app.show-shortcuts",
                    None,
                )
                .is_ok()
                {
                    adw::glib::Propagation::Stop
                } else {
                    adw::glib::Propagation::Proceed
                };
            }
            if modifiers.intersects(
                gtk::gdk::ModifierType::SHIFT_MASK
                    | gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::HYPER_MASK
                    | gtk::gdk::ModifierType::META_MASK,
            ) {
                return adw::glib::Propagation::Proceed;
            }
            let scope = article_key_focus(
                &window,
                &reader_page,
                &reader_scroller,
                &destinations,
                ui::inbox::cursor_direction(key).is_some(),
            );
            if key == gtk::gdk::Key::r {
                match &scope {
                    ArticleKeyFocus::List(list) => {
                        if let Some(entry) = ui::inbox::selected_from_list(list) {
                            if !entry.read
                                && (list == &read_context.inbox.list
                                    || list.has_css_class("unread-only-list"))
                                && !read_context
                                    .inbox
                                    .read_in_flight
                                    .borrow()
                                    .contains(&entry.id)
                            {
                                ui::inbox::select_next_unread(
                                    list,
                                    &read_context.inbox.read_in_flight.borrow(),
                                );
                            }
                            toggle_read(read_context.clone(), entry, Some(list.clone()), None);
                            return adw::glib::Propagation::Stop;
                        }
                    }
                    ArticleKeyFocus::Reader => {
                        if let Some(entry) = read_context.current.borrow().clone() {
                            let entry_id = entry.id;
                            let reader = read_context.reader.clone();
                            let destinations = destinations.clone();
                            let inbox = read_context.inbox.list.clone();
                            let returned = Rc::new(Cell::new(false));
                            let return_to_list = Rc::new(move || {
                                if !returned.replace(true) {
                                    return_to_article_list(
                                        &reader,
                                        &destinations,
                                        &inbox,
                                        entry_id,
                                    );
                                }
                            });
                            toggle_read(read_context.clone(), entry, None, Some(return_to_list));
                            return adw::glib::Propagation::Stop;
                        }
                    }
                    ArticleKeyFocus::Other => {}
                }
            }
            if key == gtk::gdk::Key::u
                && matches!(&scope, ArticleKeyFocus::List(_) | ArticleKeyFocus::Reader)
            {
                return if gtk::prelude::WidgetExt::activate_action(&window, "win.undo", None)
                    .is_ok()
                {
                    adw::glib::Propagation::Stop
                } else {
                    adw::glib::Propagation::Proceed
                };
            }
            if let ArticleKeyFocus::List(list) = &scope {
                if (key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter)
                    && !gtk::prelude::GtkWindowExt::focus(&window)
                        .is_some_and(|focus| focus.is::<gtk::Button>())
                    && let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>()
                    && selection.selected() != gtk::INVALID_LIST_POSITION
                {
                    list.emit_by_name::<()>("activate", &[&selection.selected()]);
                    return adw::glib::Propagation::Stop;
                }
                if let Some(direction) = ui::inbox::cursor_direction(key)
                    && ui::inbox::move_cursor(list, direction)
                {
                    return adw::glib::Propagation::Stop;
                }
            } else if matches!(&scope, ArticleKeyFocus::Reader)
                && let Some(direction) = ui::inbox::cursor_direction(key)
                && (key == gtk::gdk::Key::j || key == gtk::gdk::Key::k)
            {
                let adjustment = reader_scroller.vadjustment();
                let target = adjustment.value() + f64::from(direction) * 80.0;
                adjustment.set_value(target.clamp(
                    adjustment.lower(),
                    (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                ));
                return adw::glib::Propagation::Stop;
            }
            adw::glib::Propagation::Proceed
        }
    });
    window.add_controller(keys.clone());
    keys
}

fn focused_dialog(window: &gtk::Window) -> Option<adw::Dialog> {
    let mut current = gtk::prelude::GtkWindowExt::focus(window);
    while let Some(widget) = current {
        if let Ok(dialog) = widget.clone().downcast::<adw::Dialog>() {
            return Some(dialog);
        }
        current = widget.parent();
    }
    None
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

fn article_id_at(widget: &gtk::Widget) -> Option<i64> {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class("article-row") {
            return widget.widget_name().strip_prefix("article-")?.parse().ok();
        }
        current = widget.parent();
    }
    None
}

fn find_article_row(widget: &gtk::Widget, entry_id: i64) -> Option<gtk::Widget> {
    if widget.has_css_class("article-row") && widget.widget_name() == format!("article-{entry_id}")
    {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(row) = find_article_row(&widget, entry_id) {
            return Some(row);
        }
        child = widget.next_sibling();
    }
    None
}

fn capture_list_place(list: &gtk::ListView, scroller: &gtk::ScrolledWindow) -> ListPlace {
    // Anchor the article at the top of the viewport, so inserts above it do
    // not change what the reader is looking at.
    let picked = scroller.pick(24.0, 8.0, gtk::PickFlags::DEFAULT);
    let entry_id = picked.as_ref().and_then(article_id_at);
    let row_y = entry_id
        .and_then(|id| find_article_row(list.upcast_ref(), id))
        .and_then(|row| row.compute_point(scroller, &gtk::graphene::Point::new(0.0, 0.0)))
        .map_or(0.0, |point| f64::from(point.y()));
    ListPlace {
        entry_id,
        row_y,
        value: scroller.vadjustment().value(),
    }
}

fn restore_list_place(list: &gtk::ListView, scroller: &gtk::ScrolledWindow, place: ListPlace) {
    if let Some(position) = place
        .entry_id
        .and_then(|id| ui::inbox::position_of_id(list, id))
        && list.is_mapped()
    {
        list.scroll_to(position, gtk::ListScrollFlags::NONE, None);
    }
    let list = list.downgrade();
    let scroller = scroller.downgrade();
    let frames = Cell::new(0);
    let widget = scroller.upgrade().expect("scroller is still available");
    widget.add_tick_callback(move |_, _| {
        // GtkListView measures its recycled rows on the next frame.
        if frames.get() == 0 {
            frames.set(1);
            return adw::glib::ControlFlow::Continue;
        }
        let (Some(list), Some(scroller)) = (list.upgrade(), scroller.upgrade()) else {
            return adw::glib::ControlFlow::Break;
        };
        let adjustment = scroller.vadjustment();
        let target = place
            .entry_id
            .and_then(|id| find_article_row(list.upcast_ref(), id))
            .and_then(|row| row.compute_point(&scroller, &gtk::graphene::Point::new(0.0, 0.0)))
            .map_or(place.value, |point| {
                adjustment.value() + f64::from(point.y()) - place.row_y
            });
        adjustment.set_value(target.clamp(
            adjustment.lower(),
            (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
        ));
        adw::glib::ControlFlow::Break
    });
}

fn replace_view_entries(
    model: &ui::inbox::InboxModel,
    status: &adw::StatusPage,
    scroller: &gtk::ScrolledWindow,
    entries: Vec<Entry>,
) {
    let list = scroller.child().and_downcast::<gtk::ListView>();
    let place = list.as_ref().map(|list| capture_list_place(list, scroller));
    let empty = entries.is_empty();
    ui::inbox::replace(model, entries);
    status.set_visible(empty);
    scroller.set_visible(!empty);
    if let (Some(list), Some(place)) = (list, place)
        && !empty
    {
        restore_list_place(&list, scroller, place);
    }
}

/// A coarse timer runs only while the main window is active. Wall-time policy
/// catches suspension even when the compositor never changes window activation.
struct AutoRefreshUi {
    window: adw::glib::WeakRef<adw::ApplicationWindow>,
    controller: Arc<AppController>,
    inbox: InboxUi,
    toast: adw::ToastOverlay,
    action: gio::SimpleAction,
    views: OtherViews,
    network: gio::NetworkMonitor,
    timer: RefCell<Option<adw::glib::SourceId>>,
    stopped: Cell<bool>,
}

impl AutoRefreshUi {
    fn install(
        window: &adw::ApplicationWindow,
        controller: Arc<AppController>,
        inbox: InboxUi,
        toast: adw::ToastOverlay,
        action: gio::SimpleAction,
        views: OtherViews,
    ) -> Rc<Self> {
        let state = Rc::new(Self {
            window: window.downgrade(),
            controller,
            inbox,
            toast,
            action,
            views,
            network: gio::NetworkMonitor::default(),
            timer: RefCell::new(None),
            stopped: Cell::new(false),
        });
        window.connect_is_active_notify({
            let state = Rc::downgrade(&state);
            move |_| {
                if let Some(state) = state.upgrade() {
                    state.update_activity();
                }
            }
        });
        let network_signal = state.network.connect_notify_local(None, {
            let state = Rc::downgrade(&state);
            move |_, _| {
                if let Some(state) = state.upgrade() {
                    state.check();
                }
            }
        });
        window.connect_destroy({
            let state = state.clone();
            let signal = RefCell::new(Some(network_signal));
            move |_| {
                state.stopped.set(true);
                state.stop_timer();
                if let Some(signal) = signal.borrow_mut().take() {
                    state.network.disconnect(signal);
                }
            }
        });
        state.update_activity();
        state
    }

    fn stop_timer(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }

    fn update_activity(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        if !self
            .window
            .upgrade()
            .is_some_and(|window| window.is_active())
        {
            self.stop_timer();
            return;
        }
        self.check();
        if self.timer.borrow().is_none() {
            let weak = Rc::downgrade(self);
            let timer = adw::glib::timeout_add_seconds_local(60, move || {
                let Some(state) = weak.upgrade() else {
                    return adw::glib::ControlFlow::Break;
                };
                state.check();
                adw::glib::ControlFlow::Continue
            });
            *self.timer.borrow_mut() = Some(timer);
        }
    }

    fn check(&self) {
        if self.stopped.get() {
            return;
        }
        let active = self
            .window
            .upgrade()
            .is_some_and(|window| window.is_active());
        if self.inbox.refresh_policy.borrow().due(
            adw::glib::real_time() / 1000,
            active,
            self.network.is_network_available(),
            self.network.is_network_metered(),
            !self.action.is_enabled(),
        ) {
            begin_sync(
                self.controller.clone(),
                self.inbox.clone(),
                self.toast.clone(),
                self.action.clone(),
                self.views.clone(),
                true,
            );
        }
    }
}

fn begin_sync(
    controller: Arc<AppController>,
    inbox_ui: InboxUi,
    toast_overlay: adw::ToastOverlay,
    sync_action: gio::SimpleAction,
    views: OtherViews,
    automatic: bool,
) {
    if !sync_action.is_enabled() {
        return;
    }
    sync_action.set_enabled(false);
    inbox_ui.spinner.set_visible(true);
    let reload_controller = controller.clone();
    controller.sync(move |result| {
        inbox_ui
            .refresh_policy
            .borrow_mut()
            .finished(adw::glib::real_time() / 1000, result.is_ok());
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
                if automatic {
                    tracing::debug!("Automatic sync failed: {error}");
                } else {
                    toast_overlay.add_toast(adw::Toast::new(&error.sync_message()));
                }
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
                "Apply inbox updates"
            } else if new_count == 1 {
                "Apply 1 new article"
            } else {
                "Apply new articles"
            });
            if new_count > 1 {
                inbox_ui
                    .new_button
                    .set_label(&format!("Apply {new_count} new articles"));
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
    let place = capture_list_place(&inbox_ui.list, &inbox_ui.scroller);
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
    if inbox_ui.model.store.n_items() > 0 {
        inbox_ui.emptied_place.borrow_mut().take();
        restore_list_place(&inbox_ui.list, &inbox_ui.scroller, place);
    }
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
        let place = capture_list_place(&inbox_ui.list, &inbox_ui.scroller);
        ui::inbox::remove_entry(&inbox_ui.model, id);
        update_inbox_visibility(inbox_ui);
        if inbox_ui.model.store.n_items() > 0 {
            restore_list_place(&inbox_ui.list, &inbox_ui.scroller, place);
        } else {
            *inbox_ui.emptied_place.borrow_mut() = Some(place);
        }
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
            Arc::make_mut(entry).read = read;
        }
    }
}

fn update_source_read_state(list: Option<&gtk::ListView>, entry: &Entry, read: bool) {
    let Some(list) = list else {
        return;
    };
    let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>() else {
        return;
    };
    let Some(store) = selection.model().and_downcast::<gio::ListStore>() else {
        return;
    };
    let model = ui::inbox::InboxModel { store, selection };
    let selected_id = ui::inbox::selected_id(list);
    let leaves_view = (read && list.has_css_class("unread-only-list"))
        || (!read && list.has_css_class("read-only-list"));
    if leaves_view {
        let position = (selected_id == Some(entry.id))
            .then(|| ui::inbox::position_of_id(list, entry.id))
            .flatten();
        let scroller = list.parent().and_downcast::<gtk::ScrolledWindow>();
        let place = scroller
            .as_ref()
            .map(|scroller| capture_list_place(list, scroller));
        ui::inbox::remove_entry(&model, entry.id);
        let empty = model.store.n_items() == 0;
        if let Some(scroller) = scroller {
            if let Some(status) = scroller.prev_sibling().and_downcast::<adw::StatusPage>() {
                status.set_visible(empty);
            }
            scroller.set_visible(!empty);
            if let Some(place) = place
                && !empty
            {
                restore_list_place(list, &scroller, place);
            }
        }
        if let Some(position) = position
            && !empty
        {
            model
                .selection
                .set_selected(position.min(model.store.n_items() - 1));
        }
        return;
    }
    ui::inbox::update_entry(
        &model,
        Entry {
            read,
            ..entry.clone()
        },
    );
    if let Some(selected_id) = selected_id {
        ui::inbox::select_id(list, selected_id);
    }
}

fn toggle_read(
    context: ReadContext,
    entry: Entry,
    source: Option<gtk::ListView>,
    return_to_list: Option<Rc<dyn Fn()>>,
) {
    let read = context
        .current
        .borrow()
        .as_ref()
        .filter(|current| current.id == entry.id)
        .map(|current| current.read)
        .or_else(|| {
            context
                .reader
                .origin_set
                .borrow()
                .iter()
                .find(|origin| origin.id == entry.id)
                .map(|origin| origin.read)
        })
        .unwrap_or(entry.read);
    let entry = Entry { read, ..entry };
    let inbox_source = source
        .as_ref()
        .is_some_and(|list| *list == context.inbox.list);
    let source = if inbox_source { None } else { source };
    let read_in_flight = context.inbox.read_in_flight.borrow().contains(&entry.id);
    if entry.read || read_in_flight {
        mark_unread_from(context, entry, source, return_to_list);
    } else {
        mark_read_from(context, entry, false, true, source);
    }
}

fn dismiss_undo_toast(inbox: &InboxUi) {
    if let Some(notification) = inbox.undo_toast.borrow_mut().take() {
        notification.dismiss();
    }
}

fn restore_unread_row(inbox: &InboxUi, entry: Entry) {
    let place = if inbox.model.store.n_items() == 0 {
        inbox.emptied_place.borrow_mut().take()
    } else {
        None
    }
    .unwrap_or_else(|| capture_list_place(&inbox.list, &inbox.scroller));
    restore_unread_row_data(inbox, entry);
    restore_list_place(&inbox.list, &inbox.scroller, place);
}

fn restore_unread_row_data(inbox: &InboxUi, entry: Entry) {
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
    mark_unread_from(
        ReadContext {
            controller,
            inbox,
            toast,
            undo,
            reader,
            current,
        },
        entry,
        None,
        None,
    );
}

fn mark_unread_from(
    context: ReadContext,
    entry: Entry,
    source: Option<gtk::ListView>,
    return_to_list: Option<Rc<dyn Fn()>>,
) {
    let ReadContext {
        controller,
        inbox,
        toast,
        undo,
        reader,
        current,
    } = context;
    let entry_id = entry.id;
    if !entry.read && !inbox.read_in_flight.borrow().contains(&entry_id) {
        return;
    }
    if inbox.read_in_flight.borrow().contains(&entry_id) {
        inbox.suppress_read.borrow_mut().insert(entry_id);
        restore_unread_row(&inbox, entry.clone());
        if ui::inbox::selected_id(&inbox.list).is_none() && reader.active_id.get() == Some(entry_id)
        {
            ui::inbox::select_id(&inbox.list, entry_id);
        }
        update_reader_read_state(&reader, &current, entry_id, false);
        update_source_read_state(source.as_ref(), &entry, false);
        let removed_from_undo = {
            let mut pending = undo.borrow_mut();
            let before = pending.len();
            pending.retain(|item| item.id != entry_id);
            pending.len() != before
        };
        if removed_from_undo {
            dismiss_undo_toast(&inbox);
        }
        toast.add_toast(adw::Toast::new("Kept unread"));
        if let Some(return_to_list) = return_to_list {
            return_to_list();
        }
        return;
    }
    controller.set_read_local(entry_id, false, move |result| match result {
        Ok(()) => {
            restore_unread_row(&inbox, entry.clone());
            if ui::inbox::selected_id(&inbox.list).is_none()
                && reader.active_id.get() == Some(entry_id)
            {
                ui::inbox::select_id(&inbox.list, entry_id);
            }
            update_reader_read_state(&reader, &current, entry_id, false);
            update_source_read_state(source.as_ref(), &entry, false);
            let removed_from_undo = {
                let mut pending = undo.borrow_mut();
                let before = pending.len();
                pending.retain(|item| item.id != entry_id);
                pending.len() != before
            };
            if removed_from_undo {
                dismiss_undo_toast(&inbox);
            }
            toast.add_toast(adw::Toast::new("Kept unread"));
            if let Some(return_to_list) = return_to_list {
                return_to_list();
            }
        }
        Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
    });
}

fn mark_read(context: ReadContext, entry: Entry, retain_current: bool, show_toast: bool) {
    mark_read_from(context, entry, retain_current, show_toast, None);
}

fn mark_read_from(
    context: ReadContext,
    entry: Entry,
    retain_current: bool,
    show_toast: bool,
    source: Option<gtk::ListView>,
) {
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
                update_source_read_state(source.as_ref(), &entry, true);
                dismiss_undo_toast(&inbox);
                if retain_current
                    && ui::inbox::selected_id(&inbox.list) == Some(entry_id)
                    && pin_read(&inbox, entry.clone())
                {
                    *undo.borrow_mut() = vec![entry];
                } else if let Some(removed) = {
                    let place = capture_list_place(&inbox.list, &inbox.scroller);
                    let selected_position = (ui::inbox::selected_id(&inbox.list) == Some(entry_id))
                        .then(|| ui::inbox::position_of_id(&inbox.list, entry_id))
                        .flatten();
                    let removed = ui::inbox::remove_entry(&inbox.model, entry_id);
                    if removed.is_some() {
                        if inbox.model.store.n_items() > 0 {
                            if let Some(position) = selected_position {
                                inbox
                                    .model
                                    .selection
                                    .set_selected(position.min(inbox.model.store.n_items() - 1));
                            }
                            restore_list_place(&inbox.list, &inbox.scroller, place);
                        } else {
                            *inbox.emptied_place.borrow_mut() = Some(place);
                        }
                    }
                    removed
                } {
                    *undo.borrow_mut() = vec![removed];
                } else {
                    *undo.borrow_mut() = vec![entry];
                }
                update_inbox_visibility(&inbox);
                if show_toast {
                    let notification = adw::Toast::new("Marked as read");
                    notification.set_button_label(Some("Undo"));
                    notification.set_action_name(Some("win.undo"));
                    *inbox.undo_toast.borrow_mut() = Some(notification.clone());
                    toast.add_toast(notification);
                }
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
            dismiss_undo_toast(&inbox_ui);
            let place = if inbox_ui.model.store.n_items() == 0 {
                inbox_ui.emptied_place.borrow_mut().take()
            } else {
                None
            }
            .unwrap_or_else(|| capture_list_place(&inbox_ui.list, &inbox_ui.scroller));
            for entry in entries {
                let entry_id = entry.id;
                restore_unread_row_data(&inbox_ui, entry);
                update_reader_read_state(&reader, &current, entry_id, false);
            }
            if ui::inbox::selected_id(&inbox_ui.list).is_none()
                && let Some(active_id) = reader.active_id.get()
                && ui::inbox::entry_by_id(&inbox_ui.model, active_id).is_some()
            {
                ui::inbox::select_id(&inbox_ui.list, active_id);
            }
            restore_list_place(&inbox_ui.list, &inbox_ui.scroller, place);
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
    if let Some(list) = reader_ui
        .source_list
        .borrow()
        .as_ref()
        .and_then(|source| source.upgrade())
    {
        ui::inbox::refresh_open_marker(&list, Some(entry.id));
    }
    if reader_ui.active_id.get() == Some(entry.id) && reader_ui.scroller.is_visible() {
        reader_ui.split.set_show_content(true);
        reader_ui.scroller.grab_focus();
        return;
    }
    if let Some(previous_id) = reader_ui.active_id.get()
        && !reader_ui.restoring.get()
    {
        let position = ui::reader::position_from_offset(
            &reader_ui.content,
            previous_id,
            reader_ui.scroller.vadjustment().value() as i32,
        );
        reader_ui
            .positions
            .borrow_mut()
            .insert(previous_id, position.clone());
        reader_ui.controller.save_reader_position(position, |_| {});
    }
    let entry_id = entry.id;
    for request in reader_ui.image_requests.borrow_mut().drain(..) {
        request.abort();
    }
    reader_ui.restoring.set(true);
    let generation = reader_ui.open_generation.get().wrapping_add(1);
    reader_ui.open_generation.set(generation);
    reader_ui.active_id.set(Some(entry.id));
    reader_ui.images.borrow_mut().take();
    if let Some(tick) = reader_ui.build_tick.borrow_mut().take() {
        tick.remove();
    }
    ui::reader::begin(entry, &reader_ui.title, &reader_ui.content);
    reader_ui.scroller.vadjustment().set_value(0.0);
    reader_ui.placeholder.set_visible(false);
    reader_ui.scroller.set_visible(true);
    reader_ui.split.set_show_content(true);
    reader_ui.scroller.grab_focus();
    let weak = reader_ui.downgrade();
    let metadata = entry.clone();
    let request = reader_ui.controller.parse_entry(entry_id, move |result| {
        let Some(reader) = weak.upgrade() else {
            return;
        };
        if reader.open_generation.get() != generation {
            return;
        }
        reader.image_requests.borrow_mut().clear();
        let (blocks, stored_position) = match result {
            Ok(document) => document,
            Err(error) => {
                ui::reader::empty(&metadata, &reader.content, &error.sync_message());
                reader.restoring.set(false);
                return;
            }
        };
        let position = reader
            .positions
            .borrow()
            .get(&entry_id)
            .cloned()
            .or(stored_position);
        if blocks.is_empty() {
            ui::reader::empty(
                &metadata,
                &reader.content,
                "This article has no cached body.",
            );
        }
        let images = ui::reader_images::Images::new(
            reader.controller.clone(),
            &reader.content,
            &reader.scroller,
            reader.open_generation.clone(),
            entry_id,
            reader.restoring.clone(),
        );
        *reader.images.borrow_mut() = Some(images);
        let builder = RefCell::new(ui::reader::DocumentBuilder::new(blocks));
        let weak = reader.downgrade();
        let built = Cell::new(false);
        let tick = reader.content.add_tick_callback(move |content, _| {
            let Some(reader) = weak.upgrade() else {
                return adw::glib::ControlFlow::Break;
            };
            if reader.open_generation.get() != generation {
                return adw::glib::ControlFlow::Break;
            }
            if built.get() {
                // Wait for the last batch's layout before applying the saved anchor.
                // Respect a user who started scrolling while the document was built.
                if reader.scroller.vadjustment().value() == 0.0 {
                    let offset = position.as_ref().map_or(0, |position| {
                        ui::reader::offset_from_position(content, position)
                    });
                    reader.scroller.vadjustment().set_value(f64::from(offset));
                }
                reader.restoring.set(false);
                if let Some(images) = reader.images.borrow().as_ref() {
                    images.queue();
                }
                reader.build_tick.borrow_mut().take();
                return adw::glib::ControlFlow::Break;
            }
            let (slots, complete) = builder.borrow_mut().step(content);
            if let Some(images) = reader.images.borrow().as_ref() {
                images.add(slots);
            }
            built.set(complete);
            adw::glib::ControlFlow::Continue
        });
        *reader.build_tick.borrow_mut() = Some(tick);
    });
    reader_ui.image_requests.borrow_mut().push(request);
}

fn set_reader_origin(
    reader: &ReaderUi,
    model: &ui::inbox::InboxModel,
    list: &gtk::ListView,
    entry_id: i64,
    from_inbox: bool,
) {
    if let Some(previous) = reader
        .source_list
        .borrow()
        .as_ref()
        .and_then(|source| source.upgrade())
    {
        ui::inbox::refresh_open_marker(&previous, None);
    }
    *reader.source_list.borrow_mut() = Some(list.downgrade());
    let entries = (0..model.store.n_items())
        .filter_map(|position| {
            model
                .store
                .item(position)
                .and_downcast::<ui::entry_object::EntryObject>()
        })
        .map(|object| object.shared_entry())
        .collect::<Vec<_>>();
    set_reader_origin_entries(reader, entries, entry_id, from_inbox);
}

fn set_reader_origin_entries(
    reader: &ReaderUi,
    entries: Vec<Arc<Entry>>,
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

fn load_other_views(controller: Arc<AppController>, views: OtherViews, toast: adw::ToastOverlay) {
    let token = views.generation.get().wrapping_add(1);
    views.generation.set(token);
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
        // Empty-state scrollers are hidden; their parent still identifies the visible page.
        if !scroller.parent().is_some_and(|parent| parent.is_mapped()) {
            continue;
        }
        let toast = toast.clone();
        let generation = views.generation.clone();
        controller.entries_for_view(view.into(), move |result| {
            if generation.get() != token {
                return;
            }
            match result {
                Ok(entries) => {
                    replace_view_entries(&model, &status, &scroller, entries);
                }
                Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
            }
        });
    }
    if !views
        .categories_scroller
        .parent()
        .is_some_and(|parent| parent.is_mapped())
    {
        return;
    }
    controller.categories_cached({
        let list = views.categories.clone();
        let status = views.categories_status.clone();
        let scroller = views.categories_scroller.clone();
        let toast = toast.clone();
        let generation = views.generation.clone();
        move |result| {
            if generation.get() != token {
                return;
            }
            match result {
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
                let Some(entry) = reader
                    .origin_set
                    .borrow()
                    .get(next)
                    .map(|entry| entry.as_ref().clone())
                else {
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
                        false,
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
                            Arc::make_mut(origin).starred = desired;
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
        ("Keyboard Shortcuts", "app.show-shortcuts"),
        ("About Brooklet", "win.about"),
    ] {
        menu.append(Some(label), Some(action));
    }
    menu_button.set_menu_model(Some(&menu));

    let search_dialog = Rc::new(RefCell::new(None::<(adw::Dialog, gtk::SearchEntry)>));
    window.connect_destroy({
        let search_dialog = search_dialog.clone();
        move |_| {
            let open = search_dialog.borrow_mut().take();
            if let Some((dialog, _)) = open {
                dialog.close();
            }
        }
    });
    let search = gio::SimpleAction::new("search", None);
    search.connect_activate({
        let window = window.downgrade();
        let search_dialog = search_dialog.clone();
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
            if let Some((_, query)) = search_dialog.borrow().as_ref() {
                query.grab_focus();
                return;
            }
            let dialog = adw::Dialog::new();
            dialog.set_title("Search Library");
            dialog.set_content_width(620);
            dialog.set_content_height(600);
            let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let query = gtk::SearchEntry::new();
            dialog.connect_closed({
                let search_dialog = search_dialog.clone();
                move |_| {
                    search_dialog.borrow_mut().take();
                }
            });
            *search_dialog.borrow_mut() = Some((dialog.clone(), query.clone()));
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
            let model = ui::inbox::configure_with_action(&list, false, reader.active_id.clone());
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
                pending: Rc::new(RefCell::new(None)),
                request: Rc::new(RefCell::new(None)),
            };
            let signals = Rc::new(RefCell::new(Vec::<(
                adw::glib::WeakRef<adw::glib::Object>,
                adw::glib::SignalHandlerId,
            )>::new()));
            let signal = query.connect_search_changed({
                let state = state.clone();
                move |_| state.refresh()
            });
            signals.borrow_mut().push((
                query.clone().upcast::<adw::glib::Object>().downgrade(),
                signal,
            ));
            let signal = feed.connect_selected_notify({
                let state = state.clone();
                move |_| state.refresh()
            });
            signals.borrow_mut().push((
                feed.clone().upcast::<adw::glib::Object>().downgrade(),
                signal,
            ));
            let signal = category.connect_selected_notify({
                let state = state.clone();
                move |_| state.refresh()
            });
            signals.borrow_mut().push((
                category.clone().upcast::<adw::glib::Object>().downgrade(),
                signal,
            ));
            let signal = read.connect_selected_notify({
                let state = state.clone();
                let list = list.clone();
                move |dropdown| {
                    list.remove_css_class("unread-only-list");
                    list.remove_css_class("read-only-list");
                    match dropdown.selected() {
                        1 => list.add_css_class("unread-only-list"),
                        2 => list.add_css_class("read-only-list"),
                        _ => {}
                    }
                    state.refresh();
                }
            });
            signals.borrow_mut().push((
                read.clone().upcast::<adw::glib::Object>().downgrade(),
                signal,
            ));
            dialog.connect_closed({
                let signals = signals.clone();
                let generation = state.generation.clone();
                let pending = state.pending.clone();
                let request = state.request.clone();
                let store = model.store.clone();
                move |_| {
                    generation.set(generation.get().wrapping_add(1));
                    if let Some(source) = pending.borrow_mut().take() {
                        source.remove();
                    }
                    if let Some(task) = request.borrow_mut().take() {
                        task.abort();
                    }
                    for (object, signal) in signals.borrow_mut().drain(..) {
                        if let Some(object) = object.upgrade() {
                            object.disconnect(signal);
                        }
                    }
                    store.remove_all();
                }
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
                let dialog = dialog.downgrade();
                let controller = controller.clone();
                let reader = reader.downgrade();
                let current = current.clone();
                let inbox = inbox.downgrade();
                let toast = toast.downgrade();
                let undo = undo.clone();
                move |list, position| {
                    let (Some(dialog), Some(reader), Some(inbox), Some(toast)) = (
                        dialog.upgrade(),
                        reader.upgrade(),
                        inbox.upgrade(),
                        toast.upgrade(),
                    ) else {
                        return;
                    };
                    if let Some(entry) = ui::inbox::entry_at(&model, position) {
                        set_reader_origin(&reader, &model, list, entry.id, false);
                        dialog.close();
                        *current.borrow_mut() = Some(entry.clone());
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
            let signals = ui::signal_scope::SignalScope::for_dialog(&dialog);
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
            signals.track(&url, url.connect_activate({
                let button = button.clone();
                move |_| button.emit_clicked()
            }));
            signals.track(&url, url.connect_changed({
                let error = error.clone();
                move |_| error.set_visible(false)
            }));
            let submit_button = button.clone();
            let input_url = url.clone();
            signals.track(&button, button.connect_clicked({
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
            }));
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
            let signals = ui::signal_scope::SignalScope::for_dialog(&dialog);
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
            signals.track(&logout_button, logout_button.connect_clicked({
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
            }));
            page.add(&account_group);
            let storage = adw::PreferencesGroup::new();
            storage.set_title("Local cache");
            let days = adw::SpinRow::with_range(1.0, 3650.0, 1.0);
            days.set_title("Keep read articles for days");
            storage.add(&days);
            let image_cache = adw::ActionRow::new();
            image_cache.set_title("Article images");
            image_cache.set_subtitle("Up to 256 MiB, saved as you read for offline access");
            let clear_images = gtk::Button::with_label("Clear");
            clear_images.set_valign(gtk::Align::Center);
            clear_images.update_property(&[gtk::accessible::Property::Label("Clear cached article images")]);
            image_cache.add_suffix(&clear_images);
            image_cache.set_activatable_widget(Some(&clear_images));
            signals.track(&clear_images, clear_images.connect_clicked({
                let controller = controller.clone();
                let toast = toast.clone();
                move |button| {
                    button.set_sensitive(false);
                    controller.clear_image_cache({
                        let button = button.downgrade();
                        let toast = toast.clone();
                        move |result| {
                            if let Some(button) = button.upgrade() { button.set_sensitive(true); }
                            toast.add_toast(adw::Toast::new(if result.is_ok() { "Cached images cleared" } else { "Could not clear cached images" }));
                        }
                    });
                }
            }));
            storage.add(&image_cache);
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
            signals.track(&direct, direct.connect_active_notify({
                let endpoint = endpoint.clone();
                let key = key.clone();
                move |row| {
                    endpoint.set_sensitive(row.is_active());
                    key.set_sensitive(row.is_active());
                }
            }));
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
                let signals = signals.clone();
                let controller = controller.clone();
                let toast = toast.clone();
                let days = days.clone();
                move |result| match result {
                    Ok(policy) => {
                        days.set_value(policy.retain_read_days.unwrap_or(30) as f64);
                        signals.track(&days, days.connect_value_notify({
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
                        }));
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
            signals.track(&save_karakeep, save_karakeep.connect_clicked({
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
            }));
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

    let about = gio::SimpleAction::new("about", None);
    about.connect_activate({
        let window = window.downgrade();
        move |_, _| {
            if let Some(window) = window.upgrade() {
                let dialog = adw::AboutDialog::new();
                dialog.set_application_name("Brooklet");
                dialog.set_application_icon(config::APP_ID);
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

fn smoke_test_image_anchor() -> Result<(), adw::glib::BoolError> {
    let window = gtk::Window::builder()
        .default_width(400)
        .default_height(300)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let image = gtk::Box::new(gtk::Orientation::Vertical, 0);
    image.set_height_request(100);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_height_request(1200);
    content.append(&image);
    content.append(&body);
    let scroller = gtk::ScrolledWindow::builder().child(&content).build();
    window.set_child(Some(&scroller));
    window.present();
    let main_loop = adw::glib::MainLoop::new(None, false);
    let failed = Rc::new(Cell::new(false));
    let generation = Rc::new(Cell::new(1));
    content.add_tick_callback({
        let main_loop = main_loop.clone();
        let failed = failed.clone();
        let step = Cell::new(0);
        move |content, _| {
            let frame = step.get();
            step.set(frame + 1);
            match frame {
                1 => {
                    scroller.vadjustment().set_value(200.0);
                    let place = ui::reader::position_from_offset(content, 1, 200);
                    image.set_height_request(300);
                    ui::reader::preserve_after_image(
                        content,
                        &scroller,
                        generation.clone(),
                        1,
                        200.0,
                        place,
                    );
                }
                5 => {
                    if (scroller.vadjustment().value() - 400.0).abs() > 1.0 {
                        failed.set(true);
                    }
                    scroller.vadjustment().set_value(350.0);
                    let place = ui::reader::position_from_offset(content, 1, 350);
                    image.set_height_request(500);
                    ui::reader::preserve_after_image(
                        content,
                        &scroller,
                        generation.clone(),
                        1,
                        350.0,
                        place,
                    );
                    // User movement while an image loads takes precedence.
                    scroller.vadjustment().set_value(360.0);
                }
                9 => {
                    if (scroller.vadjustment().value() - 360.0).abs() > 1.0 {
                        failed.set(true);
                    }
                    main_loop.quit();
                    return adw::glib::ControlFlow::Break;
                }
                _ => {}
            }
            adw::glib::ControlFlow::Continue
        }
    });
    adw::glib::timeout_add_local_once(Duration::from_secs(3), {
        let main_loop = main_loop.clone();
        let failed = failed.clone();
        move || {
            failed.set(true);
            main_loop.quit();
        }
    });
    main_loop.run();
    window.close();
    if failed.get() {
        Err(adw::glib::bool_error!(
            "Late image changed the reader's scroll anchor"
        ))
    } else {
        Ok(())
    }
}

/// Exercise the installed window capture controller, not just cursor arithmetic.
fn smoke_test_article_keyboard(
    window: &adw::ApplicationWindow,
    builder: &gtk::Builder,
    inbox: &InboxUi,
    reader: &ReaderUi,
    entries: Vec<Entry>,
) -> Result<(), adw::glib::BoolError> {
    fn layout(window: &adw::ApplicationWindow) {
        let main_loop = adw::glib::MainLoop::new(None, false);
        window.add_tick_callback({
            let main_loop = main_loop.clone();
            let frames = Cell::new(0);
            move |_, _| {
                frames.set(frames.get() + 1);
                if frames.get() < 3 {
                    return adw::glib::ControlFlow::Continue;
                }
                main_loop.quit();
                adw::glib::ControlFlow::Break
            }
        });
        let expired = Rc::new(Cell::new(false));
        let deadline = adw::glib::timeout_add_local_once(Duration::from_secs(3), {
            let main_loop = main_loop.clone();
            let expired = expired.clone();
            move || {
                expired.set(true);
                main_loop.quit();
            }
        });
        main_loop.run();
        if !expired.get() {
            deadline.remove();
        }
    }
    let destinations: adw::ViewStack = builder.object("destinations").unwrap();
    let reader_page: adw::NavigationPage = builder.object("reader_page").unwrap();
    let keys = install_article_cursor_keys(
        window,
        &destinations,
        &reader_page,
        &reader.scroller,
        ReadContext {
            controller: reader.controller.clone(),
            reader: reader.clone(),
            inbox: inbox.clone(),
            current: Rc::new(RefCell::new(None)),
            toast: builder.object("toast_overlay").unwrap(),
            undo: Rc::new(RefCell::new(Vec::new())),
        },
    );
    if keys.propagation_phase() != gtk::PropagationPhase::Capture
        || keys.widget().as_ref() != Some(window.upcast_ref::<gtk::Widget>())
    {
        return Err(adw::glib::bool_error!("Article keys lost window capture"));
    }
    let activations = Rc::new(Cell::new(0));
    let activation_signal = inbox.list.connect_activate({
        let activations = activations.clone();
        move |_, _| activations.set(activations.get() + 1)
    });
    let undos = Rc::new(Cell::new(0));
    let undo = gio::SimpleAction::new("undo", None);
    undo.connect_activate({
        let undos = undos.clone();
        move |_, _| undos.set(undos.get() + 1)
    });
    window.add_action(&undo);
    // A separate action catches regressions to the previous reader-only binding.
    let keeps = Rc::new(Cell::new(0));
    let keep = gio::SimpleAction::new("keep-unread", None);
    keep.connect_activate({
        let keeps = keeps.clone();
        move |_, _| keeps.set(keeps.get() + 1)
    });
    window.add_action(&keep);
    // Match startup: setup is initially focused, then account discovery hides
    // it before the asynchronous cached inbox arrives and maps its rows.
    let setup: gtk::Button = builder.object("setup_button").unwrap();
    let header: gtk::Button = builder.object("sync_button").unwrap();
    // This isolated window has no app actions. Keep the controls enabled as
    // in the configured application so focus assertions cannot pass vacuously.
    setup.set_action_name(None);
    header.set_action_name(None);
    setup.set_sensitive(true);
    header.set_sensitive(true);
    window.present();
    layout(window);
    if !setup.grab_focus() {
        return Err(adw::glib::bool_error!(
            "Setup focus fixture was unavailable"
        ));
    }
    focus_inbox_when_ready(window, &inbox.list, &inbox.model.selection);
    show_account(&inbox.status, &setup);
    layout(window);
    show_entries(inbox, entries);
    layout(window);
    if !gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| {
        focus == inbox.list.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&inbox.list)
    }) || inbox.model.selection.selected() != 0
    {
        return Err(adw::glib::bool_error!(
            "Delayed inbox loading did not restore launch focus"
        ));
    }
    let press = |key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType| {
        keys.emit_by_name::<bool>("key-pressed", &[&key, &0_u32, &modifiers])
    };
    use gtk::gdk::{Key, ModifierType as Modifiers};
    if !press(Key::u, Modifiers::empty())
        || !press(Key::u, Modifiers::LOCK_MASK)
        || undos.get() != 2
        || press(Key::u, Modifiers::CONTROL_MASK)
        || press(Key::u, Modifiers::SHIFT_MASK)
        || inbox.model.selection.selected() != 0
    {
        return Err(adw::glib::bool_error!("Undo shortcut failed in the Inbox"));
    }
    for (key, modifiers, expected) in [
        (Key::Down, Modifiers::empty(), 1),
        (Key::Up, Modifiers::LOCK_MASK, 0),
        (Key::j, Modifiers::LOCK_MASK, 1),
        (Key::k, Modifiers::empty(), 0),
        (Key::Up, Modifiers::empty(), 0),
    ] {
        if !press(key, modifiers) || inbox.model.selection.selected() != expected {
            return Err(adw::glib::bool_error!(
                "Window article navigation failed for {key:?} with {modifiers:?}"
            ));
        }
    }
    if !header.grab_focus()
        || !gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| {
            focus == header.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&header)
        })
    {
        return Err(adw::glib::bool_error!(
            "Header focus fixture was unavailable"
        ));
    }
    if press(Key::u, Modifiers::empty()) || undos.get() != 2 {
        return Err(adw::glib::bool_error!("Undo intercepted header focus"));
    }
    if !press(Key::Down, Modifiers::LOCK_MASK) || inbox.model.selection.selected() != 1 {
        return Err(adw::glib::bool_error!(
            "Down did not transfer header focus to the article list"
        ));
    }
    if press(Key::Up, Modifiers::CONTROL_MASK) || inbox.model.selection.selected() != 1 {
        return Err(adw::glib::bool_error!("Modified arrows were intercepted"));
    }
    let dialog = adw::Dialog::new();
    let entry = gtk::Entry::new();
    dialog.set_child(Some(&entry));
    dialog.present(Some(window));
    layout(window);
    if !entry.grab_focus()
        || !gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| {
            focus == entry.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&entry)
        })
    {
        return Err(adw::glib::bool_error!(
            "Editing focus fixture was unavailable"
        ));
    }
    if press(Key::Up, Modifiers::empty())
        || press(Key::u, Modifiers::empty())
        || undos.get() != 2
        || inbox.model.selection.selected() != 1
    {
        return Err(adw::glib::bool_error!(
            "Article keys intercepted dialog editing"
        ));
    }
    dialog.force_close();
    layout(window);
    *reader.source_list.borrow_mut() = Some(inbox.list.downgrade());
    let selected = ui::inbox::selected_id(&inbox.list).unwrap();
    return_to_article_list(reader, &destinations, &inbox.list, selected);
    layout(window);
    if !gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| {
        focus == inbox.list.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&inbox.list)
    }) || !press(Key::Up, Modifiers::empty())
        || inbox.model.selection.selected() != 0
    {
        return Err(adw::glib::bool_error!(
            "Up failed after returning focus to the article list"
        ));
    }
    if activations.get() != 0
        || (0..inbox.model.store.n_items()).any(|position| {
            ui::inbox::entry_at(&inbox.model, position).is_some_and(|entry| entry.read)
        })
    {
        return Err(adw::glib::bool_error!(
            "Cursor navigation activated or marked an article read"
        ));
    }
    if !press(Key::Return, Modifiers::empty()) || activations.get() != 1 {
        return Err(adw::glib::bool_error!(
            "Enter did not deliberately activate the keyboard selection"
        ));
    }
    // Search and other collections use their own ListView. A list inside a
    // dialog is eligible, while the dialog's editable controls are not.
    let collection_selection =
        gtk::SingleSelection::new(Some(gtk::StringList::new(&["Other collection"])));
    let collection = gtk::ListView::new(
        Some(collection_selection),
        Some(gtk::SignalListItemFactory::new()),
    );
    let collection_dialog = adw::Dialog::new();
    collection_dialog.set_child(Some(&collection));
    collection_dialog.present(Some(window));
    layout(window);
    if !collection.grab_focus() || !press(Key::u, Modifiers::empty()) || undos.get() != 3 {
        return Err(adw::glib::bool_error!(
            "Undo failed in another article list"
        ));
    }
    collection_dialog.force_close();
    reader.placeholder.set_visible(false);
    reader.scroller.set_visible(true);
    reader.split.set_show_content(true);
    let reader_focus = gtk::Button::with_label("Reader focus fixture");
    reader.content.append(&reader_focus);
    layout(window);
    if !reader_focus.grab_focus()
        || !press(Key::u, Modifiers::empty())
        || undos.get() != 4
        || keeps.get() != 0
    {
        return Err(adw::glib::bool_error!("Undo failed in the reader"));
    }
    reader.content.remove(&reader_focus);
    window.remove_action("undo");
    window.remove_action("keep-unread");
    inbox.list.disconnect(activation_signal);
    window.remove_controller(&keys);
    Ok(())
}

fn smoke_test_reader_pipeline() -> Result<(), adw::glib::BoolError> {
    use brooklet::services::traits::Repository;
    let repository = Arc::new(
        SqliteRepository::open_in_memory().map_err(|error| adw::glib::bool_error!("{error}"))?,
    );
    let secrets = Arc::new(Oo7SecretStore::new("brooklet-smoke-test"));
    let cache_directory = adw::glib::tmp_dir().join(format!(
        "brooklet-smoke-{}",
        adw::glib::uuid_string_random()
    ));
    std::fs::create_dir(&cache_directory).map_err(|error| adw::glib::bool_error!("{error}"))?;
    let image_cache = brooklet::services::image_cache::ImageCache::new(
        cache_directory.join("images.db"),
        1024 * 1024,
    )
    .map_err(|error| adw::glib::bool_error!("{error}"))?;
    let controller = Arc::new(
        AppController::with_image_cache(
            Arc::new(AccountSetupService::new(
                Arc::new(MinifluxIdentityValidator),
                repository.clone(),
                secrets.clone(),
            )),
            Arc::new(AccountSyncService::new(
                repository.clone(),
                secrets,
                Arc::new(ReqwestMinifluxApiFactory),
            )),
            image_cache.clone(),
        )
        .map_err(|error| adw::glib::bool_error!("{error}"))?,
    );
    let png = gtk::gdk::MemoryTexture::new(
        32,
        32,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &adw::glib::Bytes::from_owned(vec![255_u8; 32 * 32 * 4]),
        32 * 4,
    )
    .save_to_png_bytes();
    controller
        .backend_handle()
        .block_on(async {
            for index in 0..40 {
                let png = png.as_ref().to_vec();
                image_cache
                    .get_or_fetch(
                        &format!("https://example.invalid/image/{index}"),
                        || async { Ok(png) },
                    )
                    .await?;
            }
            Ok::<(), BrookletError>(())
        })
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    let image_controller = controller.clone();
    let entry = |id, html: String| Entry {
        id,
        account_id: 1,
        feed_id: 1,
        feed_title: "Synthetic feed".into(),
        category_title: String::new(),
        title: format!("Synthetic article {id}"),
        url: "https://example.invalid/article".into(),
        author: None,
        published_at_ms: 0,
        html,
        content_revision: 0,
        read: false,
        starred: false,
        reading_minutes: 1,
        delivery_state: None,
        delivery_error: None,
    };
    let long = entry(
        1,
        format!(
            "{}<table>{}</table>",
            "<p>Long paragraph</p>".repeat(160),
            "<tr><th>Heading</th><td>Cell</td></tr>".repeat(40)
        ),
    );
    let replacement = entry(2, "<p>Replacement body</p>".into());
    controller
        .backend_handle()
        .block_on(async {
            repository
                .save_account(&brooklet::model::Account {
                    id: 1,
                    server_url: "https://example.invalid".into(),
                    username: "synthetic".into(),
                    server_version: "2.3.2".into(),
                })
                .await?;
            repository
                .replace_unread_snapshot(1, &[long.clone(), replacement.clone()])
                .await?;
            repository
                .save_reader_position(
                    1,
                    &ReaderPosition {
                        entry_id: 1,
                        first_visible_block: 80,
                        offset_px: 2,
                    },
                )
                .await
        })
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    let builder = gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/window.ui");
    let window: adw::ApplicationWindow = builder.object("window").unwrap();
    let active_id = Rc::new(Cell::new(None));
    let list: gtk::ListView = builder.object("inbox_list").unwrap();
    let inbox = InboxUi {
        model: ui::inbox::configure(&list, active_id.clone()),
        list,
        status: builder.object("inbox_status").unwrap(),
        scroller: builder.object("inbox_scroller").unwrap(),
        spinner: builder.object("sync_spinner").unwrap(),
        new_button: builder.object("new_articles_button").unwrap(),
        pending_entries: Rc::new(RefCell::new(None)),
        read_in_flight: Rc::new(RefCell::new(HashSet::new())),
        suppress_read: Rc::new(RefCell::new(HashSet::new())),
        pinned_read: Rc::new(RefCell::new(None)),
        rebuilding: Rc::new(Cell::new(false)),
        emptied_place: Rc::new(RefCell::new(None)),
        undo_toast: Rc::new(RefCell::new(None)),
        refresh_policy: Rc::new(RefCell::new(AutoRefreshPolicy::default())),
    };
    let reader = ReaderUi {
        split: builder.object("inbox_split").unwrap(),
        title: builder.object("reader_title").unwrap(),
        placeholder: builder.object("reader_placeholder").unwrap(),
        scroller: builder.object("reader_scroller").unwrap(),
        content: builder.object("reader_content").unwrap(),
        controller,
        active_id,
        restoring: Rc::new(Cell::new(false)),
        positions: Rc::new(RefCell::new(HashMap::new())),
        open_generation: Rc::new(Cell::new(0)),
        image_requests: Rc::new(RefCell::new(Vec::new())),
        images: Rc::new(RefCell::new(None)),
        build_tick: Rc::new(RefCell::new(None)),
        origin_set: Rc::new(RefCell::new(Vec::new())),
        origin_index: Rc::new(Cell::new(0)),
        origin_inbox: Rc::new(Cell::new(false)),
        source_list: Rc::new(RefCell::new(None)),
    };
    install_reader_position_tracking(&reader);
    smoke_test_article_keyboard(
        &window,
        &builder,
        &inbox,
        &reader,
        vec![long.clone(), replacement.clone()],
    )?;
    window.present();
    open_article(&reader, &inbox, &long);
    open_article(&reader, &inbox, &replacement);
    let main_loop = adw::glib::MainLoop::new(None, false);
    let failure = Rc::new(RefCell::new(Some("Background reader timed out".to_owned())));
    window.add_tick_callback({
        let main_loop = main_loop.clone();
        let failure = failure.clone();
        let reader = reader.downgrade();
        let inbox = inbox.downgrade();
        let stage = Cell::new(0);
        let construction_frames = Cell::new(0);
        move |_, _| {
            let (Some(reader), Some(inbox)) = (reader.upgrade(), inbox.upgrade()) else {
                return adw::glib::ControlFlow::Break;
            };
            if reader.restoring.get() {
                if stage.get() == 1 {
                    construction_frames.set(construction_frames.get() + 1);
                }
                return adw::glib::ControlFlow::Continue;
            }
            if stage.get() == 0 {
                let labels = reader
                    .content
                    .observe_children()
                    .iter::<adw::glib::Object>()
                    .filter_map(Result::ok)
                    .filter_map(|object| object.downcast::<gtk::Widget>().ok())
                    .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                    .map(|label| label.text().to_string())
                    .collect::<Vec<_>>();
                if !labels.iter().any(|text| text == "Replacement body")
                    || labels.iter().any(|text| text == "Long paragraph")
                {
                    *failure.borrow_mut() = Some("A stale article replaced the active body".into());
                    main_loop.quit();
                    return adw::glib::ControlFlow::Break;
                }
                stage.set(1);
                open_article(&reader, &inbox, &long);
                return adw::glib::ControlFlow::Continue;
            }
            if construction_frames.get() < 2 || reader.scroller.vadjustment().value() <= 0.0 {
                *failure.borrow_mut() =
                    Some("Reader construction or saved anchor restoration failed".into());
            } else {
                failure.borrow_mut().take();
            }
            main_loop.quit();
            adw::glib::ControlFlow::Break
        }
    });
    let deadline = adw::glib::timeout_add_local_once(Duration::from_secs(5), {
        let main_loop = main_loop.clone();
        move || main_loop.quit()
    });
    main_loop.run();
    if failure.borrow().is_none() {
        deadline.remove();
    }
    let weak = reader.downgrade();
    // Keep the adjustment alive to prove its signal doesn't keep the reader alive.
    let adjustment = reader.scroller.vadjustment();
    reader.images.borrow_mut().take();
    if let Some(tick) = reader.build_tick.borrow_mut().take() {
        tick.remove();
    }
    window.destroy();
    drop(reader);
    drop(inbox);
    drop(window);
    drop(builder);
    if weak.content.upgrade().is_some() || weak.scroller.upgrade().is_some() {
        return Err(adw::glib::bool_error!(
            "Adjustment signal retained the reader"
        ));
    }
    drop(adjustment);
    if let Some(error) = failure.borrow_mut().take() {
        return Err(adw::glib::bool_error!("{error}"));
    }
    let result = ui::reader_images::smoke_test(image_controller);
    let _ = std::fs::remove_dir_all(cache_directory);
    result
}

pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    adw::init()?;
    let texture = gtk::gdk::MemoryTexture::new(
        320,
        160,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &adw::glib::Bytes::from_owned(vec![255_u8; 320 * 160 * 4]),
        320 * 4,
    );
    let picture = ui::reader::picture(&texture.upcast(), "Image layout test");
    for (width, expected_height) in [(160, 80), (320, 160)] {
        let (minimum, natural, _, _) = picture.measure(gtk::Orientation::Vertical, width);
        if minimum != expected_height || natural != expected_height {
            return Err(adw::glib::bool_error!(
                "Article image lost its proportional height"
            ));
        }
    }
    register_resources();
    let builder = gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/window.ui");
    let window: adw::ApplicationWindow = builder
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
    let list: gtk::ListView = builder.object("inbox_list").expect("inbox_list");
    let selection = gtk::SingleSelection::new(Some(gtk::StringList::new(&["Launch focus test"])));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        item.downcast_ref::<gtk::ListItem>()
            .expect("list item")
            .set_child(Some(&gtk::Label::new(Some("Launch focus test"))));
    });
    list.set_model(Some(&selection));
    list.set_factory(Some(&factory));
    window.present();
    focus_inbox_when_ready(&window, &list, &selection);
    builder
        .object::<adw::StatusPage>("inbox_status")
        .expect("inbox_status")
        .set_visible(false);
    builder
        .object::<gtk::ScrolledWindow>("inbox_scroller")
        .expect("inbox_scroller")
        .set_visible(true);
    let main_loop = adw::glib::MainLoop::new(None, false);
    list.add_tick_callback({
        let main_loop = main_loop.clone();
        let frames = Cell::new(0);
        move |_, _| {
            if frames.replace(1) == 0 {
                return adw::glib::ControlFlow::Continue;
            }
            main_loop.quit();
            adw::glib::ControlFlow::Break
        }
    });
    adw::glib::timeout_add_local_once(Duration::from_secs(3), {
        let main_loop = main_loop.clone();
        move || main_loop.quit()
    });
    main_loop.run();
    let focused = gtk::prelude::GtkWindowExt::focus(&window).is_some_and(|focus| {
        focus == list.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&list)
    });
    window.close();
    if !focused || selection.selected() != 0 {
        return Err(adw::glib::bool_error!("Inbox did not receive launch focus"));
    }
    smoke_test_image_anchor()?;
    ui::signal_scope::smoke_test()?;
    smoke_test_reader_pipeline()
}

fn register_resources() {
    gio::resources_register_include!("brooklet.gresource")
        .expect("Brooklet GResources must be valid");
}

#[cfg(test)]
mod shortcut_tests {
    use super::opens_shortcuts;
    use gtk::gdk::{Key, ModifierType as Modifiers};

    #[test]
    fn help_accepts_question_and_slash_with_lock_modifiers() {
        for key in [Key::question, Key::slash] {
            for extra in [
                Modifiers::empty(),
                Modifiers::SHIFT_MASK,
                Modifiers::LOCK_MASK,
            ] {
                assert!(opens_shortcuts(key, Modifiers::CONTROL_MASK | extra));
            }
            assert!(!opens_shortcuts(key, Modifiers::empty()));
            assert!(!opens_shortcuts(
                key,
                Modifiers::CONTROL_MASK | Modifiers::ALT_MASK
            ));
        }
        assert!(opens_shortcuts(Key::F1, Modifiers::LOCK_MASK));
        assert!(!opens_shortcuts(Key::F1, Modifiers::CONTROL_MASK));
    }
}
