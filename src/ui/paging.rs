//! Demand-driven summary prefixes. Unvisited rows never become EntryObjects.
use adw::{gio, prelude::*};
use brooklet::{
    controller::AppController,
    error::BrookletError,
    model::{Entry, EntryCursor, EntryPage, SUMMARY_PAGE_SIZE},
    reload::ReloadQueue,
};
use std::{cell::Cell, rc::Rc, sync::Arc};

type Completion = Box<dyn FnOnce(Result<EntryPage, BrookletError>)>;
type Loader = Rc<dyn Fn(Option<EntryCursor>, usize, Completion)>;

pub struct PagedList {
    pub view: String,
    load: Loader,
    store: adw::glib::WeakRef<gio::ListStore>,
    scroller: adw::glib::WeakRef<gtk::ScrolledWindow>,
    apply: Rc<dyn Fn(Vec<Entry>, bool)>,
    error: Rc<dyn Fn(BrookletError)>,
    queue: Rc<ReloadQueue>,
    generation: Cell<u64>,
    loading: Cell<bool>,
    next: Cell<Option<EntryCursor>>,
}
impl PagedList {
    pub fn install(
        model: &super::inbox::InboxModel,
        scroller: &gtk::ScrolledWindow,
        controller: Arc<AppController>,
        view: String,
        apply: impl Fn(Vec<Entry>, bool) + 'static,
        error: impl Fn(BrookletError) + 'static,
    ) -> Rc<Self> {
        let query_view = view.clone();
        let load = Rc::new(move |after, limit, complete: Completion| {
            controller.entries_page(query_view.clone(), after, limit, complete);
        });
        Self::with_loader(model, scroller, view, load, Rc::new(apply), Rc::new(error))
    }
    fn with_loader(
        model: &super::inbox::InboxModel,
        scroller: &gtk::ScrolledWindow,
        view: String,
        load: Loader,
        apply: Rc<dyn Fn(Vec<Entry>, bool)>,
        error: Rc<dyn Fn(BrookletError)>,
    ) -> Rc<Self> {
        let pager = Rc::new(Self {
            view,
            load,
            store: model.store.downgrade(),
            scroller: scroller.downgrade(),
            apply,
            error,
            queue: Rc::default(),
            generation: Cell::new(0),
            loading: Cell::new(false),
            next: Cell::new(None),
        });
        *model.pager.borrow_mut() = Some(pager.clone());
        let adjustment = scroller.vadjustment();
        adjustment.connect_value_changed({
            let weak = Rc::downgrade(&pager);
            move |_| {
                if let Some(pager) = weak.upgrade() {
                    pager.more();
                }
            }
        });
        adjustment.connect_changed({
            let weak = Rc::downgrade(&pager);
            move |_| {
                if let Some(pager) = weak.upgrade() {
                    pager.more();
                }
            }
        });
        model.selection.connect_selected_notify({
            let weak = Rc::downgrade(&pager);
            move |selection| {
                if let Some(pager) = weak.upgrade()
                    && selection.selected() != gtk::INVALID_LIST_POSITION
                    && selection.selected().saturating_add(32) >= selection.n_items()
                {
                    pager.load_next();
                }
            }
        });
        model.store.connect_items_changed({
            let weak = Rc::downgrade(&pager);
            move |store, _, _, _| {
                if store.n_items() < SUMMARY_PAGE_SIZE as u32 {
                    let weak = weak.clone();
                    adw::glib::idle_add_local_once(move || {
                        if let Some(pager) = weak.upgrade()
                            && pager
                                .scroller
                                .upgrade()
                                .and_then(|s| s.parent())
                                .is_some_and(|p| p.is_mapped())
                        {
                            pager.load_next();
                        }
                    });
                }
            }
        });
        pager
    }
    /// Accept sync's first page directly unless a deeper prefix is already visible.
    pub fn snapshot(self: &Rc<Self>, page: EntryPage) {
        if self
            .store
            .upgrade()
            .is_some_and(|store| store.n_items() as usize > SUMMARY_PAGE_SIZE)
        {
            self.reload();
            return;
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        self.loading.set(false);
        self.next.set(page.next);
        (self.apply)(page.entries, false);
        self.more_later();
    }
    pub fn clear(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.next.set(None);
        // A queued reload observes the new generation and does no work.
        self.loading.set(false);
    }
    pub fn reload(self: &Rc<Self>) {
        let token = self.generation.get().wrapping_add(1);
        self.generation.set(token);
        self.loading.set(true);
        let weak = Rc::downgrade(self);
        self.queue.request(move |done| {
            let Some(pager) = weak.upgrade() else {
                done();
                return;
            };
            if pager.generation.get() != token {
                done();
                return;
            }
            let limit = pager.store.upgrade().map_or(SUMMARY_PAGE_SIZE, |store| {
                (store.n_items() as usize).max(SUMMARY_PAGE_SIZE)
            });
            let load = pager.load.clone();
            load(
                None,
                limit,
                Box::new(move |result| {
                    if let Some(pager) = weak.upgrade()
                        && pager.generation.get() == token
                    {
                        pager.loading.set(false);
                        match result {
                            Ok(page) => {
                                pager.next.set(page.next);
                                (pager.apply)(page.entries, false);
                                pager.more_later();
                            }
                            Err(error) => (pager.error)(error),
                        }
                    }
                    done();
                }),
            );
        });
    }
    fn more_later(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        adw::glib::idle_add_local_once(move || {
            if let Some(pager) = weak.upgrade() {
                pager.more();
            }
        });
    }
    fn more(self: &Rc<Self>) {
        let Some(scroller) = self.scroller.upgrade() else {
            return;
        };
        if !scroller.is_mapped() {
            return;
        }
        let a = scroller.vadjustment();
        if a.page_size() > 0.0 && a.value() + 2.0 * a.page_size() >= a.upper() {
            self.load_next();
        }
    }
    fn load_next(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        let Some(cursor) = self.next.get() else {
            return;
        };
        self.loading.set(true);
        let token = self.generation.get();
        let weak = Rc::downgrade(self);
        self.queue.request(move |done| {
            let Some(pager) = weak.upgrade() else {
                done();
                return;
            };
            if pager.generation.get() != token {
                done();
                return;
            }
            let load = pager.load.clone();
            load(
                Some(cursor),
                SUMMARY_PAGE_SIZE,
                Box::new(move |result| {
                    if let Some(pager) = weak.upgrade()
                        && pager.generation.get() == token
                    {
                        pager.loading.set(false);
                        match result {
                            Ok(page) => {
                                pager.next.set(page.next);
                                (pager.apply)(page.entries, true);
                                pager.more_later();
                            }
                            Err(error) => (pager.error)(error),
                        }
                    }
                    done();
                }),
            );
        });
    }
}

/// Deterministic GTK model regression with delayed, injectable database completions.
pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    use super::inbox::{self, InboxModel};
    use std::{cell::RefCell, collections::VecDeque};
    type Request = (Option<EntryCursor>, usize, Completion);
    let requests = Rc::new(RefCell::new(VecDeque::<Request>::new()));
    let list = gtk::ListView::new(None::<gtk::SelectionModel>, None::<gtk::ListItemFactory>);
    let model = inbox::configure_with_action(&list, false, Rc::new(Cell::new(None)));
    let scroller = gtk::ScrolledWindow::new();
    let store = model.store.clone();
    let selection = model.selection.clone();
    let pager = PagedList::with_loader(
        &model,
        &scroller,
        "all".into(),
        {
            let requests = requests.clone();
            Rc::new(move |after, limit, done| requests.borrow_mut().push_back((after, limit, done)))
        },
        Rc::new(move |entries, append| {
            let model = InboxModel {
                store: store.clone(),
                selection: selection.clone(),
                pager: Rc::default(),
            };
            if append {
                inbox::append_page(&model, entries);
            } else {
                inbox::replace(&model, entries);
            }
        }),
        Rc::new(|error| panic!("Unexpected paging error: {error}")),
    );
    let entry = |id| Entry {
        id,
        account_id: 1,
        feed_id: 7,
        feed_title: "Feed".into(),
        category_title: "Category".into(),
        title: format!("Article {id}"),
        url: format!("https://example.com/{id}"),
        author: None,
        published_at_ms: id,
        html: String::new(),
        content_revision: 0,
        read: false,
        starred: false,
        reading_minutes: 1,
        delivery_state: None,
        delivery_error: None,
    };
    let page = |first: i64| EntryPage {
        entries: (first - 127..=first).rev().map(&entry).collect(),
        next: Some(EntryCursor {
            published_at_ms: first - 127,
            id: first - 127,
        }),
    };
    pager.reload();
    for _ in 0..20 {
        pager.reload();
    }
    assert_eq!(requests.borrow().len(), 1);
    let (_, limit, done) = requests.borrow_mut().pop_front().unwrap();
    assert_eq!(limit, SUMMARY_PAGE_SIZE);
    done(Ok(page(5000)));
    assert_eq!(model.store.n_items(), 0, "Obsolete initial page applied");
    let (_, _, done) = requests.borrow_mut().pop_front().unwrap();
    done(Ok(page(5000)));
    assert_eq!(model.store.n_items(), 128);
    assert!(
        requests.borrow().is_empty(),
        "Unvisited rows loaded eagerly"
    );
    model.selection.set_selected(120);
    assert_eq!(
        requests.borrow().len(),
        1,
        "Keyboard cursor did not prefetch"
    );
    pager.load_next();
    assert_eq!(requests.borrow().len(), 1, "Duplicate page request");
    let (after, _, done) = requests.borrow_mut().pop_front().unwrap();
    assert_eq!(after.unwrap().id, 4873);
    pager.reload();
    done(Ok(page(4872)));
    assert_eq!(
        model.store.n_items(),
        128,
        "Obsolete append applied after refresh"
    );
    let (_, _, done) = requests.borrow_mut().pop_front().unwrap();
    done(Ok(page(5001)));
    // A selection near the boundary may immediately queue the next page.
    if requests.borrow().is_empty() {
        pager.load_next();
    }
    let (_, _, done) = requests.borrow_mut().pop_front().unwrap();
    done(Ok(page(4873)));
    assert_eq!(model.store.n_items(), 256);
    assert_eq!(inbox::selected_from_list(&list).unwrap().id, 4880);
    assert!((0..model.store.n_items()).all(|i| !inbox::entry_at(&model, i).unwrap().read));
    pager.reload();
    let (_, limit, done) = requests.borrow_mut().pop_front().unwrap();
    assert_eq!(limit, 256, "Refresh forgot visited prefix");
    pager.clear();
    done(Ok(page(6000)));
    assert_eq!(
        model.store.n_items(),
        256,
        "Clear admitted an obsolete completion"
    );
    let weak = Rc::downgrade(&pager);
    model.pager.borrow_mut().take();
    drop(pager);
    assert!(
        weak.upgrade().is_none(),
        "Pager leaked after model teardown"
    );
    Ok(())
}
