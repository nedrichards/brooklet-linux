use super::*;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

type Pending<T> = Rc<RefCell<VecDeque<(i64, Completion<T>)>>>;

fn layout() {
    let deadline = Instant::now() + Duration::from_millis(350);
    while Instant::now() < deadline {
        while adw::glib::MainContext::default().pending() {
            adw::glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn story(id: i64) -> Entry {
    Entry {
        id,
        account_id: 1,
        feed_id: 21,
        feed_title: "Feed".into(),
        category_title: "Category".into(),
        title: format!("Story {id}"),
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
    }
}
fn feeds() -> Vec<Feed> {
    (1..=35)
        .map(|id| Feed {
            id,
            category_id: 7,
            title: format!("Feed {id}"),
            site_url: "https://example.com".into(),
            feed_url: format!("https://example.com/{id}.xml"),
            parsing_error_message: String::new(),
            parsing_error_count: 0,
            disabled: false,
        })
        .collect()
}
fn focused_inside(widget: &impl IsA<gtk::Widget>) -> bool {
    let widget = widget.as_ref();
    widget
        .root()
        .and_then(|root| root.focus())
        .is_some_and(|focus| focus == *widget || focus.is_ancestor(widget))
}

fn article_anchor(scroller: &gtk::ScrolledWindow) -> Option<(i64, f64)> {
    let mut picked = scroller.pick(24.0, 8.0, gtk::PickFlags::DEFAULT);
    while let Some(widget) = picked {
        if widget.has_css_class("article-row") {
            let id = widget
                .widget_name()
                .strip_prefix("article-")?
                .parse()
                .ok()?;
            let y = widget
                .compute_point(scroller, &gtk::graphene::Point::new(0.0, 0.0))?
                .y();
            return Some((id, f64::from(y)));
        }
        picked = widget.parent();
    }
    None
}
fn category_anchor(page: &Page) -> Option<(i64, f64)> {
    let Contents::Category { list, .. } = &page.contents else {
        return None;
    };
    feed_rows(list).iter().find_map(|row| {
        let y = row
            .compute_point(&page.scroller, &gtk::graphene::Point::new(0.0, 0.0))?
            .y();
        (y + row.height() as f32 > 0.0)
            .then(|| Some((feed_id(row)?, f64::from(y))))
            .flatten()
    })
}
fn check(condition: bool, message: &str) -> Result<(), adw::glib::BoolError> {
    if condition {
        Ok(())
    } else {
        Err(adw::glib::bool_error!("{message}"))
    }
}

pub fn run() -> Result<(), adw::glib::BoolError> {
    let entries: Pending<Entry> = Rc::new(RefCell::new(VecDeque::new()));
    let categories: Pending<Feed> = Rc::new(RefCell::new(VecDeque::new()));
    let errors = Rc::new(Cell::new(0));
    let navigation = adw::NavigationView::new();
    navigation.add(&adw::NavigationPage::new(
        &gtk::Label::new(Some("Library")),
        "Library",
    ));
    let pages = LibraryPages::with_loaders(
        &navigation,
        Loaders {
            entries: Rc::new({
                let entries = entries.clone();
                move |id, complete| entries.borrow_mut().push_back((id, complete))
            }),
            feeds: Rc::new({
                let categories = categories.clone();
                move |id, complete| categories.borrow_mut().push_back((id, complete))
            }),
            error: Rc::new({
                let errors = errors.clone();
                move |_| errors.set(errors.get() + 1)
            }),
        },
    );
    let destinations = gtk::Stack::new();
    // This fixture checks the settled viewport. Transition frames can temporarily
    // hide the page from picking even after the response has been applied.
    navigation.set_animate_transitions(false);
    destinations.add_named(&navigation, Some("library"));
    destinations.add_named(&gtk::Label::new(Some("Other destination")), Some("other"));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&destinations);
    let outside = gtk::Button::with_label("Outside list");
    body.append(&outside);
    let window = gtk::Window::builder()
        .default_width(640)
        .default_height(480)
        .child(&body)
        .build();
    let actions = adw::gio::SimpleActionGroup::new();
    actions.add_action(&adw::gio::SimpleAction::new(
        "library-feed",
        Some(&i64::static_variant_type()),
    ));
    window.insert_action_group("win", Some(&actions));
    window.present();
    layout();

    pages.open_category(7);
    layout();
    let (id, complete) = categories.borrow_mut().pop_front().unwrap();
    check(id == 7, "Wrong category scope")?;
    complete(Ok(feeds()));
    layout();
    let category = pages.pages.borrow()[0].clone();
    let Contents::Category { list, .. } = &category.contents else {
        unreachable!()
    };
    let row = feed_rows(list)[19].clone();
    row.grab_focus();
    layout();
    category.scroller.vadjustment().set_value(800.0);
    layout();
    let before = category_anchor(&category)
        .ok_or_else(|| adw::glib::bool_error!("Missing category viewport anchor"))?;
    let mut changed = feeds();
    changed[19].title = "Renamed feed".into();
    changed[19].parsing_error_message = "HTTP 503 <unavailable>".into();
    changed[19].parsing_error_count = 3;
    changed[19].disabled = true;
    changed[0].parsing_error_count = 1;
    changed[1].disabled = true;
    let mut inserted = changed[0].clone();
    inserted.id = 999;
    inserted.title = "New feed".into();
    changed.insert(0, inserted);
    pages.refresh_visible();
    categories.borrow_mut().pop_front().unwrap().1(Ok(changed));
    layout();
    let after = category_anchor(&category).unwrap();
    check(
        row.subtitle().as_deref()
            == Some("Updates disabled in Miniflux · Update failed: HTTP 503 <unavailable>")
            && !row.uses_markup()
            && row.is_activatable(),
        "Feed failure status or cached navigation missing",
    )?;
    check(
        row.tooltip_text().as_deref() == row.subtitle().as_deref(),
        "Feed failure detail missing",
    )?;
    let rows = feed_rows(list);
    check(
        rows[0].subtitle().as_deref() == Some("Update failed on the server")
            && rows[2].subtitle().as_deref() == Some("Updates disabled in Miniflux")
            && rows[3].subtitle().as_deref().is_none_or(str::is_empty),
        "Feed fallback or healthy status incorrect",
    )?;

    check(
        before.0 == after.0 && (before.1 - after.1).abs() < 2.0,
        "Category refresh lost viewport anchor",
    )?;
    check(
        row.title() == "Renamed feed" && focused_inside(&row),
        &format!(
            "Category refresh lost feed focus or rename: title={} has_focus={} is_focus={} child={:?} root={:?}",
            row.title(),
            row.has_focus(),
            row.is_focus(),
            row.focus_child().map(|widget| widget.type_().name()),
            row.root()
                .and_then(|root| root.focus())
                .map(|widget| widget.type_().name())
        ),
    )?;
    check(
        navigation.visible_page().as_ref() == Some(&category.page),
        "Category refresh navigated away",
    )?;

    let mut recovered = feeds();
    recovered[19].title = "Renamed feed".into();
    pages.refresh_visible();
    categories.borrow_mut().pop_front().unwrap().1(Ok(recovered));
    layout();
    check(
        row.subtitle().as_deref().is_none_or(str::is_empty) && row.tooltip_text().is_none(),
        "Recovered feed retained stale error",
    )?;

    let (list, model) = pages.open_feed(21, Rc::new(Cell::new(None)));
    layout();
    let (id, complete) = entries.borrow_mut().pop_front().unwrap();
    check(id == 21, "Wrong feed scope")?;
    let initial = (1..=100).rev().map(story).collect::<Vec<_>>();
    complete(Ok(initial.clone()));
    layout();
    let feed = pages.pages.borrow()[1].clone();
    inbox::select_id(&list, 80);
    list.grab_focus();
    layout();
    feed.scroller.vadjustment().set_value(900.0);
    layout();
    let before = article_anchor(&feed.scroller)
        .ok_or_else(|| adw::glib::bool_error!("Missing article viewport anchor"))?;
    let mut changed = initial.clone();
    changed[20].title = "Updated article".into();
    changed[20].starred = true;
    changed.insert(0, story(101));
    pages.refresh_visible();
    entries.borrow_mut().pop_front().unwrap().1(Ok(changed));
    layout();
    let after = article_anchor(&feed.scroller).unwrap();
    check(
        before.0 == after.0 && (before.1 - after.1).abs() < 2.0,
        "Feed refresh lost viewport anchor",
    )?;
    check(
        inbox::selected_id(&list) == Some(80)
            && inbox::entry_by_id(&model, 80).is_some_and(|entry| {
                entry.title == "Updated article" && entry.starred && !entry.read
            }),
        "Feed refresh lost selection, updates or unread state",
    )?;
    check(focused_inside(&list), "Feed refresh lost list focus")?;
    check(
        navigation.visible_page().as_ref() == Some(&feed.page),
        "Feed refresh changed navigation page",
    )?;

    // Overlapping refreshes must apply only the newest response.
    pages.refresh_visible();
    pages.refresh_visible();
    let (_, old) = entries.borrow_mut().pop_front().unwrap();
    let (_, latest) = entries.borrow_mut().pop_front().unwrap();
    let mut updated = initial.clone();
    updated[20].title = "Latest response".into();
    latest(Ok(updated));
    old(Ok(initial.clone()));
    layout();
    check(
        inbox::entry_by_id(&model, 80).unwrap().title == "Latest response",
        "Stale feed response overwrote newest contents",
    )?;
    outside.grab_focus();
    pages.refresh_visible();
    entries.borrow_mut().pop_front().unwrap().1(Ok(initial.clone()));
    layout();
    check(
        focused_inside(&outside),
        "Refresh stole focus from another control",
    )?;
    pages.refresh_visible();
    entries.borrow_mut().pop_front().unwrap().1(Err(BrookletError::InvalidSetup("test failure")));
    check(
        errors.get() == 1 && model.store.n_items() == 100,
        "Failed refresh discarded cached articles",
    )?;

    // Navigation while fetching must not update the old page or resurrect a popped page.
    pages.refresh_visible();
    let (_, late) = entries.borrow_mut().pop_front().unwrap();
    let (other_list, other_model) = pages.open_feed(22, Rc::new(Cell::new(None)));
    layout();
    let (_, other_complete) = entries.borrow_mut().pop_front().unwrap();
    late(Ok(vec![story(300)]));
    check(
        model.store.n_items() == 100,
        "Hidden feed accepted a late response",
    )?;
    let weak_other = other_list.downgrade();
    drop(other_list);
    drop(other_model);
    navigation.pop();
    layout();
    other_complete(Ok(vec![story(400)]));
    check(
        pages.pages.borrow().len() == 2,
        "Popped feed retained its refresh state",
    )?;
    pages.refresh_visible();
    let (id, complete) = entries.borrow_mut().pop_front().unwrap();
    check(id == 21, "Back refreshed the wrong feed")?;
    complete(Ok(Vec::new()));
    layout();
    check(
        feed.status.is_visible() && !feed.scroller.is_visible(),
        "Feed did not transition to empty",
    )?;
    pages.refresh_visible();
    entries.borrow_mut().pop_front().unwrap().1(Ok(vec![story(80)]));
    layout();
    check(
        !feed.status.is_visible() && feed.scroller.is_visible(),
        "Empty feed did not receive articles",
    )?;
    check(weak_other.upgrade().is_none(), "Popped feed list leaked")?;

    destinations.set_visible_child_name("other");
    layout();
    pages.refresh_visible();
    check(
        entries.borrow().is_empty(),
        "Hidden Library refreshed unnecessarily",
    )?;
    destinations.set_visible_child_name("library");
    layout();
    navigation.pop();
    layout();
    pages.refresh_visible();
    categories.borrow_mut().pop_front().unwrap().1(Ok(Vec::new()));
    layout();
    check(
        category.status.is_visible() && !category.scroller.is_visible(),
        "Category did not transition to empty",
    )?;
    pages.refresh_visible();
    categories.borrow_mut().pop_front().unwrap().1(Ok(feeds()));
    layout();
    check(
        !category.status.is_visible() && category.scroller.is_visible(),
        "Empty category did not receive feeds",
    )?;
    pages.refresh_visible();
    let (_, after_close) = categories.borrow_mut().pop_front().unwrap();
    let weak_category = Rc::downgrade(&category);
    drop(category);
    drop(feed);
    pages.clear();
    after_close(Ok(feeds()));
    check(
        weak_category.upgrade().is_none(),
        "Cleared Library retained its page state",
    )?;
    window.close();
    layout();
    sync_journey(false)?;
    sync_journey(true)
}

/// Follow the production navigation and Sync actions with real controller/storage.
fn sync_journey(refresh: bool) -> Result<(), adw::glib::BoolError> {
    use async_trait::async_trait;
    use brooklet::{
        api::miniflux::{CategoryDto, EntriesDto, EntryDto, EntryQuery, FeedDto, ServerIdentity},
        model::Account,
        services::traits::{MinifluxApi, Repository, SecretStore},
        setup::{AccountSetupService, MinifluxIdentityValidator},
        storage::sqlite::SqliteRepository,
        sync::{AccountSyncService, MinifluxApiFactory, SyncService},
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Server(Arc<AtomicBool>, Arc<AtomicBool>);
    #[async_trait]
    impl MinifluxApi for Server {
        async fn validate(&self) -> Result<ServerIdentity, BrookletError> {
            unreachable!()
        }
        async fn categories(&self) -> Result<Vec<CategoryDto>, BrookletError> {
            Ok(vec![CategoryDto {
                id: 7,
                title: "Category".into(),
            }])
        }
        async fn feeds(&self) -> Result<Vec<FeedDto>, BrookletError> {
            Ok(vec![FeedDto {
                id: 21,
                title: if self.0.load(Ordering::Acquire) {
                    "Renamed feed"
                } else {
                    "Feed"
                }
                .into(),
                site_url: "https://example.com".into(),
                feed_url: "https://example.com/feed".into(),
                parsing_error_message: String::new(),
                parsing_error_count: 0,
                disabled: false,
                category: Some(CategoryDto {
                    id: 7,
                    title: "Category".into(),
                }),
            }])
        }
        async fn entries(&self, query: &EntryQuery) -> Result<EntriesDto, BrookletError> {
            let updated = self.0.load(Ordering::Acquire);
            let mut ids = if updated {
                vec![101, 99, 42]
            } else {
                vec![99, 42]
            };
            if self.1.load(Ordering::Acquire) {
                ids.retain(|id| *id != 42);
            }
            if query.order == "id" {
                ids.sort_unstable();
            }
            if query.limit == 1 {
                ids.truncate(1);
            }
            Ok(EntriesDto {
                total: ids.len(),
                entries: ids
                    .into_iter()
                    .map(|id| EntryDto {
                        id,
                        feed_id: 21,
                        title: if updated && id == 42 {
                            "Updated article".into()
                        } else {
                            format!("Story {id}")
                        },
                        url: format!("https://example.com/{id}"),
                        author: None,
                        published_at: "2026-10-03T12:00:00Z".into(),
                        changed_at: "2026-10-03T12:00:00Z".into(),
                        content: "<p>Cached article</p>".into(),
                        status: "unread".into(),
                        starred: updated && id == 42,
                        reading_time: 1,
                        feed: None,
                    })
                    .collect(),
            })
        }
        async fn entry_ids(
            &self,
            _: usize,
            _: usize,
        ) -> Result<brooklet::api::miniflux::EntryIdsDto, BrookletError> {
            let entry_ids: Vec<i64> = if self.1.load(Ordering::Acquire) {
                vec![99, 101]
            } else {
                vec![42, 99, 101]
            };
            Ok(brooklet::api::miniflux::EntryIdsDto {
                total: entry_ids.len(),
                entry_ids,
            })
        }
        async fn entry(&self, id: i64) -> Result<EntryDto, BrookletError> {
            assert!(id == 42 && self.1.load(Ordering::Acquire));
            Err(BrookletError::Http {
                status: 404,
                kind: brooklet::model::classify_http_status(404),
            })
        }
        async fn set_read(&self, _: &[i64], _: bool) -> Result<(), BrookletError> {
            unreachable!("Navigation or refresh changed read state")
        }
        async fn set_starred(&self, _: &[i64], _: bool) -> Result<(), BrookletError> {
            unreachable!("Navigation or refresh changed saved state")
        }
        async fn save_to_integration(&self, _: i64) -> Result<(), BrookletError> {
            unreachable!()
        }
        async fn refresh_feeds(&self) -> Result<(), BrookletError> {
            let updated = self.0.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                updated.store(true, Ordering::Release);
            });
            Ok(())
        }
        async fn subscribe(&self, _: &str, _: Option<i64>) -> Result<i64, BrookletError> {
            unreachable!()
        }
    }
    impl MinifluxApiFactory for Server {
        fn create(&self, _: &str, _: String) -> Result<Box<dyn MinifluxApi>, BrookletError> {
            Ok(Box::new(Server(self.0.clone(), self.1.clone())))
        }
    }
    struct Secrets;
    #[async_trait]
    impl SecretStore for Secrets {
        async fn load_miniflux_token(&self, _: i64) -> Result<Option<String>, BrookletError> {
            Ok(Some("fixture".into()))
        }
        async fn store_miniflux_token(&self, _: i64, _: &str) -> Result<(), BrookletError> {
            unreachable!()
        }
        async fn delete_account_secrets(&self, _: i64) -> Result<(), BrookletError> {
            Ok(())
        }
    }
    fn wait_until(test: impl Fn() -> bool) -> Result<(), adw::glib::BoolError> {
        let deadline = Instant::now() + Duration::from_secs(25);
        while !test() && Instant::now() < deadline {
            layout();
        }
        check(test(), "Library Sync journey timed out")
    }
    fn find_list(widget: &gtk::Widget) -> Option<gtk::ListView> {
        if let Some(list) = widget.downcast_ref::<gtk::ListView>() {
            return Some(list.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(list) = find_list(&widget) {
                return Some(list);
            }
            child = widget.next_sibling();
        }
        None
    }
    fn find_feed_row(widget: &gtk::Widget) -> Option<adw::ActionRow> {
        if let Some(row) = widget.downcast_ref::<adw::ActionRow>()
            && row.action_name().as_deref() == Some("win.library-feed")
        {
            return Some(row.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(row) = find_feed_row(&widget) {
                return Some(row);
            }
            child = widget.next_sibling();
        }
        None
    }
    let repo =
        Arc::new(SqliteRepository::open_in_memory().map_err(|e| adw::glib::bool_error!("{e}"))?);
    let runtime = tokio::runtime::Runtime::new().map_err(|e| adw::glib::bool_error!("{e}"))?;
    runtime
        .block_on(repo.save_account(&Account {
            id: 1,
            server_url: "https://miniflux.example".into(),
            username: "reader".into(),
            server_version: "2.3.2".into(),
        }))
        .map_err(|e| adw::glib::bool_error!("{e}"))?;
    let updated = Arc::new(AtomicBool::new(false));
    let deleted = Arc::new(AtomicBool::new(false));
    let secrets = Arc::new(Secrets);
    let service = Arc::new(AccountSyncService::new(
        repo.clone(),
        secrets.clone(),
        Arc::new(Server(updated.clone(), deleted.clone())),
    ));
    runtime
        .block_on(service.sync())
        .map_err(|e| adw::glib::bool_error!("{e}"))?;
    let directory =
        std::env::temp_dir().join(format!("brooklet-library-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|e| adw::glib::bool_error!("{e}"))?;
    let cache = brooklet::services::image_cache::ImageCache::new(directory.join("images.db"), 1024)
        .map_err(|e| adw::glib::bool_error!("{e}"))?;
    let controller = Arc::new(
        AppController::with_image_cache(
            Arc::new(AccountSetupService::new(
                Arc::new(MinifluxIdentityValidator),
                repo.clone(),
                secrets,
            )),
            service,
            cache,
        )
        .map_err(|e| adw::glib::bool_error!("{e}"))?,
    );
    let app = crate::application::library_smoke_application(controller.clone())?;
    app.activate();
    layout();
    let window = app
        .active_window()
        .ok_or_else(|| adw::glib::bool_error!("Missing Library journey window"))?;
    let builder_nav = find_navigation(window.upcast_ref())
        .ok_or_else(|| adw::glib::bool_error!("Missing Library navigation"))?;
    // Switch destinations exactly as the Library sidebar does.
    find_destinations(window.upcast_ref())
        .unwrap()
        .set_visible_child_name("library");
    layout();
    window
        .activate_action("win.library-category", Some(&7_i64.to_variant()))
        .map_err(|e| adw::glib::bool_error!("{e}"))?;
    wait_until(|| {
        builder_nav
            .visible_page()
            .is_some_and(|page| find_feed_row(page.upcast_ref()).is_some())
    })?;
    let category = builder_nav.visible_page().unwrap();
    window
        .activate_action("win.library-feed", Some(&21_i64.to_variant()))
        .map_err(|e| adw::glib::bool_error!("{e}"))?;
    let feed = builder_nav.visible_page().unwrap();
    let list = find_list(feed.upcast_ref()).unwrap();
    wait_until(|| list.model().is_some_and(|model| model.n_items() == 2))?;
    inbox::select_id(&list, 42);
    list.grab_focus();
    if refresh {
        window
            .activate_action("win.refresh-feeds", None)
            .map_err(|e| adw::glib::bool_error!("{e}"))?;
        check(
            !window
                .clone()
                .downcast::<adw::ApplicationWindow>()
                .unwrap()
                .lookup_action("refresh-feeds")
                .unwrap()
                .is_enabled(),
            "Refresh action remained enabled",
        )?;
        window
            .activate_action("win.refresh-feeds", None)
            .map_err(|e| adw::glib::bool_error!("{e}"))?;
    } else {
        updated.store(true, Ordering::Release);
        app.activate_action("sync", None);
    }
    wait_until(|| {
        list.model().is_some_and(|model| model.n_items() == 3)
            && inbox::selected_from_list(&list)
                .is_some_and(|entry| entry.title == "Updated article" && entry.starred)
    })?;
    check(
        window
            .clone()
            .downcast::<adw::ApplicationWindow>()
            .unwrap()
            .lookup_action("refresh-feeds")
            .unwrap()
            .is_enabled(),
        "Refresh action did not recover",
    )?;
    check(
        focused_inside(&list),
        "Production refresh lost feed-list focus",
    )?;
    if let Some(driver) = std::env::var_os("BROOKLET_KEYBOARD_DRIVER") {
        let title = format!(
            "Brooklet Keyboard Regression Library {}",
            std::process::id()
        );
        window.set_title(Some(&title));
        layout();
        for (key, expected) in [("Up", 99), ("Down", 42)] {
            let status = std::process::Command::new("python3")
                .arg(&driver)
                .arg(&title)
                .arg(key)
                .arg("both")
                .status()
                .map_err(|error| adw::glib::bool_error!("Library key driver failed: {error}"))?;
            layout();
            check(
                status.success() && inbox::selected_id(&list) == Some(expected),
                &format!(
                    "Physical {key} after feed refresh: expected={expected} selected={:?} focus={:?} mapped={} driver={}",
                    inbox::selected_id(&list),
                    gtk::prelude::GtkWindowExt::focus(&window).map(|widget| widget.type_().name()),
                    list.is_mapped(),
                    status.success()
                ),
            )?;
        }
    }
    check(
        builder_nav.visible_page().as_ref() == Some(&feed) && inbox::selected_id(&list) == Some(42),
        "Sync changed feed scope or selection",
    )?;
    check(
        runtime
            .block_on(repo.cached_entry(1, 42))
            .unwrap()
            .is_some_and(|entry| !entry.read),
        "Sync/navigation marked article read",
    )?;
    builder_nav.pop();
    wait_until(|| {
        find_feed_row(category.upcast_ref()).is_some_and(|row| row.title() == "Renamed feed")
    })?;
    check(
        builder_nav.visible_page().as_ref() == Some(&category),
        "Back lost category scope",
    )?;
    if refresh {
        fn find_health(widget: &gtk::Widget) -> Option<gtk::Box> {
            if widget.widget_name() == "sync-health" {
                return widget.clone().downcast().ok();
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(view) = find_health(&widget) {
                    return Some(view);
                }
                child = widget.next_sibling();
            }
            None
        }
        let health = find_health(window.upcast_ref()).expect("production health view");
        let headline = health
            .first_child()
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        runtime
            .block_on(repo.record_sync_error(1, "HTTP 401", 0))
            .unwrap();
        runtime
            .block_on(repo.record_delivery_error(1, Some("HTTP 503")))
            .unwrap();
        wait_until(|| headline.text() == "Sync needs attention")?;
        check(
            health.property::<bool>("visible"),
            "Failure did not reveal health controls",
        )?;
        check(
            builder_nav.visible_page().as_ref() == Some(&category),
            "Health update changed Library scope",
        )?;
        let window_actions = window.clone().downcast::<adw::ApplicationWindow>().unwrap();
        for (action, title) in [
            ("win.reconnect", "Reconnect to Miniflux"),
            ("win.delivery-review", "Karakeep deliveries"),
        ] {
            let menu = health
                .last_child()
                .unwrap()
                .downcast::<gtk::MenuButton>()
                .unwrap();
            menu.popup();
            layout();
            let body = menu.popover().unwrap().child().unwrap();
            let mut child = body.first_child();
            let mut clicked = false;
            while let Some(widget) = child {
                if let Some(button) = widget.downcast_ref::<gtk::Button>()
                    && button.action_name().as_deref() == Some(action)
                {
                    button.emit_clicked();
                    clicked = true;
                    break;
                }
                child = widget.next_sibling();
            }
            check(clicked, "Health recovery button was missing")?;
            wait_until(|| window_actions.visible_dialog().is_some())?;
            let dialog = window_actions.visible_dialog().unwrap();
            check(
                dialog.title().as_str() == title,
                "Health recovery action opened the wrong dialog",
            )?;
            dialog.close();
            wait_until(|| window_actions.visible_dialog().is_none())?;
        }
        runtime
            .block_on(repo.record_delivery_error(1, None))
            .unwrap();
        runtime.block_on(repo.complete_sync(1, 1, 1)).unwrap();
        wait_until(|| headline.text().starts_with("Last sync"))?;
        check(
            !health.property::<bool>("visible"),
            "Recovered health remained prominent",
        )?;
    }
    if refresh {
        window
            .activate_action("win.library-feed", Some(&21_i64.to_variant()))
            .map_err(|e| adw::glib::bool_error!("{e}"))?;
        let page = builder_nav.visible_page().unwrap();
        let list = find_list(page.upcast_ref()).unwrap();
        wait_until(|| list.model().is_some_and(|model| model.n_items() == 3))?;
        inbox::select_id(&list, 99);
        list.grab_focus();
        deleted.store(true, Ordering::Release);
        app.activate_action("sync", None);
        wait_until(|| list.model().is_some_and(|model| model.n_items() == 2))?;
        check(
            inbox::selected_id(&list) == Some(99)
                && builder_nav.visible_page().as_ref() == Some(&page),
            "Remote deletion changed surviving selection or feed scope",
        )?;
        check(
            runtime
                .block_on(repo.cached_entry(1, 42))
                .unwrap()
                .is_none(),
            "Hard-deleted article remained cached",
        )?;
    }
    window.close();
    layout();
    drop(app);
    drop(controller);
    drop(runtime);
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}
fn find_navigation(widget: &gtk::Widget) -> Option<adw::NavigationView> {
    if let Some(view) = widget.downcast_ref::<adw::NavigationView>() {
        return Some(view.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(view) = find_navigation(&widget) {
            return Some(view);
        }
        child = widget.next_sibling();
    }
    None
}
fn find_destinations(widget: &gtk::Widget) -> Option<adw::ViewStack> {
    if let Some(view) = widget.downcast_ref::<adw::ViewStack>() {
        return Some(view.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(view) = find_destinations(&widget) {
            return Some(view);
        }
        child = widget.next_sibling();
    }
    None
}
