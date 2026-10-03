//! Dynamic Library pages live for their navigation-stack lifetime and reload in place.
use super::inbox::{self, InboxModel};
use adw::prelude::*;
use brooklet::{
    controller::AppController,
    error::BrookletError,
    model::{Entry, Feed},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
};

type Completion<T> = Box<dyn FnOnce(Result<Vec<T>, BrookletError>)>;
type Loader<T> = Rc<dyn Fn(i64, Completion<T>)>;
struct Loaders {
    feeds: Loader<Feed>,
    entries: Loader<Entry>,
    error: Rc<dyn Fn(BrookletError)>,
}

enum Contents {
    Category { id: i64, list: gtk::ListBox },
    Feed { id: i64, model: InboxModel },
}
struct Page {
    page: adw::NavigationPage,
    contents: Contents,
    status: adw::StatusPage,
    scroller: gtk::ScrolledWindow,
    generation: Cell<u64>,
}

pub struct LibraryPages {
    navigation: adw::glib::WeakRef<adw::NavigationView>,
    pages: RefCell<Vec<Rc<Page>>>,
    loaders: Loaders,
}
impl LibraryPages {
    pub fn new(
        navigation: &adw::NavigationView,
        controller: Arc<AppController>,
        toast: &adw::ToastOverlay,
    ) -> Rc<Self> {
        let feeds_controller = controller.clone();
        let toast = toast.downgrade();
        Self::with_loaders(
            navigation,
            Loaders {
                feeds: Rc::new(move |id, complete| {
                    feeds_controller.feeds_cached(Some(id), complete)
                }),
                entries: Rc::new(move |id, complete| {
                    controller.entries_for_view(format!("feed:{id}"), complete)
                }),
                error: Rc::new(move |error| {
                    if let Some(toast) = toast.upgrade() {
                        toast.add_toast(adw::Toast::new(&error.sync_message()));
                    }
                }),
            },
        )
    }
    fn with_loaders(navigation: &adw::NavigationView, loaders: Loaders) -> Rc<Self> {
        let state = Rc::new(Self {
            navigation: navigation.downgrade(),
            pages: RefCell::new(Vec::new()),
            loaders,
        });
        navigation.connect_popped({
            let state = Rc::downgrade(&state);
            move |_, popped| {
                if let Some(state) = state.upgrade() {
                    state.pages.borrow_mut().retain(|page| page.page != *popped);
                }
            }
        });
        navigation.connect_destroy({
            let state = Rc::downgrade(&state);
            move |_| {
                if let Some(state) = state.upgrade() {
                    state.clear();
                }
            }
        });
        state
    }
    pub fn clear(&self) {
        self.pages.borrow_mut().clear();
    }

    fn push(
        &self,
        contents: Contents,
        child: &impl IsA<gtk::Widget>,
        title: &str,
        empty_title: &str,
        description: &str,
    ) {
        let Some(navigation) = self.navigation.upgrade() else {
            return;
        };
        let status = adw::StatusPage::builder()
            .title("Loading…")
            .description(description)
            .icon_name("folder-symbolic")
            .vexpand(true)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(child)
            .vexpand(true)
            .visible(false)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&status);
        body.append(&scroller);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&body));
        let page = Rc::new(Page {
            page: adw::NavigationPage::new(&toolbar, title),
            contents,
            status,
            scroller,
            generation: Cell::new(0),
        });
        page.status.set_title(empty_title);
        self.pages.borrow_mut().push(page.clone());
        navigation.push(&page.page);
        self.reload(&page);
    }
    pub fn open_category(&self, id: i64) {
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        self.push(
            Contents::Category {
                id,
                list: list.clone(),
            },
            &list,
            "Feeds",
            "No feeds in this category",
            "Feeds added in Miniflux will appear after sync.",
        );
    }
    pub fn open_feed(
        &self,
        id: i64,
        open_id: Rc<Cell<Option<i64>>>,
    ) -> (gtk::ListView, InboxModel) {
        let list = gtk::ListView::new(None::<gtk::SelectionModel>, None::<gtk::ListItemFactory>);
        let model = inbox::configure_with_action(&list, false, open_id);
        self.push(
            Contents::Feed {
                id,
                model: model.clone(),
            },
            &list,
            "Feed articles",
            "No cached articles",
            "Articles from this feed will appear after sync.",
        );
        (list, model)
    }
    pub fn refresh_visible(&self) {
        let Some(navigation) = self.navigation.upgrade() else {
            return;
        };
        let visible = navigation.visible_page();
        let page = self
            .pages
            .borrow()
            .iter()
            .find(|page| Some(&page.page) == visible.as_ref())
            .cloned();
        if let Some(page) = page
            && page.page.is_mapped()
        {
            self.reload(&page);
        }
    }
    fn reload(&self, page: &Rc<Page>) {
        let token = page.generation.get().wrapping_add(1);
        page.generation.set(token);
        let weak = Rc::downgrade(page);
        let navigation = self.navigation.clone();
        let error = self.loaders.error.clone();
        match &page.contents {
            Contents::Category { id, .. } => (self.loaders.feeds)(
                *id,
                Box::new(move |result| {
                    let Some(page) = current_page(&weak, &navigation, token) else {
                        return;
                    };
                    match result {
                        Ok(feeds) => replace_feeds(&page, feeds),
                        Err(failure) => error(failure),
                    }
                }),
            ),
            Contents::Feed { id, .. } => (self.loaders.entries)(
                *id,
                Box::new(move |result| {
                    let Some(page) = current_page(&weak, &navigation, token) else {
                        return;
                    };
                    if let Contents::Feed { model, .. } = &page.contents {
                        match result {
                            Ok(entries) => crate::application::replace_view_entries(
                                model,
                                &page.status,
                                &page.scroller,
                                entries,
                            ),
                            Err(failure) => error(failure),
                        }
                    }
                }),
            ),
        }
    }
}
fn current_page(
    weak: &std::rc::Weak<Page>,
    navigation: &adw::glib::WeakRef<adw::NavigationView>,
    token: u64,
) -> Option<Rc<Page>> {
    let page = weak.upgrade()?;
    (page.generation.get() == token
        && navigation.upgrade()?.visible_page().as_ref() == Some(&page.page))
    .then_some(page)
}

fn feed_rows(list: &gtk::ListBox) -> Vec<adw::ActionRow> {
    let mut rows = Vec::new();
    let mut child = list.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(row) = widget.downcast::<adw::ActionRow>() {
            rows.push(row);
        }
    }
    rows
}
fn feed_id(row: &adw::ActionRow) -> Option<i64> {
    row.action_target_value().and_then(|value| value.get())
}
fn replace_feeds(page: &Rc<Page>, feeds: Vec<Feed>) {
    let Contents::Category { list, .. } = &page.contents else {
        return;
    };
    let old = feed_rows(list);
    let focus = list.root().and_then(|root| root.focus());
    let focused_id = old
        .iter()
        .find(|row| {
            focus.as_ref().is_some_and(|focus| {
                focus == row.upcast_ref::<gtk::Widget>() || focus.is_ancestor(*row)
            })
        })
        .and_then(feed_id);
    let value = page.scroller.vadjustment().value();
    let anchor = old.iter().find_map(|row| {
        let y = row
            .compute_point(&page.scroller, &gtk::graphene::Point::new(0.0, 0.0))?
            .y();
        (f64::from(y) + f64::from(row.height()) > 0.0)
            .then(|| Some((feed_id(row)?, f64::from(y))))
            .flatten()
    });
    let same_order = old
        .iter()
        .filter_map(feed_id)
        .eq(feeds.iter().map(|feed| feed.id));
    let mut existing = old
        .iter()
        .filter_map(|row| Some((feed_id(row)?, row.clone())))
        .collect::<HashMap<_, _>>();
    if !same_order {
        for row in &old {
            list.remove(row);
        }
    }
    for feed in &feeds {
        let row = existing.remove(&feed.id).unwrap_or_else(|| {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .activatable(true)
                .build();
            row.set_action_name(Some("win.library-feed"));
            row.set_action_target_value(Some(&feed.id.to_variant()));
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            row
        });
        row.set_title(&feed.title);
        if !same_order {
            list.append(&row);
        }
    }
    let empty = feeds.is_empty();
    page.status.set_visible(empty);
    page.scroller.set_visible(!empty);
    if let Some(id) = focused_id
        && let Some(row) = feed_rows(list).iter().find(|row| feed_id(row) == Some(id))
    {
        row.grab_focus();
    }
    let weak = Rc::downgrade(page);
    let token = page.generation.get();
    let frames = Cell::new(0);
    page.scroller.add_tick_callback(move |_, _| {
        if frames.replace(1) == 0 {
            return adw::glib::ControlFlow::Continue;
        }
        if let Some(page) = weak.upgrade()
            && page.generation.get() == token
        {
            let Contents::Category { list, .. } = &page.contents else {
                return adw::glib::ControlFlow::Break;
            };
            let adjustment = page.scroller.vadjustment();
            let target = anchor
                .and_then(|(id, y)| {
                    feed_rows(list)
                        .iter()
                        .find(|row| feed_id(row) == Some(id))
                        .and_then(|row| {
                            row.compute_point(&page.scroller, &gtk::graphene::Point::new(0.0, 0.0))
                        })
                        .map(|point| adjustment.value() + f64::from(point.y()) - y)
                })
                .unwrap_or(value);
            adjustment.set_value(target.clamp(
                adjustment.lower(),
                (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
            ));
        }
        adw::glib::ControlFlow::Break
    });
}

#[path = "library_smoke.rs"]
mod smoke;
pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    smoke::run()
}
