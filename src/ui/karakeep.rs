use adw::prelude::*;
use brooklet::{
    controller::AppController,
    model::{DeliveryState, KarakeepDelivery, KarakeepRoute},
};
use std::{rc::Rc, sync::Arc};

fn delivery_row(delivery: &KarakeepDelivery) -> (adw::ActionRow, gtk::Button, gtk::Button) {
    let row = adw::ActionRow::builder()
        .title(&delivery.title)
        .use_markup(false)
        .build();
    let route = if delivery.route == KarakeepRoute::Direct {
        "Direct"
    } else {
        "Miniflux integration"
    };
    let state = if delivery.state == DeliveryState::NeedsAttention {
        "Needs attention"
    } else {
        "Queued"
    };
    row.set_subtitle(&format!(
        "{}\n{route} · {state}\n{}",
        delivery.canonical_url,
        delivery.error.as_deref().unwrap_or("Waiting to send")
    ));
    row.set_subtitle_lines(0);
    let retry = gtk::Button::with_label("Retry");
    let dismiss = gtk::Button::with_label("Dismiss");
    for button in [&retry, &dismiss] {
        button.set_valign(gtk::Align::Center);
        row.add_suffix(button);
    }
    retry.set_tooltip_text(Some("Retry using the current Karakeep settings"));
    dismiss.set_tooltip_text(Some("Remove this local delivery; keep the article"));
    (row, retry, dismiss)
}

fn load(
    group: &adw::PreferencesGroup,
    status: &gtk::Label,
    controller: Arc<AppController>,
    on_retry: Rc<dyn Fn()>,
    refresh: gtk::Button,
) {
    status.set_text("Loading deliveries…");
    let group = group.clone();
    let status = status.clone();
    controller.unfinished_karakeep({
        let controller = controller.clone();
        move |result| {
            refresh.set_sensitive(true);
            match result {
                Err(error) => status.set_text(&error.karakeep_message()),
                Ok(deliveries) => {
                    status.set_text(if deliveries.is_empty() {
                        "No unfinished deliveries"
                    } else {
                        "Retry uses your current settings. Dismiss keeps the article."
                    });
                    for delivery in deliveries {
                        let (row, retry, dismiss) = delivery_row(&delivery);
                        group.add(&row);
                        for (button, is_retry) in [(&retry, true), (&dismiss, false)] {
                            button.connect_clicked({
                                let controller = controller.clone();
                                let row = row.downgrade();
                                let group = group.downgrade();
                                let status = status.clone();
                                let on_retry = on_retry.clone();
                                move |_| {
                                    let (Some(row), Some(group)) = (row.upgrade(), group.upgrade())
                                    else {
                                        return;
                                    };
                                    if !row.is_sensitive() {
                                        return;
                                    }
                                    row.set_sensitive(false);
                                    controller.recover_karakeep(delivery.id, is_retry, {
                                        let row = row.clone();
                                        let group = group.clone();
                                        let status = status.clone();
                                        let on_retry = on_retry.clone();
                                        move |result| match result {
                                            Ok(()) => {
                                                group.remove(&row);
                                                status.set_text(if is_retry {
                                                    "Delivery queued. Refresh to check its result."
                                                } else {
                                                    "Delivery dismissed. Article kept."
                                                });
                                                if is_retry {
                                                    on_retry();
                                                }
                                            }
                                            Err(error) => {
                                                row.set_sensitive(true);
                                                status.set_text(&error.karakeep_message());
                                            }
                                        }
                                    });
                                }
                            });
                        }
                    }
                }
            }
        }
    });
}

pub fn present(
    window: &adw::ApplicationWindow,
    controller: Arc<AppController>,
    on_retry: impl Fn() + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title("Karakeep deliveries")
        .content_width(640)
        .content_height(480)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    content.append(&status);
    let groups = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&groups);
    let refresh = gtk::Button::with_label("Refresh");
    content.append(&refresh);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroller));
    dialog.set_child(Some(&toolbar));
    let on_retry: Rc<dyn Fn()> = Rc::new(on_retry);
    let reload: Rc<dyn Fn()> = Rc::new({
        let groups = groups.downgrade();
        let status = status.downgrade();
        let refresh = refresh.downgrade();
        move || {
            let (Some(groups), Some(status), Some(refresh)) =
                (groups.upgrade(), status.upgrade(), refresh.upgrade())
            else {
                return;
            };
            if let Some(child) = groups.first_child() {
                groups.remove(&child);
            }
            let group = adw::PreferencesGroup::new();
            groups.append(&group);
            refresh.set_sensitive(false);
            // Re-enable after the controller request, preserving one active load.
            load(
                &group,
                &status,
                controller.clone(),
                on_retry.clone(),
                refresh,
            );
        }
    });
    refresh.connect_clicked({
        let reload = reload.clone();
        move |_| reload()
    });
    reload();
    dialog.present(Some(window));
}

pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    let delivery = KarakeepDelivery {
        id: 1,
        account_id: 1,
        entry_id: 42,
        canonical_url: "https://example.com".into(),
        title: "<Article>".into(),
        route: KarakeepRoute::Direct,
        state: DeliveryState::NeedsAttention,
        error: Some("Karakeep rejected the API key".into()),
    };
    let (row, retry, dismiss) = delivery_row(&delivery);
    if row.uses_markup()
        || row.title() != "<Article>"
        || !row
            .subtitle()
            .unwrap_or_default()
            .contains("Needs attention")
        || retry.label().as_deref() != Some("Retry")
        || dismiss.label().as_deref() != Some("Dismiss")
    {
        return Err(adw::glib::bool_error!("Karakeep recovery controls missing"));
    }

    use brooklet::{
        error::BrookletError,
        model::{Account, Entry},
        services::traits::{Repository, SecretStore},
        setup::{AccountSetupService, MinifluxIdentityValidator},
        storage::sqlite::SqliteRepository,
        sync::{AccountSyncService, ReqwestMinifluxApiFactory},
    };
    use std::{
        cell::Cell,
        time::{Duration, Instant},
    };
    struct Secrets;
    #[async_trait::async_trait]
    impl SecretStore for Secrets {
        async fn load_miniflux_token(&self, _: i64) -> Result<Option<String>, BrookletError> {
            Ok(None)
        }
        async fn store_miniflux_token(&self, _: i64, _: &str) -> Result<(), BrookletError> {
            unreachable!()
        }
        async fn delete_account_secrets(&self, _: i64) -> Result<(), BrookletError> {
            Ok(())
        }
    }
    fn wait_until(test: impl Fn() -> bool) -> Result<(), adw::glib::BoolError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !test() && Instant::now() < deadline {
            while adw::glib::MainContext::default().pending() {
                adw::glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if test() {
            Ok(())
        } else {
            Err(adw::glib::bool_error!("Karakeep UI request timed out"))
        }
    }
    fn find_button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && button.label().as_deref() == Some(label)
        {
            return Some(button);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(button) = find_button(&current, label) {
                return Some(button);
            }
            child = current.next_sibling();
        }
        None
    }
    let repo =
        Arc::new(SqliteRepository::open_in_memory().map_err(|e| adw::glib::bool_error!("{e}"))?);
    let runtime = tokio::runtime::Runtime::new().map_err(|e| adw::glib::bool_error!("{e}"))?;
    runtime
        .block_on(async {
            repo.save_account(&Account {
                id: 1,
                server_url: "https://miniflux.example".into(),
                username: "reader".into(),
                server_version: "2.3.2".into(),
            })
            .await?;
            repo.merge_changed_page(
                1,
                &[Entry {
                    id: 42,
                    account_id: 1,
                    feed_id: 7,
                    feed_title: "Feed".into(),
                    category_title: String::new(),
                    title: "<Article>".into(),
                    url: delivery.canonical_url.clone(),
                    author: None,
                    published_at_ms: 0,
                    html: "<p>Offline article</p>".into(),
                    content_revision: 0,
                    read: false,
                    starred: false,
                    reading_minutes: 1,
                    delivery_state: None,
                    delivery_error: None,
                }],
                &[],
            )
            .await?;
            repo.queue_karakeep(&delivery).await?;
            let id = repo.pending_karakeep(1).await?[0].id;
            repo.finish_karakeep(id, Some("Rejected key"), 0).await
        })
        .map_err(|e: BrookletError| adw::glib::bool_error!("{e}"))?;
    let secrets = Arc::new(Secrets);
    let service = Arc::new(AccountSyncService::new(
        repo.clone(),
        secrets.clone(),
        Arc::new(ReqwestMinifluxApiFactory),
    ));
    let directory =
        std::env::temp_dir().join(format!("brooklet-karakeep-smoke-{}", std::process::id()));
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
    let group = adw::PreferencesGroup::new();
    let status = gtk::Label::new(None);
    let refresh = gtk::Button::with_label("Refresh");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&status);
    body.append(&group);
    let window = gtk::Window::builder().child(&body).build();
    window.present();
    let retried = Rc::new(Cell::new(0));
    let on_retry: Rc<dyn Fn()> = Rc::new({
        let retried = retried.clone();
        move || retried.set(retried.get() + 1)
    });
    refresh.set_sensitive(false);
    load(
        &group,
        &status,
        controller.clone(),
        on_retry.clone(),
        refresh.clone(),
    );
    wait_until(|| refresh.is_sensitive())?;
    let retry = find_button(group.upcast_ref(), "Retry")
        .ok_or_else(|| adw::glib::bool_error!("Missing retry control"))?;
    retry.emit_clicked();
    retry.emit_clicked();
    wait_until(|| retried.get() == 1)?;
    if runtime.block_on(repo.pending_karakeep(1)).unwrap().len() != 1 {
        return Err(adw::glib::bool_error!("UI retry did not persist"));
    }
    // Reload the queued receipt, then exercise its actual dismiss control.
    refresh.set_sensitive(false);
    load(
        &group,
        &status,
        controller.clone(),
        on_retry,
        refresh.clone(),
    );
    wait_until(|| refresh.is_sensitive())?;
    find_button(group.upcast_ref(), "Dismiss")
        .ok_or_else(|| adw::glib::bool_error!("Missing dismiss control"))?
        .emit_clicked();
    wait_until(|| status.text().contains("dismissed"))?;
    if !runtime
        .block_on(repo.unfinished_karakeep(1))
        .unwrap()
        .is_empty()
        || runtime
            .block_on(repo.cached_entry(1, 42))
            .unwrap()
            .is_none()
    {
        return Err(adw::glib::bool_error!(
            "UI dismiss lost article or kept receipt"
        ));
    }
    window.close();
    drop(controller);
    drop(runtime);
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}
