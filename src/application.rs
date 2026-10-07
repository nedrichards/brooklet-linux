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

use crate::{keyboard, ui};

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
    refresh_generation: Rc<Cell<u64>>,
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
    refresh_generation: Rc<Cell<u64>>,
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
            refresh_generation: self.refresh_generation.clone(),
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
            refresh_generation: self.refresh_generation.clone(),
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
    actions: gio::SimpleActionGroup,
    menu: gtk::MenuButton,
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
    actions: gio::SimpleActionGroup,
    menu: adw::glib::WeakRef<gtk::MenuButton>,
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
            actions: self.actions.clone(),
            menu: self.menu.downgrade(),
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
            actions: self.actions.clone(),
            menu: self.menu.upgrade()?,
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
    drill_down: Rc<ui::library::LibraryPages>,
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
    scope: gtk::DropDown,
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
        let read = match (self.scope.selected(), self.read.selected()) {
            (1, _) | (_, 1) => Some(false),
            (_, 2) => Some(true),
            _ => None,
        };
        let scope = match self.scope.selected() {
            1 => "inbox",
            2 => "saved",
            _ => "all",
        }
        .to_string();
        self.read.set_sensitive(scope != "inbox");
        self.status.set_title("Searching cached articles…");
        self.status.set_description(None);
        // Keep the mapped list alive while the next query runs. Hiding it
        // discards its layout and flashes the loading page on every keystroke.
        if self.model.store.n_items() == 0 {
            self.status.set_visible(true);
            self.scroller.set_visible(false);
        }
        let text = self.query.text().to_string();
        let model = self.model.clone();
        let generation = self.generation.clone();
        let toast = self.toast.clone();
        let status = self.status.clone();
        let scroller = self.scroller.clone();
        let controller = self.controller.clone();
        let pending = self.pending.clone();
        let request = self.request.clone();
        let source = adw::glib::timeout_add_local_once(Duration::from_millis(60), move || {
            pending.borrow_mut().take();
            if generation.get() != token {
                return;
            }
            let completed = request.clone();
            let task =
                controller.search_entries(text, feed, category, read, scope, move |result| {
                    if generation.get() != token {
                        return;
                    }
                    completed.borrow_mut().take();
                    match result {
                        Ok(mut entries) => {
                            entries.truncate(500);
                            let empty = entries.is_empty();
                            ui::inbox::replace(&model, entries);
                            status.set_title("No matching articles");
                            status.set_description(Some(
                                "Try a different search or clear the filters.",
                            ));
                            status.set_visible(empty);
                            scroller.set_visible(!empty);
                            scroller.vadjustment().set_value(0.0);
                        }
                        Err(error) => {
                            status.set_title("Unable to search");
                            status.set_description(Some(&error.sync_message()));
                            status.set_visible(true);
                            scroller.set_visible(false);
                            toast.add_toast(adw::Toast::new(&error.sync_message()));
                        }
                    }
                });
            *request.borrow_mut() = Some(task);
        });
        *self.pending.borrow_mut() = Some(source);
    }
}

impl BrookletApplication {
    pub fn run() -> adw::glib::ExitCode {
        adw::glib::set_application_name(config::APP_NAME);
        register_resources();
        let application = adw::Application::builder()
            .application_id(config::APP_ID)
            .build();
        let database_path = adw::glib::user_data_dir()
            .join(config::APP_ID)
            .join("brooklet.db");
        let image_path = adw::glib::user_cache_dir()
            .join(config::APP_ID)
            .join("images.db");
        Self::install_startup(&application, database_path, image_path);
        application.run()
    }

    fn install_startup(
        application: &adw::Application,
        database_path: std::path::PathBuf,
        image_path: std::path::PathBuf,
    ) {
        let weak_application = application.downgrade();
        ui::startup::install(
            application,
            database_path.clone(),
            Arc::new(move || {
                let repository = Arc::new(SqliteRepository::open(&database_path)?);
                let images = brooklet::services::image_cache::ImageCache::new(
                    image_path.clone(),
                    brooklet::services::image_cache::DEFAULT_IMAGE_CACHE_BYTES,
                )?;
                Ok((repository, images))
            }),
            Rc::new(move |(repository, images)| {
                let Some(application) = weak_application.upgrade() else {
                    return Ok(());
                };
                let secrets = Arc::new(Oo7SecretStore::new(config::APP_ID));
                let setup = Arc::new(AccountSetupService::new(
                    Arc::new(MinifluxIdentityValidator),
                    repository.clone(),
                    secrets.clone(),
                ));
                let sync = Arc::new(AccountSyncService::new(
                    repository,
                    secrets,
                    Arc::new(ReqwestMinifluxApiFactory),
                ));
                let controller = Arc::new(AppController::with_image_cache(setup, sync, images)?);
                let this = Self {
                    application,
                    controller,
                };
                this.install_actions();
                this.connect_lifecycle();
                Ok(())
            }),
        );
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
            let health_host: gtk::Box = builder
                .object("sync_health_host")
                .expect("sync_health_host");
            ui::sync_health::install(&window, &health_host, controller.clone());
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
                drill_down: ui::library::LibraryPages::new(
                    &library_navigation,
                    controller.clone(),
                    &toast_overlay,
                ),
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
                refresh_generation: Rc::new(Cell::new(0)),
                read_in_flight: Rc::new(RefCell::new(HashSet::new())),
                suppress_read: Rc::new(RefCell::new(HashSet::new())),
                pinned_read: Rc::new(RefCell::new(None)),
                rebuilding: Rc::new(Cell::new(false)),
                emptied_place: Rc::new(RefCell::new(None)),
                undo_toast: Rc::new(RefCell::new(None)),
                refresh_policy: Rc::new(RefCell::new(AutoRefreshPolicy::default())),
            };
            install_read_pin_tracking(&inbox_ui);
            let reader_ui = ReaderUi {
                split: inbox_split,
                title: reader_title,
                actions: gio::SimpleActionGroup::new(),
                menu: builder.object("reader_menu").unwrap(),
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
            let tools = WindowTools {
                controller: controller.clone(),
                reader: reader_ui.clone(),
                current: current_entry.clone(),
                inbox: inbox_ui.clone(),
                views: other_views.clone(),
                toast: toast_overlay.clone(),
                undo: undo_entry.clone(),
            };
            install_reader_actions(&window, &builder, tools.clone());
            install_reader_controls(&window, &reader_ui);
            install_window_tools(&window, application, &builder, tools);
            let destinations: adw::ViewStack =
                builder.object("destinations").expect("destinations");
            let switcher: adw::ViewSwitcherBar = builder
                .object("destination_switcher")
                .expect("destination_switcher");
            install_inbox_top_shortcut(&switcher, &inbox_ui.scroller);
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
                let controller = controller.clone();
                let application = application.downgrade();
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
                    if let Some(application) = application.upgrade() {
                        let hold = application.hold();
                        controller.drain_local_writes(move |_| drop(hold));
                    }
                    adw::glib::Propagation::Proceed
                }
            });
            window.connect_destroy({
                let pages = other_views.drill_down.clone();
                let reader = reader_ui.clone();
                let application = application.downgrade();
                let current = current_entry.clone();
                let undo = undo_entry.clone();
                move |_| {
                    pages.clear();
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
                let pages = other_views.drill_down.clone();
                move |_, value| {
                    if let Some(id) = value.and_then(|value| value.get::<i64>()) {
                        pages.open_category(id);
                    }
                }
            });
            window.add_action(&category_action);
            let feed_action =
                gio::SimpleAction::new("library-feed", Some(&i64::static_variant_type()));
            feed_action.connect_activate({
                let controller = controller.clone();
                let pages = other_views.drill_down.clone();
                let reader = reader_ui.clone();
                let current = current_entry.clone();
                let inbox = inbox_ui.clone();
                let toast = toast_overlay.clone();
                let undo = undo_entry.clone();
                move |_, value| {
                    let Some(feed_id) = value.and_then(|value| value.get::<i64>()) else {
                        return;
                    };
                    let (list, model) = pages.open_feed(feed_id, reader.active_id.clone());
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
                    if let Some(window) = window.upgrade() {
                        navigate_back(
                            &window,
                            &reader,
                            &reader_page,
                            &destinations,
                            &inbox,
                            &navigation,
                        );
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
                                    Ok(entries) => refresh_inbox(&controller, &inbox_ui, entries),
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
        let controller = self.controller.clone();
        let quit = gio::ActionEntry::builder("quit")
            .activate(move |application: &adw::Application, _, _| {
                let application = application.clone();
                let hold = application.hold();
                controller.drain_local_writes(move |_| {
                    application.quit();
                    drop(hold);
                });
            })
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
                keyboard::populate_dialog(&dialog);
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
        for binding in keyboard::BINDINGS {
            if let keyboard::Command::Action(action) = binding.command {
                self.application.set_accels_for_action(action, binding.keys);
            }
        }
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
        && (key == gtk::gdk::Key::question
            || (key == gtk::gdk::Key::slash
                && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)))
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
            return if list.has_css_class("article-list") {
                ArticleKeyFocus::List(list)
            } else {
                ArticleKeyFocus::Other
            };
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

fn navigate_back(
    window: &adw::ApplicationWindow,
    reader: &ReaderUi,
    reader_page: &adw::NavigationPage,
    destinations: &adw::ViewStack,
    inbox: &gtk::ListView,
    navigation: &adw::NavigationView,
) {
    if let Some(dialog) = focused_dialog(window.upcast_ref()) {
        dialog.close();
        return;
    }
    let reader_focused = gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| {
        focus == reader_page.clone().upcast::<gtk::Widget>() || focus.is_ancestor(reader_page)
    });
    if (reader.split.is_collapsed() && reader.split.shows_content())
        || (reader_focused && reader.scroller.is_mapped())
    {
        if let Some(entry_id) = reader.active_id.get() {
            return_to_article_list(reader, destinations, inbox, entry_id);
        } else if reader.split.is_collapsed() {
            reader.split.set_show_content(false);
        }
        return;
    }
    // Back should affect Library only when Library is the current destination.
    if destinations.visible_child_name().as_deref() == Some("library") {
        navigation.pop();
    }
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

fn update_shortcut_tooltips(widget: &gtk::Widget) {
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && let Some(action) = button.action_name()
        && let Some(tooltip) = widget.tooltip_text()
    {
        let hint = keyboard::action_hint(&action);
        if !hint.is_empty() {
            widget.set_tooltip_text(Some(&format!("{tooltip} ({hint})")));
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        update_shortcut_tooltips(&widget);
        child = widget.next_sibling();
    }
}

fn focus_has_popup(window: &adw::ApplicationWindow) -> bool {
    let mut current = gtk::prelude::GtkWindowExt::focus(window);
    while let Some(widget) = current {
        if widget.is::<gtk::Popover>() {
            return true;
        }
        current = widget.parent();
    }
    false
}

fn reader_navigation_is_native(window: &adw::ApplicationWindow, key: gtk::gdk::Key) -> bool {
    let Some(focus) = gtk::prelude::GtkWindowExt::focus(window) else {
        return false;
    };
    // Selectable labels own their cursor movement and selection shortcuts.
    // J/K remain explicit reading shortcuts, regardless of text focus.
    if key != gtk::gdk::Key::j
        && key != gtk::gdk::Key::k
        && focus
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.is_selectable())
    {
        return true;
    }
    key == gtk::gdk::Key::space && (focus.is::<gtk::Button>() || focus.is::<gtk::LinkButton>())
}

fn list_page_step(list: &gtk::ListView) -> i32 {
    let row_height = ui::inbox::selected_id(list)
        .and_then(|id| find_article_row(list.upcast_ref(), id))
        .map(|row| row.height().max(1))
        .unwrap_or(72);
    (list.height() / row_height).max(1)
}

fn move_list_to(list: &gtk::ListView, position: u32) -> bool {
    let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>() else {
        return false;
    };
    if selection.n_items() == 0 {
        return false;
    }
    list.grab_focus();
    selection.set_selected(position.min(selection.n_items() - 1));
    list.scroll_to(selection.selected(), gtk::ListScrollFlags::FOCUS, None);
    true
}

fn install_article_cursor_keys(
    window: &adw::ApplicationWindow,
    destinations: &adw::ViewStack,
    reader_page: &adw::NavigationPage,
    reader_scroller: &gtk::ScrolledWindow,
    read_context: ReadContext,
) -> gtk::EventControllerKey {
    use keyboard::Command;
    let bindings = keyboard::parsed_bindings();
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    // Track physical keys: navigation may repeat, state changes and activation may not.
    let held = Rc::new(RefCell::new(HashSet::<u32>::new()));
    keys.connect_key_released({
        let held = held.clone();
        move |_, _, keycode, _| {
            held.borrow_mut().remove(&keycode);
        }
    });
    window.connect_is_active_notify({
        let held = held.clone();
        move |window| {
            if !window.is_active() {
                held.borrow_mut().clear();
            }
        }
    });
    keys.connect_key_pressed({
        let window = window.downgrade();
        let destinations = destinations.downgrade();
        let reader_page = reader_page.downgrade();
        let reader_scroller = reader_scroller.downgrade();
        move |_, key, keycode, modifiers| {
            let (Some(window), Some(destinations), Some(reader_page), Some(reader_scroller)) = (
                window.upgrade(),
                destinations.upgrade(),
                reader_page.upgrade(),
                reader_scroller.upgrade(),
            ) else {
                return adw::glib::Propagation::Proceed;
            };
            let activate = |name: &str| {
                let _ = gtk::prelude::WidgetExt::activate_action(&window, name, None);
                adw::glib::Propagation::Stop
            };
            if opens_shortcuts(key, modifiers) {
                return activate("app.show-shortcuts");
            }
            let modifiers = keyboard::modifiers(modifiers);
            let Some(binding) = bindings
                .iter()
                .find(|(bound, mods, _)| *mods == modifiers && *bound == key.to_lower())
                .map(|(_, _, binding)| *binding)
            else {
                return adw::glib::Propagation::Proceed;
            };
            let command = binding.command;
            // Let GTK dispatch standard accelerators to actions (and text widgets).
            if matches!(command, Command::Action(_)) {
                return adw::glib::Propagation::Proceed;
            }
            if keycode != 0 && held.borrow().contains(&keycode) {
                return adw::glib::Propagation::Stop;
            }
            if focus_has_popup(&window) {
                return adw::glib::Propagation::Proceed;
            }
            if command == Command::Back {
                if keycode != 0 {
                    held.borrow_mut().insert(keycode);
                }
                return activate("win.back");
            }
            let dialog = focused_dialog(window.upcast_ref()).is_some();
            if matches!(
                command,
                Command::NextPane | Command::PreviousPane | Command::Destination(_) | Command::Menu
            ) {
                if dialog {
                    return adw::glib::Propagation::Proceed;
                }
                match command {
                    Command::Destination(name) => {
                        if read_context.reader.split.is_collapsed() {
                            read_context.reader.split.set_show_content(false);
                        }
                        destinations.set_visible_child_name(name);
                        if let Some(list) = destinations
                            .visible_child()
                            .and_then(|child| mapped_article_list(&child))
                        {
                            list.grab_focus();
                        } else {
                            destinations.child_focus(gtk::DirectionType::TabForward);
                        }
                    }
                    Command::Menu => {
                        show_main_menu(&window);
                    }
                    _ => {
                        if read_context.reader.split.is_collapsed() || !reader_scroller.is_mapped()
                        {
                            return adw::glib::Propagation::Proceed;
                        }
                        let focus = article_key_focus(
                            &window,
                            &reader_page,
                            &reader_scroller,
                            &destinations,
                            false,
                        );
                        if matches!(focus, ArticleKeyFocus::Reader) {
                            if let Some(list) = destinations
                                .visible_child()
                                .and_then(|child| mapped_article_list(&child))
                            {
                                list.grab_focus();
                            } else {
                                destinations.child_focus(gtk::DirectionType::TabForward);
                            }
                        } else {
                            reader_scroller.grab_focus();
                        }
                    }
                }
                if keycode != 0 {
                    held.borrow_mut().insert(keycode);
                }
                return adw::glib::Propagation::Stop;
            }
            let navigation = matches!(
                command,
                Command::Next
                    | Command::Previous
                    | Command::First
                    | Command::Last
                    | Command::PageDown
                    | Command::PageUp
            );
            let scope = article_key_focus(
                &window,
                &reader_page,
                &reader_scroller,
                &destinations,
                navigation,
            );
            if matches!(scope, ArticleKeyFocus::Other) {
                return adw::glib::Propagation::Proceed;
            }
            if binding.context == keyboard::Context::Reader
                && !matches!(scope, ArticleKeyFocus::Reader)
            {
                return adw::glib::Propagation::Proceed;
            }
            if binding.context == keyboard::Context::List
                && !matches!(scope, ArticleKeyFocus::List(_))
            {
                return adw::glib::Propagation::Proceed;
            }
            if navigation
                && matches!(scope, ArticleKeyFocus::Reader)
                && reader_navigation_is_native(&window, key.to_lower())
            {
                return adw::glib::Propagation::Proceed;
            }
            if command == Command::Open
                && (!matches!(scope, ArticleKeyFocus::List(_))
                    || gtk::prelude::GtkWindowExt::focus(&window)
                        .is_some_and(|focus| focus.is::<gtk::Button>()))
            {
                return adw::glib::Propagation::Proceed;
            }
            if !navigation && keycode != 0 {
                held.borrow_mut().insert(keycode);
            }
            if command == keyboard::Command::Read {
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
                        return activate("win.keep-unread");
                    }
                    ArticleKeyFocus::Other => {}
                }
            }
            let action = match command {
                Command::Save => Some("win.toggle-star"),
                Command::Browser => Some("win.open-browser"),
                Command::Copy => Some("win.copy-link"),
                Command::Undo => Some("win.undo"),
                Command::NextArticle if matches!(scope, ArticleKeyFocus::Reader) => {
                    Some("win.next-article")
                }
                Command::PreviousArticle if matches!(scope, ArticleKeyFocus::Reader) => {
                    Some("win.previous-article")
                }
                _ => None,
            };
            if let Some(action) = action {
                return activate(action);
            }
            if let ArticleKeyFocus::List(list) = &scope {
                match command {
                    Command::Open => {
                        if gtk::prelude::GtkWindowExt::focus(&window)
                            .is_some_and(|focus| focus.is::<gtk::Button>())
                        {
                            return adw::glib::Propagation::Proceed;
                        }
                        if let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>()
                            && selection.selected() != gtk::INVALID_LIST_POSITION
                        {
                            list.emit_by_name::<()>("activate", &[&selection.selected()]);
                        }
                    }
                    Command::Next | Command::Previous => {
                        ui::inbox::move_cursor(list, if command == Command::Next { 1 } else { -1 });
                    }
                    Command::First => {
                        move_list_to(list, 0);
                    }
                    Command::Last => {
                        move_list_to(list, u32::MAX);
                    }
                    Command::PageDown | Command::PageUp => {
                        ui::inbox::move_cursor(
                            list,
                            list_page_step(list)
                                * if command == Command::PageDown { 1 } else { -1 },
                        );
                    }
                    _ => return adw::glib::Propagation::Proceed,
                }
                return adw::glib::Propagation::Stop;
            }
            if matches!(scope, ArticleKeyFocus::Reader) && navigation {
                let adjustment = reader_scroller.vadjustment();
                let lower = adjustment.lower();
                let upper = (adjustment.upper() - adjustment.page_size()).max(lower);
                let step = adjustment.step_increment().max(40.0);
                let page = (adjustment.page_size() * 0.9).max(step);
                let target = match command {
                    Command::First => lower,
                    Command::Last => upper,
                    Command::Next => adjustment.value() + step,
                    Command::Previous => adjustment.value() - step,
                    Command::PageDown => adjustment.value() + page,
                    Command::PageUp => adjustment.value() - page,
                    _ => unreachable!(),
                };
                adjustment.set_value(target.clamp(lower, upper));
                return adw::glib::Propagation::Stop;
            }
            adw::glib::Propagation::Proceed
        }
    });
    window.add_controller(keys.clone());
    keys
}

fn show_main_menu(window: &adw::ApplicationWindow) {
    let Some(menu) = find_menu_button(window.upcast_ref(), "app_menu") else {
        return;
    };
    if menu.is_mapped() {
        menu.popup();
        return;
    }
    // The Inbox menu button is hidden on other destinations and in a narrow
    // reader. Present the same menu against the visible window content there.
    let (Some(model), Some(content)) = (menu.menu_model(), window.content()) else {
        return;
    };
    let popup = gtk::PopoverMenu::from_model(Some(&model));
    popup.set_parent(&content);
    popup.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        content.width().saturating_sub(48),
        24,
        1,
        1,
    )));
    popup.connect_closed(|popup| popup.unparent());
    popup.popup();
}

fn find_menu_button(widget: &gtk::Widget, name: &str) -> Option<gtk::MenuButton> {
    if widget.widget_name() == name {
        return widget.clone().downcast().ok();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = find_menu_button(&widget, name) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
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
        return list.has_css_class("article-list").then_some(list);
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

pub(crate) fn replace_view_entries(
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
                refresh_inbox(&reload_controller, &inbox_ui, result.inbox);
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
    (0..inbox_ui.model.store.n_items())
        .filter_map(|position| ui::inbox::entry_at(&inbox_ui.model, position))
        .collect()
}

fn show_entries(inbox_ui: &InboxUi, entries: Vec<Entry>) {
    inbox_ui
        .refresh_generation
        .set(inbox_ui.refresh_generation.get().wrapping_add(1));
    apply_inbox_entries(inbox_ui, entries);
}

fn refresh_inbox(controller: &AppController, inbox: &InboxUi, entries: Vec<Entry>) {
    let selected = ui::inbox::selected_id(&inbox.list)
        .and_then(|id| ui::inbox::entry_by_id(&inbox.model, id))
        .filter(|selected| !entries.iter().any(|entry| entry.id == selected.id));
    if let Some(selected) = &selected {
        // Preserve the selection while checking whether it is read or deleted.
        *inbox.pinned_read.borrow_mut() = Some(selected.clone());
    }
    show_entries(inbox, entries);
    if let Some(selected) = selected {
        let generation = inbox.refresh_generation.get();
        let weak = inbox.downgrade();
        controller.cached_entry(selected.id, move |result| {
            let Some(inbox) = weak.upgrade() else {
                return;
            };
            if inbox.refresh_generation.get() != generation
                || ui::inbox::selected_id(&inbox.list) != Some(selected.id)
                || !inbox
                    .pinned_read
                    .borrow()
                    .as_ref()
                    .is_some_and(|entry| entry.id == selected.id)
                || inbox.list.root().is_none()
            {
                return;
            }
            match result {
                Ok(Some(cached)) => {
                    // Keep current UI fields (including newer local stars); this
                    // read-only lookup confirms only read state and existence.
                    let Some(mut entry) = ui::inbox::entry_by_id(&inbox.model, selected.id) else {
                        return;
                    };
                    entry.read = cached.read;
                    *inbox.pinned_read.borrow_mut() = cached.read.then(|| entry.clone());
                    inbox.rebuilding.set(true);
                    ui::inbox::update_entry(&inbox.model, entry);
                    ui::inbox::select_id(&inbox.list, selected.id);
                    inbox.rebuilding.set(false);
                }
                Ok(None) => release_read_pin_unless(&inbox, None),
                Err(error) => tracing::warn!(%error, "could not confirm selected article state"),
            }
        });
    }
}

fn apply_inbox_entries(inbox_ui: &InboxUi, mut entries: Vec<Entry>) {
    let place = capture_list_place(&inbox_ui.list, &inbox_ui.scroller);
    let pin = inbox_ui.pinned_read.borrow().clone();
    if let Some(pinned) = pin {
        if entries.iter().any(|entry| entry.id == pinned.id) {
            inbox_ui.pinned_read.borrow_mut().take();
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

fn install_read_pin_tracking(inbox: &InboxUi) -> adw::glib::SignalHandlerId {
    inbox.model.selection.connect_selected_item_notify({
        let weak = inbox.downgrade();
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
                        release_read_pin_unless(&inbox, ui::inbox::selected_id(&inbox.list));
                    }
                });
            }
        }
    })
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
    inbox_ui
        .refresh_generation
        .set(inbox_ui.refresh_generation.get().wrapping_add(1));
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
        if let Some(return_to_list) = return_to_list {
            return_to_list();
        }
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
    update_reader_controls(reader_ui, Some(entry));
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
    views.drill_down.refresh_visible();
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

fn update_visible_starred(widget: &gtk::Widget, entry_id: i64, starred: bool) {
    if let Some(list) = widget.downcast_ref::<gtk::ListView>()
        && list.has_css_class("article-list")
        && let Some(position) = ui::inbox::position_of_id(list, entry_id)
        && let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>()
        && let Some(object) = selection
            .item(position)
            .and_downcast::<ui::entry_object::EntryObject>()
        && let Some(store) = selection.model().and_downcast::<gio::ListStore>()
    {
        let selected_id = ui::inbox::selected_id(list);
        let scroller = list.parent().and_downcast::<gtk::ScrolledWindow>();
        let place = scroller
            .as_ref()
            .map(|scroller| capture_list_place(list, scroller));
        let mut entry = object.entry().clone();
        entry.starred = starred;
        store.splice(position, 1, &[ui::entry_object::EntryObject::new(entry)]);
        if let Some(id) = selected_id {
            ui::inbox::select_id(list, id);
        }
        if let (Some(scroller), Some(place)) = (scroller, place) {
            restore_list_place(list, &scroller, place);
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        update_visible_starred(&widget, entry_id, starred);
        child = widget.next_sibling();
    }
}

fn article_action_target(
    window: &adw::ApplicationWindow,
    page: &adw::NavigationPage,
    scroller: &gtk::ScrolledWindow,
    destinations: &adw::ViewStack,
    current: &RefCell<Option<Entry>>,
) -> Option<Entry> {
    match article_key_focus(window, page, scroller, destinations, false) {
        ArticleKeyFocus::List(list) => ui::inbox::selected_from_list(&list),
        // Reader menu activation has popup focus, rather than reader focus.
        _ => current.borrow().clone(),
    }
}

fn reader_menu_model(starred: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    for entries in [
        vec![
            ("Open in Browser", "reader.open-browser"),
            ("Mark as Unread and Return", "reader.keep-unread"),
        ],
        vec![
            (
                if starred { "Unsave" } else { "Save" },
                "reader.toggle-star",
            ),
            ("Send to Karakeep", "reader.send-karakeep"),
            ("Copy Link", "reader.copy-link"),
        ],
        vec![
            ("Previous Article", "reader.previous-article"),
            ("Next Article", "reader.next-article"),
        ],
    ] {
        let section = gio::Menu::new();
        for (label, action) in entries {
            let item = gio::MenuItem::new(Some(label), Some(action));
            if let Some(accelerator) = keyboard::action_accelerator(action) {
                item.set_attribute_value("accel", Some(&accelerator.to_variant()));
            }
            section.append_item(&item);
        }
        menu.append_section(None, &section);
    }
    menu
}

fn update_reader_controls(reader: &ReaderUi, entry: Option<&Entry>) {
    for name in reader.actions.list_actions() {
        if let Some(action) = reader
            .actions
            .lookup_action(&name)
            .and_downcast::<gio::SimpleAction>()
        {
            let enabled = entry.is_some()
                && match name.as_str() {
                    "previous-article" => reader.origin_index.get() > 0,
                    "next-article" => {
                        reader.origin_index.get() + 1 < reader.origin_set.borrow().len()
                    }
                    _ => true,
                };
            action.set_enabled(enabled);
        }
    }
    reader.menu.set_sensitive(entry.is_some());
    reader.menu.set_menu_model(Some(&reader_menu_model(
        entry.is_some_and(|entry| entry.starred),
    )));
}

fn install_reader_controls(window: &adw::ApplicationWindow, reader: &ReaderUi) {
    for name in [
        "open-browser",
        "keep-unread",
        "toggle-star",
        "send-karakeep",
        "copy-link",
        "previous-article",
        "next-article",
    ] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate({
            let window = window.downgrade();
            let scroller = reader.scroller.downgrade();
            move |_, _| {
                if let (Some(window), Some(scroller)) = (window.upgrade(), scroller.upgrade()) {
                    // Reader chrome always targets the open article, even if a list had focus.
                    scroller.grab_focus();
                    let _ = gtk::prelude::WidgetExt::activate_action(
                        &window,
                        &format!("win.{name}"),
                        None,
                    );
                }
            }
        });
        reader.actions.add_action(&action);
    }
    fn compact_labels(widget: &gtk::Widget) {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(24);
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            compact_labels(&widget);
            child = widget.next_sibling();
        }
    }
    compact_labels(reader.title.upcast_ref());
    update_reader_controls(reader, None);
    window.insert_action_group("reader", Some(&reader.actions));
}

fn install_keep_unread_action(
    window: &adw::ApplicationWindow,
    builder: &gtk::Builder,
    context: ReadContext,
) {
    let destinations: adw::ViewStack = builder.object("destinations").unwrap();
    let keep = gio::SimpleAction::new("keep-unread", None);
    keep.connect_activate(move |_, _| {
        let Some(entry) = context.current.borrow().clone() else {
            return;
        };
        let reader = context.reader.clone();
        let destinations = destinations.clone();
        let inbox = context.inbox.list.clone();
        let entry_id = entry.id;
        mark_unread_from(
            context.clone(),
            entry,
            None,
            Some(Rc::new(move || {
                return_to_article_list(&reader, &destinations, &inbox, entry_id);
            })),
        );
    });
    window.add_action(&keep);
}

fn install_reader_actions(
    window: &adw::ApplicationWindow,
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
    let target: Rc<dyn Fn() -> Option<Entry>> = Rc::new({
        let window = window.downgrade();
        let destinations = builder
            .object::<adw::ViewStack>("destinations")
            .unwrap()
            .downgrade();
        let page = builder
            .object::<adw::NavigationPage>("reader_page")
            .unwrap()
            .downgrade();
        let scroller = reader.scroller.downgrade();
        let current = current.clone();
        move || {
            let (Some(window), Some(destinations), Some(page), Some(scroller)) = (
                window.upgrade(),
                destinations.upgrade(),
                page.upgrade(),
                scroller.upgrade(),
            ) else {
                return None;
            };
            article_action_target(&window, &page, &scroller, &destinations, &current)
        }
    });
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
                if let Some(source) = reader
                    .source_list
                    .borrow()
                    .as_ref()
                    .and_then(|source| source.upgrade())
                {
                    ui::inbox::select_id(&source, entry.id);
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
    install_keep_unread_action(
        window,
        builder,
        ReadContext {
            controller: controller.clone(),
            current: current.clone(),
            reader: reader.clone(),
            inbox: inbox.clone(),
            toast: toast.clone(),
            undo: undo.clone(),
        },
    );
    let star = gio::SimpleAction::new("toggle-star", None);
    let star_in_flight = Rc::new(RefCell::new(HashSet::new()));
    star.connect_activate({
        let target = target.clone();
        let window = window.downgrade();
        let controller = controller.clone();
        let current = current.clone();
        let views = views.clone();
        let toast = toast.clone();
        let star_in_flight = star_in_flight.clone();
        move |_, _| {
            let Some(entry) = target() else {
                return;
            };
            if !star_in_flight.borrow_mut().insert(entry.id) {
                return;
            }
            let entry_id = entry.id;
            let desired = !entry.starred;
            controller.set_starred_local(entry_id, desired, {
                let current = current.clone();
                let window = window.clone();
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
                        if let Some(window) = window.upgrade() {
                            update_visible_starred(window.upcast_ref(), entry_id, desired);
                        }
                        update_reader_controls(&reader, current.borrow().as_ref());
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
        let target = target.clone();
        let toast = toast.clone();
        move |_, _| {
            if let (Some(entry), Some(display)) = (target(), gtk::gdk::Display::default()) {
                display.clipboard().set_text(&entry.url);
                toast.add_toast(adw::Toast::new("Link copied"));
            }
        }
    });
    window.add_action(&copy);
    let browser = gio::SimpleAction::new("open-browser", None);
    browser.connect_activate({
        let target = target.clone();
        let window = window.downgrade();
        let toast = toast.clone();
        move |_, _| {
            if let (Some(entry), Some(window)) = (target(), window.upgrade()) {
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

fn install_article_search(
    window: &adw::ApplicationWindow,
    application: &adw::Application,
    builder: &gtk::Builder,
    context: ReadContext,
) {
    let ReadContext {
        controller,
        reader,
        current,
        inbox,
        toast,
        undo,
    } = context;
    let destinations: adw::ViewStack = builder.object("destinations").expect("destinations");
    let navigation: adw::NavigationView = builder
        .object("library_navigation")
        .expect("library_navigation");
    let search_page = Rc::new(RefCell::new(
        None::<(adw::NavigationPage, gtk::SearchEntry, SearchState)>,
    ));
    window.connect_destroy({
        let search_page = search_page.clone();
        move |_| {
            let open = search_page.borrow_mut().take();
            drop(open);
        }
    });
    let search = gio::SimpleAction::new("search", None);
    search.connect_activate({
        let destinations = destinations.clone();
        let navigation = navigation.clone();
        let window = window.downgrade();
        let search_page = search_page.clone();
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
            if reader.split.is_collapsed() {
                reader.split.set_show_content(false);
            }
            if let Some((page, query, state)) = search_page.borrow().as_ref() {
                destinations.set_visible_child_name("library");
                if navigation.visible_page().as_ref() != Some(page) {
                    navigation.push(page);
                }
                state.refresh();
                query.grab_focus();
                return;
            }
            let toolbar = adw::ToolbarView::new();
            toolbar.add_top_bar(&adw::HeaderBar::new());
            let page = adw::NavigationPage::new(&toolbar, "Search Articles");
            page.set_tag(Some("article-search"));
            let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let query = gtk::SearchEntry::new();
            // Debounce once in SearchState, with immediate cancellation of
            // superseded requests as soon as the editable text changes.
            query.set_search_delay(0);
            query.set_widget_name("article-search-query");
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
            let scope = gtk::DropDown::from_strings(&["Library", "Inbox", "Saved"]);
            scope.set_widget_name("article-search-scope");
            scope.set_tooltip_text(Some("Search scope"));
            scope.set_selected(match destinations.visible_child_name().as_deref() {
                Some("inbox") => 1,
                Some("saved") => 2,
                _ => 0,
            });
            filters.append(&scope);
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
            list.set_widget_name("article-search-results");
            let model = ui::inbox::configure_with_action(&list, false, reader.active_id.clone());
            scroller.set_child(Some(&list));
            let status = adw::StatusPage::new();
            status.set_vexpand(true);
            status.set_icon_name(Some("system-search-symbolic"));
            status.set_title("No matching articles");
            status.set_description(Some("Try a different search or filter."));
            body.append(&status);
            body.append(&scroller);
            toolbar.set_content(Some(&body));
            let state = SearchState {
                controller: controller.clone(),
                query: query.clone(),
                feed: feed.clone(),
                category: category.clone(),
                read: read.clone(),
                scope: scope.clone(),
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
            let signal = scope.connect_selected_notify({
                let state = state.clone();
                move |_| state.refresh()
            });
            signals.borrow_mut().push((
                scope.clone().upcast::<adw::glib::Object>().downgrade(),
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
            window.connect_destroy({
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
            *search_page.borrow_mut() = Some((page.clone(), query.clone(), state.clone()));
            state.refresh();
            list.connect_activate({
                let page = page.downgrade();
                let controller = controller.clone();
                let reader = reader.downgrade();
                let current = current.clone();
                let inbox = inbox.downgrade();
                let toast = toast.downgrade();
                let undo = undo.clone();
                move |list, position| {
                    let (Some(_page), Some(reader), Some(inbox), Some(toast)) = (
                        page.upgrade(),
                        reader.upgrade(),
                        inbox.upgrade(),
                        toast.upgrade(),
                    ) else {
                        return;
                    };
                    if let Some(entry) = ui::inbox::entry_at(&model, position) {
                        set_reader_origin(&reader, &model, list, entry.id, false);
                        // Keep the search page and its list alive as the reader origin.
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
            destinations.set_visible_child_name("library");
            navigation.add(&page);
            navigation.push(&page);
            query.grab_focus();
        }
    });
    application.add_action(&search);
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
    let close = gio::SimpleAction::new("close", None);
    close.connect_activate({
        let window = window.downgrade();
        move |_, _| {
            if let Some(window) = window.upgrade() {
                window.close();
            }
        }
    });
    window.add_action(&close);
    update_shortcut_tooltips(window.upcast_ref());
    let menu_button: gtk::MenuButton = builder.object("app_menu").expect("app_menu");
    let menu = gio::Menu::new();
    menu.append(Some("Mark All Read"), Some("win.mark-all-read"));
    for (label, action) in [
        ("Search Articles", "app.search"),
        ("Refresh Feeds", "win.refresh-feeds"),
        ("Subscribe…", "win.subscribe"),
        ("Preferences", "win.preferences"),
        ("Keyboard Shortcuts", "app.show-shortcuts"),
        ("About Brooklet", "win.about"),
    ] {
        menu.append(Some(label), Some(action));
    }
    menu_button.set_widget_name("app_menu");
    menu_button.set_menu_model(Some(&menu));

    install_article_search(
        window,
        application,
        builder,
        ReadContext {
            controller: controller.clone(),
            reader: reader.clone(),
            current: current.clone(),
            inbox: inbox.clone(),
            toast: toast.clone(),
            undo: undo.clone(),
        },
    );

    let refresh = gio::SimpleAction::new("refresh-feeds", None);
    refresh.connect_activate({
        let controller = controller.clone();
        let inbox = inbox.clone();
        let views = views.clone();
        let toast = toast.clone();
        move |action, _| {
            action.set_enabled(false);
            toast.add_toast(adw::Toast::new("Refreshing feeds…"));
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
                            refresh_inbox(&controller, &inbox, result.inbox);
                            load_other_views(controller, views, toast.clone());
                            let message = changes.toast_message().unwrap_or_else(|| {
                                "No new articles yet. Slow feeds may appear on the next sync."
                                    .into()
                            });
                            toast.add_toast(adw::Toast::new(&message));
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

    let reconnect = gio::SimpleAction::new("reconnect", None);
    reconnect.connect_activate({
        let window = window.downgrade();
        let controller = controller.clone();
        let toast = toast.clone();
        move |_, _| {
            controller.existing_account({
                let window = window.clone();
                let controller = controller.clone();
                let toast = toast.clone();
                move |result| {
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    match result {
                        Ok(Some(account)) => {
                            ui::reconnect::present(&window, controller, &account, {
                                let window = window.downgrade();
                                move |_| {
                                    if let Some(window) = window.upgrade() {
                                        ui::reconnect::sync_when_ready(&window);
                                    }
                                }
                            })
                        }
                        Ok(None) => {}
                        Err(error) => toast.add_toast(adw::Toast::new(&error.sync_message())),
                    }
                }
            });
        }
    });
    window.add_action(&reconnect);
    let deliveries = gio::SimpleAction::new("delivery-review", None);
    deliveries.connect_activate({
        let window = window.downgrade();
        let controller = controller.clone();
        move |_, _| {
            if let Some(window) = window.upgrade() {
                let weak = window.downgrade();
                ui::karakeep::present(&window, controller.clone(), move || {
                    if let Some(window) = weak.upgrade() {
                        ui::reconnect::sync_when_ready(&window);
                    }
                });
            }
        }
    });
    window.add_action(&deliveries);

    let preferences = gio::SimpleAction::new("preferences", None);
    preferences.connect_activate({
        let window = window.downgrade();
        let application = application.downgrade();
        let controller = controller.clone();
        let toast = toast.clone();
        move |_, _| {
            let (Some(window), Some(application)) = (window.upgrade(), application.upgrade()) else {
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
            let connected_account = Rc::new(RefCell::new(None::<brooklet::model::Account>));
            let reconnect_row = adw::ActionRow::new();
            reconnect_row.set_title("Reconnect account");
            reconnect_row.set_subtitle("Replace the API token while keeping cached articles and pending changes");
            let reconnect_button = gtk::Button::with_label("Reconnect…");
            reconnect_button.set_valign(gtk::Align::Center);
            reconnect_button.set_sensitive(false);
            reconnect_row.add_suffix(&reconnect_button);
            reconnect_row.set_activatable_widget(Some(&reconnect_button));
            account_group.add(&reconnect_row);
            signals.track(&reconnect_button, reconnect_button.connect_clicked({
                let account = connected_account.clone();
                let controller = controller.clone();
                let window = window.downgrade();
                let dialog = dialog.clone();
                let toast = toast.clone();
                move |_| {
                    let Some(window) = window.upgrade() else { return; };
                    let Some(account) = account.borrow().clone() else { return; };
                    dialog.close();
                    ui::reconnect::present(&window, controller.clone(), &account, {
                        let window = window.downgrade();
                        let toast = toast.clone();
                        move |_| {
                            toast.add_toast(adw::Toast::new("Account reconnected"));
                            if let Some(window) = window.upgrade() {
                                ui::reconnect::sync_when_ready(&window);
                            }
                        }
                    });
                }
            }));
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
            direct.set_sensitive(false);
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
            let deliveries = gtk::Button::with_label("Review failed deliveries…");
            deliveries.set_margin_top(8);
            deliveries.set_visible(false);
            karakeep.add(&deliveries);
            controller.unfinished_karakeep({
                let deliveries = deliveries.downgrade();
                move |result| {
                    if let (Some(button), Ok(pending)) = (deliveries.upgrade(), result) {
                        button.set_visible(pending.iter().any(|delivery| delivery.error.is_some()));
                    }
                }
            });
            signals.track(&deliveries, deliveries.connect_clicked({
                let window = window.clone();
                let dialog = dialog.clone();
                let controller = controller.clone();
                move |_| {
                    dialog.close();
                    let weak = window.downgrade();
                    ui::karakeep::present(&window, controller.clone(), move || {
                        if let Some(window) = weak.upgrade() { ui::reconnect::sync_when_ready(&window); }
                    });
                }
            }));
            page.add(&karakeep);
            let diagnostics = adw::PreferencesGroup::new();
            diagnostics.set_title("Sync diagnostics");
            let health_host = gtk::Box::new(gtk::Orientation::Vertical, 0);
            diagnostics.add(&health_host);
            ui::sync_health::install_dialog(&dialog, &health_host, controller.clone());
            page.add(&diagnostics);
            dialog.add(&page);
            controller.existing_account(move |result| match result {
                Ok(Some(account)) => {
                    *connected_account.borrow_mut() = Some(account.clone());
                    reconnect_button.set_sensitive(true);
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
                let save = save_karakeep.clone();
                let direct = direct.clone();
                let endpoint = endpoint.clone();
                let toast = toast.clone();
                move |result| {
                    save.set_sensitive(true);
                    direct.set_sensitive(true);
                    match result {
                    Ok(Some(config)) => {
                        direct.set_active(config.route == brooklet::model::KarakeepRoute::Direct);
                        endpoint.set_text(config.direct_endpoint.as_deref().unwrap_or(""));
                    }
                    Ok(None) => {}
                    Err(error) => toast.add_toast(adw::Toast::new(&error.karakeep_message())),
                    }
                }
            });
            save_karakeep.set_sensitive(false);
            signals.track(&save_karakeep, save_karakeep.connect_clicked({
                let controller = controller.clone();
                let direct = direct.clone();
                let endpoint = endpoint.clone();
                let key = key.clone();
                let toast = toast.clone();
                move |button| {
                    if !button.is_sensitive() { return; }
                    button.set_sensitive(false);
                    button.set_label("Checking Karakeep…");
                    direct.set_sensitive(false);
                    endpoint.set_sensitive(false);
                    key.set_sensitive(false);
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
                            let button = button.clone();
                            let direct = direct.clone();
                            let endpoint = endpoint.clone();
                            let key = key.clone();
                            move |result| {
                                button.set_sensitive(true);
                                button.set_label("Save Karakeep settings");
                                direct.set_sensitive(true);
                                endpoint.set_sensitive(direct.is_active());
                                key.set_sensitive(direct.is_active());
                                match result {
                                Ok(()) => {
                                    toast.add_toast(adw::Toast::new("Karakeep settings saved"))
                                }
                                Err(error) => {
                                    toast.add_toast(adw::Toast::new(&error.karakeep_message()))
                                }
                                }
                            }
                        },
                    );
                }
            }));
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

// Capture the second press before the switcher's button handles it. Picking
// the actual button keeps the shortcut correct across resizing and RTL layouts.
fn install_inbox_top_shortcut(switcher: &adw::ViewSwitcherBar, scroller: &gtk::ScrolledWindow) {
    fn has_inbox_icon(widget: &gtk::Widget) -> bool {
        if let Some(image) = widget.downcast_ref::<gtk::Image>()
            && image.icon_name().as_deref() == Some("mail-unread-symbolic")
        {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if has_inbox_icon(&widget) {
                return true;
            }
            child = widget.next_sibling();
        }
        false
    }

    let gesture = gtk::GestureClick::builder()
        .button(1)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let weak_switcher = switcher.downgrade();
    let weak_scroller = scroller.downgrade();
    gesture.connect_pressed(move |gesture, presses, x, y| {
        if presses != 2 {
            return;
        }
        let (Some(switcher), Some(scroller)) = (weak_switcher.upgrade(), weak_scroller.upgrade())
        else {
            return;
        };
        let mut picked = switcher.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(widget) = picked {
            if widget.is::<gtk::ToggleButton>() {
                if has_inbox_icon(&widget) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    let adjustment = scroller.vadjustment();
                    adjustment.set_value(adjustment.lower());
                }
                break;
            }
            picked = widget.parent();
        }
    });
    switcher.add_controller(gesture);
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

fn smoke_test_inbox_top_shortcut(builder: &gtk::Builder) -> Result<(), adw::glib::BoolError> {
    fn buttons(widget: &gtk::Widget, result: &mut Vec<gtk::ToggleButton>) {
        if let Some(button) = widget.downcast_ref::<gtk::ToggleButton>() {
            result.push(button.clone());
            return;
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            buttons(&widget, result);
            child = widget.next_sibling();
        }
    }
    let switcher: adw::ViewSwitcherBar = builder.object("destination_switcher").unwrap();
    // Use an isolated adjustment to verify the real switcher hit targets without
    // disturbing the article-keyboard fixture's scroll state.
    let scroller = gtk::ScrolledWindow::new();
    install_inbox_top_shortcut(&switcher, &scroller);
    let controllers = switcher.observe_controllers();
    let gesture = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .find(|gesture| gesture.propagation_phase() == gtk::PropagationPhase::Capture)
        .ok_or_else(|| adw::glib::bool_error!("Inbox shortcut gesture missing"))?;
    let mut targets = Vec::new();
    buttons(switcher.upcast_ref(), &mut targets);
    if targets.len() != 3 {
        return Err(adw::glib::bool_error!("Expected three destination buttons"));
    }
    let adjustment = scroller.vadjustment();
    for (index, button) in targets.iter().enumerate() {
        let bounds = button
            .compute_bounds(&switcher)
            .ok_or_else(|| adw::glib::bool_error!("Destination button has no bounds"))?;
        let x = f64::from(bounds.x() + bounds.width() / 2.0);
        let y = f64::from(bounds.y() + bounds.height() / 2.0);
        adjustment.configure(500.0, 0.0, 1000.0, 1.0, 100.0, 100.0);
        gesture.emit_by_name::<()>("pressed", &[&1_i32, &x, &y]);
        if adjustment.value() != 500.0 {
            return Err(adw::glib::bool_error!(
                "Single destination press moved Inbox"
            ));
        }
        gesture.emit_by_name::<()>("pressed", &[&2_i32, &x, &y]);
        let expected = if index == 0 { 0.0 } else { 500.0 };
        if adjustment.value() != expected {
            return Err(adw::glib::bool_error!(
                "Inbox double press targeted wrong destination"
            ));
        }
    }
    switcher.remove_controller(&gesture);
    Ok(())
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
    let keyboard_window_title = format!(
        "Brooklet Keyboard Regression {}",
        adw::glib::uuid_string_random()
    );
    window.set_title(Some(&keyboard_window_title));
    reader.scroller.set_focusable(true);
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
    smoke_test_inbox_top_shortcut(builder)?;
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
    collection.add_css_class("article-list");
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
    // Reader arrows must change the adjustment, including reader header focus.
    reader_focus.set_height_request(3000);
    layout(window);
    reader.scroller.grab_focus();
    reader.scroller.vadjustment().set_value(0.0);
    if !press(Key::Down, Modifiers::empty()) || reader.scroller.vadjustment().value() <= 0.0 {
        return Err(adw::glib::bool_error!("Reader Down did not scroll"));
    }
    if !press(Key::Up, Modifiers::empty()) || reader.scroller.vadjustment().value() != 0.0 {
        return Err(adw::glib::bool_error!("Reader Up did not scroll"));
    }
    if !press(Key::space, Modifiers::empty())
        || reader.scroller.vadjustment().value() <= 0.0
        || !press(Key::space, Modifiers::SHIFT_MASK)
        || reader.scroller.vadjustment().value() != 0.0
    {
        return Err(adw::glib::bool_error!("Reader Space paging failed"));
    }
    for key in [Key::End, Key::Home, Key::Page_Down, Key::Page_Up] {
        if !press(key, Modifiers::empty()) {
            return Err(adw::glib::bool_error!("Reader paging failed for {key:?}"));
        }
    }
    reader_focus.grab_focus();
    if press(Key::space, Modifiers::empty()) || press(Key::Return, Modifiers::empty()) {
        return Err(adw::glib::bool_error!(
            "Reader intercepted button activation"
        ));
    }
    let text = gtk::Label::new(Some("Selectable reader text"));
    text.set_selectable(true);
    reader.content.prepend(&text);
    layout(window);
    text.grab_focus();
    for key in [Key::Up, Key::Down, Key::Home, Key::End] {
        if press(key, Modifiers::empty()) {
            return Err(adw::glib::bool_error!(
                "Reader intercepted text cursor {key:?}"
            ));
        }
    }
    if press(Key::Right, Modifiers::SHIFT_MASK) || press(Key::c, Modifiers::CONTROL_MASK) {
        return Err(adw::glib::bool_error!(
            "Reader intercepted text selection or copy"
        ));
    }
    reader.content.remove(&text);
    inbox.list.grab_focus();
    for (key, expected) in [
        (Key::End, 1),
        (Key::Home, 0),
        (Key::Page_Down, 1),
        (Key::Page_Up, 0),
    ] {
        if !press(key, Modifiers::empty()) || inbox.model.selection.selected() != expected {
            return Err(adw::glib::bool_error!("List paging failed for {key:?}"));
        }
    }
    let open = RefCell::new(ui::inbox::entry_at(&inbox.model, 1));
    if article_action_target(window, &reader_page, &reader.scroller, &destinations, &open)
        .map(|entry| entry.id)
        != ui::inbox::selected_id(&inbox.list)
    {
        return Err(adw::glib::bool_error!(
            "List action targeted the open article"
        ));
    }
    reader.scroller.grab_focus();
    if article_action_target(window, &reader_page, &reader.scroller, &destinations, &open)
        .map(|entry| entry.id)
        != open.borrow().as_ref().map(|entry| entry.id)
    {
        return Err(adw::glib::bool_error!(
            "Reader action targeted the list selection"
        ));
    }
    // A held mutation shortcut is consumed once, until the physical release.
    let physical_press =
        || keys.emit_by_name::<bool>("key-pressed", &[&Key::u, &42_u32, &Modifiers::empty()]);
    if !physical_press() || !physical_press() || undos.get() != 5 {
        return Err(adw::glib::bool_error!("Held Undo repeated"));
    }
    keys.emit_by_name::<()>("key-released", &[&Key::u, &42_u32, &Modifiers::empty()]);
    if !physical_press() || undos.get() != 6 {
        return Err(adw::glib::bool_error!("Undo did not reset on release"));
    }
    keys.emit_by_name::<()>("key-released", &[&Key::u, &42_u32, &Modifiers::empty()]);
    let back = gio::SimpleAction::new("back", None);
    back.connect_activate({
        let window = window.downgrade();
        let reader = reader.clone();
        let page = reader_page.clone();
        let destinations = destinations.clone();
        let inbox = inbox.list.clone();
        let navigation: adw::NavigationView = builder.object("library_navigation").unwrap();
        move |_, _| {
            if let Some(window) = window.upgrade() {
                navigate_back(&window, &reader, &page, &destinations, &inbox, &navigation);
            }
        }
    });
    window.add_action(&back);
    // Physical delivery supplements handler assertions in the required X11 CI gate.
    if let Some(driver) = std::env::var_os("BROOKLET_KEYBOARD_DRIVER") {
        let real_event = |key: &str,
                          mode: &str,
                          modifiers: &[&str]|
         -> Result<(), adw::glib::BoolError> {
            let status = std::process::Command::new("python3")
                .arg(&driver)
                .arg(&keyboard_window_title)
                .arg(key)
                .arg(mode)
                .args(modifiers)
                .status()
                .map_err(|error| adw::glib::bool_error!("Keyboard event driver failed: {error}"))?;
            if !status.success() {
                return Err(adw::glib::bool_error!(
                    "Keyboard event driver failed for {key}"
                ));
            }
            layout(window);
            Ok(())
        };
        let real_key = |key: &str, modifiers: &[&str]| real_event(key, "both", modifiers);
        inbox.list.grab_focus();
        inbox.model.selection.set_selected(0);
        layout(window);
        header.grab_focus();
        layout(window);
        // The test display may have no window manager to activate this window.
        // Process its native FocusIn separately from the first key event.
        real_event("Down", "focus", &[])?;
        real_key("Down", &[])?;
        if inbox.model.selection.selected() != 1 {
            return Err(adw::glib::bool_error!("Physical Down failed from header"));
        }
        real_key("Up", &[])?;
        if inbox.model.selection.selected() != 0 {
            return Err(adw::glib::bool_error!("Physical Up failed in list"));
        }
        let before = activations.get();
        real_key("Return", &[])?;
        if activations.get() != before + 1 {
            return Err(adw::glib::bool_error!("Physical Enter failed to activate"));
        }
        reader.scroller.grab_focus();
        reader.scroller.vadjustment().set_value(0.0);
        real_key("Down", &[])?;
        if reader.scroller.vadjustment().value() <= 0.0 {
            return Err(adw::glib::bool_error!(
                "Physical reader Down did not scroll"
            ));
        }
        real_key("Up", &[])?;
        if reader.scroller.vadjustment().value() != 0.0 {
            return Err(adw::glib::bool_error!("Physical reader Up did not scroll"));
        }
        real_key("F6", &[])?;
        if !matches!(
            article_key_focus(window, &reader_page, &reader.scroller, &destinations, false),
            ArticleKeyFocus::List(_)
        ) {
            return Err(adw::glib::bool_error!("Physical F6 did not focus list"));
        }
        real_key("F6", &["Shift_L"])?;
        if !matches!(
            article_key_focus(window, &reader_page, &reader.scroller, &destinations, false),
            ArticleKeyFocus::Reader
        ) {
            return Err(adw::glib::bool_error!(
                "Physical Shift+F6 did not focus reader"
            ));
        }
        let held_before = undos.get();
        real_event("u", "press", &[])?;
        let repeat_loop = adw::glib::MainLoop::new(None, false);
        adw::glib::timeout_add_local_once(Duration::from_millis(900), {
            let repeat_loop = repeat_loop.clone();
            move || repeat_loop.quit()
        });
        repeat_loop.run();
        real_event("u", "release", &[])?;
        if undos.get() != held_before + 1 {
            return Err(adw::glib::bool_error!("Physical held Undo repeated"));
        }
        let before = undos.get();
        real_key("z", &["Control_L"])?;
        if undos.get() != before + 1 {
            return Err(adw::glib::bool_error!("Physical Ctrl+Z failed"));
        }
        let dialog = adw::Dialog::new();
        let input = gtk::Entry::new();
        dialog.set_child(Some(&input));
        dialog.present(Some(window));
        layout(window);
        input.grab_focus();
        real_key("r", &[])?;
        if input.text() != "r" {
            return Err(adw::glib::bool_error!("Physical R intercepted text entry"));
        }
        real_key("Down", &[])?;
        if undos.get() != before + 1 {
            return Err(adw::glib::bool_error!("Dialog key changed article state"));
        }
        real_key("Escape", &[])?;
        // Dialog closing is animated; wait for it to leave the window completely.
        for _ in 0..10 {
            if dialog.parent().is_none() {
                break;
            }
            layout(window);
        }
        if dialog.parent().is_some() {
            return Err(adw::glib::bool_error!(
                "Physical Escape did not dismiss dialog"
            ));
        }
        reader.active_id.set(ui::inbox::selected_id(&inbox.list));
        *reader.source_list.borrow_mut() = Some(inbox.list.downgrade());
        reader.scroller.grab_focus();
        real_key("Escape", &[])?;
        if !matches!(
            article_key_focus(window, &reader_page, &reader.scroller, &destinations, false),
            ArticleKeyFocus::List(_)
        ) {
            return Err(adw::glib::bool_error!(
                "Physical Escape did not restore source-list focus"
            ));
        }
        real_key("Down", &[])?;
        if inbox.model.selection.selected() != 1 {
            return Err(adw::glib::bool_error!("Physical Down failed after Escape"));
        }
        let menu: gtk::MenuButton = builder.object("app_menu").unwrap();
        menu.set_widget_name("app_menu");
        let model = gio::Menu::new();
        model.append(Some("Undo"), Some("win.undo"));
        menu.set_menu_model(Some(&model));
        real_key("F10", &[])?;
        if !menu.popover().is_some_and(|popover| popover.is_visible()) {
            return Err(adw::glib::bool_error!(
                "Physical F10 did not open main menu"
            ));
        }
        real_key("Escape", &[])?;
        if menu.popover().is_some_and(|popover| popover.is_visible()) {
            return Err(adw::glib::bool_error!("Physical Escape did not close menu"));
        }
        // Saved and Library use the same article-list contract, with their own selection.
        let saved: gtk::ListView = builder.object("saved_list").unwrap();
        let saved_model = ui::inbox::configure_with_action(&saved, false, Rc::new(Cell::new(None)));
        ui::inbox::replace(&saved_model, inbox_snapshot(inbox));
        builder
            .object::<adw::StatusPage>("saved_status")
            .unwrap()
            .set_visible(false);
        builder
            .object::<gtk::ScrolledWindow>("saved_scroller")
            .unwrap()
            .set_visible(true);
        real_key("2", &["Control_L"])?;
        real_key("Home", &[])?;
        real_key("Down", &[])?;
        if destinations.visible_child_name().as_deref() != Some("saved")
            || saved_model.selection.selected() != 1
        {
            return Err(adw::glib::bool_error!("Physical Saved shortcuts failed"));
        }
        real_key("F10", &[])?;
        if !focus_has_popup(window) {
            return Err(adw::glib::bool_error!("Physical F10 failed outside Inbox"));
        }
        real_key("Escape", &[])?;
        let library: gtk::ListView = builder.object("library_all_list").unwrap();
        let library_model =
            ui::inbox::configure_with_action(&library, false, Rc::new(Cell::new(None)));
        ui::inbox::replace(&library_model, inbox_snapshot(inbox));
        builder
            .object::<adw::StatusPage>("library_all_status")
            .unwrap()
            .set_visible(false);
        builder
            .object::<gtk::ScrolledWindow>("library_all_scroller")
            .unwrap()
            .set_visible(true);
        real_key("3", &["Control_L"])?;
        let navigation: adw::NavigationView = builder.object("library_navigation").unwrap();
        navigation.push_by_tag("library-all");
        layout(window);
        library.grab_focus();
        real_key("Home", &[])?;
        real_key("j", &[])?;
        real_key("k", &[])?;
        if destinations.visible_child_name().as_deref() != Some("library")
            || library_model.selection.selected() != 0
        {
            return Err(adw::glib::bool_error!("Physical Library shortcuts failed"));
        }
        real_key("Left", &["Alt_L"])?;
        real_key("1", &["Control_L"])?;
        if destinations.visible_child_name().as_deref() != Some("inbox") {
            return Err(adw::glib::bool_error!("Physical Inbox shortcut failed"));
        }
        // The same return contract must hold when the list is hidden in a narrow window.
        window.set_default_size(550, 700);
        // Window managers may ignore resizing an already mapped window. Exercise
        // the breakpoint's collapsed mode explicitly, independently of that policy.
        reader.split.set_collapsed(true);
        reader.active_id.set(ui::inbox::selected_id(&inbox.list));
        layout(window);
        if !reader.split.is_collapsed() {
            return Err(adw::glib::bool_error!(
                "Narrow keyboard fixture did not collapse"
            ));
        }
        reader.split.set_show_content(true);
        layout(window);
        reader.scroller.grab_focus();
        real_key("Escape", &[])?;
        layout(window);
        if reader.split.shows_content()
            || !matches!(
                article_key_focus(window, &reader_page, &reader.scroller, &destinations, false),
                ArticleKeyFocus::List(_)
            )
        {
            return Err(adw::glib::bool_error!(
                "Narrow Escape lost source-list focus"
            ));
        }
        let narrow_selection = inbox.model.selection.selected();
        real_key("Up", &[])?;
        // Collapsing reparents the list and queues focus/layout work. Observe
        // the cursor result rather than assuming three frames finish it all.
        for _ in 0..10 {
            if inbox.model.selection.selected() == 0 {
                break;
            }
            layout(window);
        }
        if inbox.model.selection.selected() != 0 {
            return Err(adw::glib::bool_error!(
                "Narrow list Up failed: before={narrow_selection}, after={}, focus={:?}",
                inbox.model.selection.selected(),
                gtk::prelude::GtkWindowExt::focus(window)
                    .map(|widget| widget.type_().name().to_string())
            ));
        }
        window.set_default_size(1080, 720);
        reader.active_id.set(None);
        layout(window);
        println!("Physical keyboard event regression passed (wide and narrow)");
    }
    reader.content.remove(&reader_focus);
    window.remove_action("undo");
    window.remove_action("keep-unread");
    window.remove_action("back");
    inbox.list.disconnect(activation_signal);
    window.remove_controller(&keys);
    Ok(())
}

fn smoke_test_automatic_inbox(
    window: &adw::ApplicationWindow,
    inbox: &InboxUi,
    reader: &ReaderUi,
    repository: &SqliteRepository,
    template: &Entry,
) -> Result<(), adw::glib::BoolError> {
    use brooklet::services::traits::Repository;
    fn wait(predicate: impl Fn() -> bool) -> Result<(), adw::glib::BoolError> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let context = adw::glib::MainContext::default();
        while !predicate() {
            while context.pending() {
                context.iteration(false);
            }
            if std::time::Instant::now() > deadline {
                return Err(adw::glib::bool_error!("Automatic inbox update timed out"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
    fn layout() -> Result<(), adw::glib::BoolError> {
        let settled = Rc::new(Cell::new(false));
        adw::glib::timeout_add_local_once(Duration::from_millis(350), {
            let settled = settled.clone();
            move || settled.set(true)
        });
        wait(|| settled.get())
    }
    fn check(value: bool, message: &str) -> Result<(), adw::glib::BoolError> {
        if value {
            Ok(())
        } else {
            Err(adw::glib::bool_error!("{message}"))
        }
    }
    let initial = (1000..1120)
        .rev()
        .map(|id| Entry {
            id,
            title: format!("Story {id}"),
            published_at_ms: id * 1000,
            html: "<p>Reader paragraph</p>".repeat(80),
            read: false,
            starred: false,
            ..template.clone()
        })
        .collect::<Vec<_>>();
    let before_pending = reader
        .controller
        .backend_handle()
        .block_on(repository.pending_mutations(1))
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    reader
        .controller
        .backend_handle()
        .block_on(repository.merge_changed_page(1, &initial, &[]))
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    let tracking = install_read_pin_tracking(inbox);
    reader.split.set_show_content(false);
    window.present();
    show_entries(inbox, initial.clone());
    layout()?;
    ui::inbox::select_id(&inbox.list, 1100);
    inbox.list.grab_focus();
    layout()?;
    inbox.scroller.vadjustment().set_value(800.0);
    layout()?;
    let before = capture_list_place(&inbox.list, &inbox.scroller);
    check(
        before.entry_id.is_some() && before.value > 40.0,
        "Automatic inbox viewport fixture missing",
    )?;
    let mut updated = initial.clone();
    updated
        .iter_mut()
        .find(|entry| entry.id == 1101)
        .unwrap()
        .title = "Updated automatically".into();
    updated.insert(
        0,
        Entry {
            id: 1120,
            title: "New automatically".into(),
            published_at_ms: 1_120_000,
            ..template.clone()
        },
    );
    refresh_inbox(&reader.controller, inbox, updated.clone());
    layout()?;
    let after = capture_list_place(&inbox.list, &inbox.scroller);
    check(
        ui::inbox::entry_by_id(&inbox.model, 1120).is_some()
            && ui::inbox::entry_by_id(&inbox.model, 1101).unwrap().title == "Updated automatically"
            && ui::inbox::selected_id(&inbox.list) == Some(1100),
        "Scrolled inbox deferred updates or lost selection",
    )?;
    check(
        before.entry_id == after.entry_id && (before.row_y - after.row_y).abs() < 2.0,
        "Automatic additions lost viewport anchor",
    )?;
    check(
        gtk::prelude::GtkWindowExt::focus(window).is_some_and(|focus| {
            focus == inbox.list.clone().upcast::<gtk::Widget>() || focus.is_ancestor(&inbox.list)
        }),
        "Automatic update lost list focus",
    )?;

    // Keep the open reader and selected row when another client reads the entry.
    let selected = initial
        .iter()
        .find(|entry| entry.id == 1100)
        .unwrap()
        .clone();
    // Start from an already-rendered reader; asynchronous construction has
    // separate pipeline coverage and is not part of an inbox refresh.
    ui::reader::begin(&selected, &reader.title, &reader.content);
    let blocks = brooklet::reader::parse_document(&selected.html, Some(&selected.url));
    let mut document = ui::reader::DocumentBuilder::new(blocks);
    while !document.step(&reader.content).1 {}
    reader.active_id.set(Some(selected.id));
    reader.restoring.set(false);
    reader.placeholder.set_visible(false);
    reader.scroller.set_visible(true);
    reader.split.set_show_content(true);
    layout()?;
    reader.scroller.vadjustment().set_value(120.0);
    layout()?;
    let reader_offset = reader.scroller.vadjustment().value();
    check(reader_offset > 0.0, "Reader position fixture missing")?;
    let read = Entry {
        read: true,
        ..selected.clone()
    };
    reader
        .controller
        .backend_handle()
        .block_on(repository.merge_changed_page(1, &[read], &[]))
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    updated.retain(|entry| entry.id != 1100);
    refresh_inbox(&reader.controller, inbox, updated.clone());
    // A later local UI update must not be overwritten by the read-only lookup.
    update_visible_starred(window.upcast_ref(), 1100, true);
    wait(|| ui::inbox::entry_by_id(&inbox.model, 1100).is_some_and(|entry| entry.read)).map_err(
        |_| {
            adw::glib::bool_error!(
                "Remote read not confirmed: selected={:?} pin={:?} generation={}",
                ui::inbox::selected_id(&inbox.list),
                inbox
                    .pinned_read
                    .borrow()
                    .as_ref()
                    .map(|entry| (entry.id, entry.read)),
                inbox.refresh_generation.get()
            )
        },
    )?;
    layout()?;
    check(
        ui::inbox::selected_id(&inbox.list) == Some(1100)
            && ui::inbox::entry_by_id(&inbox.model, 1100).unwrap().starred
            && reader.active_id.get() == Some(1100)
            && (reader.scroller.vadjustment().value() - reader_offset).abs() < 1.0,
        "Remote read lost selected row, newer star, or reader position",
    )?;
    ui::inbox::move_cursor(&inbox.list, 1);
    layout()?;
    check(
        ui::inbox::entry_by_id(&inbox.model, 1100).is_none(),
        "Read row stayed pinned after navigation",
    )?;

    // A newer unread snapshot invalidates an older read-state confirmation.
    show_entries(inbox, initial.clone());
    ui::inbox::select_id(&inbox.list, 1100);
    refresh_inbox(&reader.controller, inbox, updated.clone());
    refresh_inbox(&reader.controller, inbox, initial.clone());
    layout()?;
    check(
        ui::inbox::entry_by_id(&inbox.model, 1100).is_some_and(|entry| !entry.read)
            && inbox.pinned_read.borrow().is_none(),
        "Late read lookup overwrote a newer unread snapshot",
    )?;

    // Restoring unread remotely clears an already-confirmed read pin.
    refresh_inbox(&reader.controller, inbox, updated.clone());
    wait(|| ui::inbox::entry_by_id(&inbox.model, 1100).is_some_and(|entry| entry.read)).map_err(
        |_| {
            adw::glib::bool_error!(
                "Remote read not confirmed: selected={:?} pin={:?} generation={}",
                ui::inbox::selected_id(&inbox.list),
                inbox
                    .pinned_read
                    .borrow()
                    .as_ref()
                    .map(|entry| (entry.id, entry.read)),
                inbox.refresh_generation.get()
            )
        },
    )?;
    reader
        .controller
        .backend_handle()
        .block_on(repository.merge_changed_page(1, std::slice::from_ref(&selected), &[]))
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    refresh_inbox(&reader.controller, inbox, initial.clone());
    layout()?;
    check(
        inbox.pinned_read.borrow().is_none()
            && ui::inbox::entry_by_id(&inbox.model, 1100).is_some_and(|entry| !entry.read),
        "Remote unread retained stale read state",
    )?;

    // Confirm a real deletion instead of retaining a permanently stale row.
    reader
        .controller
        .backend_handle()
        .block_on(repository.merge_changed_page(1, &[], &[1100]))
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    refresh_inbox(&reader.controller, inbox, updated);
    wait(|| ui::inbox::entry_by_id(&inbox.model, 1100).is_none())
        .map_err(|_| adw::glib::bool_error!("Deleted selected article was not removed"))?;
    check(
        reader.active_id.get() == Some(1100),
        "Remote deletion closed the open reader",
    )?;
    let after_pending = reader
        .controller
        .backend_handle()
        .block_on(repository.pending_mutations(1))
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    check(
        before_pending == after_pending,
        "Automatic inbox updates wrote read/star intentions",
    )?;
    reader.active_id.set(None);
    reader.split.set_show_content(false);
    inbox.model.selection.disconnect(tracking);
    let ids = (1000..1120).collect::<Vec<_>>();
    reader
        .controller
        .backend_handle()
        .block_on(repository.merge_changed_page(1, &[], &ids))
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    Ok(())
}

#[path = "reader_controls_smoke.rs"]
mod reader_controls_smoke;

fn smoke_test_integrated_search(
    window: &adw::ApplicationWindow,
    builder: &gtk::Builder,
    reader: &ReaderUi,
    inbox: &InboxUi,
) -> Result<(), adw::glib::BoolError> {
    fn named(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
        if widget.widget_name() == name {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(found) = named(&widget, name) {
                return Some(found);
            }
            child = widget.next_sibling();
        }
        None
    }
    fn wait_for(condition: impl Fn() -> bool) -> Result<(), adw::glib::BoolError> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !condition() {
            while adw::glib::MainContext::default().iteration(false) {}
            if std::time::Instant::now() > deadline {
                return Err(adw::glib::bool_error!("Search results timed out"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
    let application = adw::Application::builder()
        .application_id("com.nedrichards.brooklet.SearchTest")
        .build();
    install_article_search(
        window,
        &application,
        builder,
        ReadContext {
            controller: reader.controller.clone(),
            reader: reader.clone(),
            inbox: inbox.clone(),
            current: Rc::new(RefCell::new(None)),
            toast: builder.object("toast_overlay").unwrap(),
            undo: Rc::new(RefCell::new(Vec::new())),
        },
    );
    let destinations: adw::ViewStack = builder.object("destinations").unwrap();
    destinations.set_visible_child_name("saved");
    let action = application.lookup_action("search").unwrap();
    action.activate(None);
    let navigation: adw::NavigationView = builder.object("library_navigation").unwrap();
    let page = navigation.visible_page().unwrap();
    if page.tag().as_deref() != Some("article-search")
        || destinations.visible_child_name().as_deref() != Some("library")
    {
        return Err(adw::glib::bool_error!(
            "Search was not integrated into Library navigation"
        ));
    }
    let query = named(page.upcast_ref(), "article-search-query")
        .unwrap()
        .downcast::<gtk::SearchEntry>()
        .unwrap();
    let scope = named(page.upcast_ref(), "article-search-scope")
        .unwrap()
        .downcast::<gtk::DropDown>()
        .unwrap();
    let list = named(page.upcast_ref(), "article-search-results")
        .unwrap()
        .downcast::<gtk::ListView>()
        .unwrap();
    if scope.selected() != 2 {
        return Err(adw::glib::bool_error!(
            "Saved search lost its initial scope"
        ));
    }
    query.set_text("Synthetic");
    scope.set_selected(0);
    wait_for(|| list.model().is_some_and(|model| model.n_items() > 0) && list.is_mapped())?;
    // Filter changes must leave existing results mapped during debounce and
    // backend work, rather than tearing down the list's layout.
    scope.set_selected(1);
    if !list.is_mapped() {
        return Err(adw::glib::bool_error!(
            "Search hid existing results while updating filters"
        ));
    }
    scope.set_selected(0);
    if !navigation.pop() {
        return Err(adw::glib::bool_error!("Search Back failed"));
    }
    action.activate(None);
    if navigation.visible_page().as_ref() != Some(&page) || query.text() != "Synthetic" {
        return Err(adw::glib::bool_error!("Search query was lost after Back"));
    }
    wait_for(|| list.is_mapped())?;
    navigation.pop();
    application.remove_action("search");
    Ok(())
}

fn smoke_test_reader_pipeline(
    keyboard_only: bool,
    search_only: bool,
) -> Result<(), adw::glib::BoolError> {
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
        refresh_generation: Rc::new(Cell::new(0)),
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
        actions: gio::SimpleActionGroup::new(),
        menu: builder.object("reader_menu").unwrap(),
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
    if search_only {
        window.present();
        let result = smoke_test_integrated_search(&window, &builder, &reader, &inbox);
        window.destroy();
        drop(reader);
        drop(inbox);
        drop(window);
        drop(builder);
        let _ = std::fs::remove_dir_all(cache_directory);
        return result;
    }
    install_reader_position_tracking(&reader);
    smoke_test_article_keyboard(
        &window,
        &builder,
        &inbox,
        &reader,
        vec![long.clone(), replacement.clone()],
    )?;
    smoke_test_automatic_inbox(&window, &inbox, &reader, &repository, &replacement)?;
    reader_controls_smoke::run(
        &window,
        &builder,
        &inbox,
        &reader,
        &repository,
        &replacement,
    )?;
    smoke_test_integrated_search(&window, &builder, &reader, &inbox)?;
    if keyboard_only {
        window.destroy();
        drop(reader);
        drop(inbox);
        drop(window);
        drop(builder);
        let _ = std::fs::remove_dir_all(cache_directory);
        return Ok(());
    }
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

pub fn search_test() -> Result<(), adw::glib::BoolError> {
    adw::init()?;
    register_resources();
    smoke_test_reader_pipeline(false, true)
}

pub fn reader_test() -> Result<(), adw::glib::BoolError> {
    adw::init()?;
    register_resources();
    smoke_test_reader_pipeline(false, false)
}

pub fn keyboard_test() -> Result<(), adw::glib::BoolError> {
    adw::init()?;
    register_resources();
    smoke_test_reader_pipeline(true, false)?;
    ui::library::smoke_test()
}

pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    ui::startup::smoke_test()?;
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
    let shortcuts_dialog: adw::ShortcutsDialog = shortcuts
        .object("shortcuts_dialog")
        .expect("shortcuts-dialog.ui must define the shortcuts dialog");
    keyboard::populate_dialog(&shortcuts_dialog);
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
    ui::reconnect::smoke_test()?;
    ui::karakeep::smoke_test()?;
    ui::sync_health::smoke_test()?;
    ui::library::smoke_test()?;
    smoke_test_reader_pipeline(false, false)
}

fn register_resources() {
    gio::resources_register_include!("brooklet.gresource")
        .expect("Brooklet GResources must be valid");
}

/// Exercise the production startup wiring with isolated local data.
pub(crate) fn startup_smoke_application(
    path: std::path::PathBuf,
    images: std::path::PathBuf,
) -> Result<adw::Application, adw::glib::BoolError> {
    register_resources();
    let application = adw::Application::new(
        Some("com.nedrichards.brooklet.StartupProductionTest"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    application
        .register(None::<&gio::Cancellable>)
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    BrookletApplication::install_startup(&application, path, images);
    Ok(application)
}

/// Isolated full-window journey using the production action and lifecycle wiring.
pub(crate) fn library_smoke_application(
    controller: Arc<AppController>,
) -> Result<adw::Application, adw::glib::BoolError> {
    let application = adw::Application::new(
        Some("com.nedrichards.brooklet.LibraryTest"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    application
        .register(None::<&gio::Cancellable>)
        .map_err(|error| adw::glib::bool_error!("{error}"))?;
    let app = BrookletApplication {
        application: application.clone(),
        controller,
    };
    app.install_actions();
    app.connect_lifecycle();
    Ok(application)
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
                let expected = key == Key::question || extra.contains(Modifiers::SHIFT_MASK);
                assert_eq!(
                    opens_shortcuts(key, Modifiers::CONTROL_MASK | extra),
                    expected
                );
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
