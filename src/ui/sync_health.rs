use adw::prelude::*;
use brooklet::{controller::AppController, error::BrookletError, model::SyncStatus};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

type StatusCallback = Box<dyn FnOnce(Result<SyncStatus, BrookletError>)>;
type StatusLoader = Rc<dyn Fn(StatusCallback)>;

struct HealthView {
    bar: gtk::Box,
    headline: gtk::Label,
    refresh: gtk::Label,
    delivery: gtk::Label,
    pending: gtk::Label,
    last_sync: gtk::Label,
    retry: gtk::Button,
    deliveries: gtk::Button,
    show_healthy: bool,
    alive: Cell<bool>,
    busy: Cell<bool>,
    timer: RefCell<Option<adw::glib::SourceId>>,
}

impl HealthView {
    fn new(show_healthy: bool) -> Rc<Self> {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.set_widget_name("sync-health");
        bar.set_visible(show_healthy);
        bar.set_margin_start(12);
        bar.set_margin_end(12);
        bar.set_margin_top(4);
        bar.set_margin_bottom(4);
        let headline = gtk::Label::new(Some("Loading sync status…"));
        headline.set_xalign(0.0);
        headline.set_hexpand(true);
        headline.set_ellipsize(gtk::pango::EllipsizeMode::End);
        headline.add_css_class("dim-label");
        bar.append(&headline);
        let menu = gtk::MenuButton::new();
        menu.set_label(if show_healthy {
            "Sync status"
        } else {
            "Details"
        });
        menu.add_css_class("flat");
        let popover = gtk::Popover::new();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.set_margin_start(12);
        body.set_margin_end(12);
        body.set_margin_top(12);
        body.set_margin_bottom(12);
        let label = |text| {
            let label = gtk::Label::new(Some(text));
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_max_width_chars(32);
            body.append(&label);
            label
        };
        let refresh = label("Loading refresh status…");
        let delivery = label("Loading delivery status…");
        let pending = label("Loading pending changes…");
        let last_sync = label("Loading last sync…");
        let retry = gtk::Button::with_label("Sync now");
        retry.set_action_name(Some("app.sync"));
        body.append(&retry);
        let reconnect = gtk::Button::with_label("Reconnect account…");
        reconnect.set_action_name(Some("win.reconnect"));
        body.append(&reconnect);
        let deliveries = gtk::Button::with_label("Review failed deliveries…");
        deliveries.set_action_name(Some("win.delivery-review"));
        body.append(&deliveries);
        let settings = gtk::Button::with_label("Account and delivery settings…");
        settings.set_action_name(Some("win.preferences"));
        body.append(&settings);
        // Hide the popover before opening a recovery dialog so focus is unambiguous.
        for button in [&retry, &reconnect, &deliveries, &settings] {
            let popover = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
            });
        }
        popover.set_child(Some(&body));
        menu.set_popover(Some(&popover));
        bar.append(&menu);
        Rc::new(Self {
            bar,
            headline,
            refresh,
            delivery,
            pending,
            last_sync,
            retry,
            deliveries,
            show_healthy,
            alive: Cell::new(true),
            busy: Cell::new(false),
            timer: RefCell::new(None),
        })
    }

    fn update(&self, result: Result<SyncStatus, BrookletError>) {
        if !self.alive.get() {
            return;
        }
        match result {
            Ok(status) => {
                self.bar.set_visible(
                    self.show_healthy
                        || status.refresh_error.is_some()
                        || status.delivery_error.is_some()
                        || status.error.is_some(),
                );
                let headline = brooklet::sync_health::headline(&status);
                self.headline.set_text(&headline);
                self.headline.set_tooltip_text(Some(&headline));
                self.refresh
                    .set_text(&status.refresh_error.as_ref().map_or_else(
                        || "Refresh: no recorded failure".into(),
                        |error| {
                            format!("Refresh failed: {error}. Cached articles remain available.")
                        },
                    ));
                self.delivery
                    .set_text(&status.delivery_error.as_ref().map_or_else(
                        || "Delivery: no recorded failure".into(),
                        |error| {
                            format!("Delivery failed: {error}. Pending changes are kept locally.")
                        },
                    ));
                self.pending.set_text(&format!(
                    "{} article changes · {} Karakeep deliveries waiting",
                    status
                        .queued_mutations
                        .saturating_sub(status.queued_karakeep),
                    status.queued_karakeep
                ));
                self.last_sync
                    .set_text(&status.last_successful_sync_at_ms.map_or_else(
                        || "Last successful refresh: never".into(),
                        |time| {
                            jiff::Timestamp::from_millisecond(time).map_or_else(
                                |_| "Last successful refresh: unknown".into(),
                                |time| format!("Last successful refresh: {time}"),
                            )
                        },
                    ));
                self.retry.set_sensitive(!status.running);
                self.deliveries
                    .set_visible(status.queued_karakeep > 0 && status.delivery_error.is_some());
            }
            Err(BrookletError::InvalidSetup("a configured Miniflux account")) => {
                self.bar.set_visible(false);
            }
            Err(error) => {
                self.bar.set_visible(true);
                self.headline.set_text("Sync status unavailable");
                self.refresh.set_text(&error.sync_message());
                // Retain the last known delivery/count/time details on a read failure.
                self.retry.set_sensitive(true);
            }
        }
    }

    fn request(self: &Rc<Self>, load: &StatusLoader) {
        if !self.alive.get() || self.busy.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        load(Box::new(move |result| {
            if let Some(view) = weak.upgrade() {
                view.busy.set(false);
                view.update(result);
            }
        }));
    }

    fn stop(&self) {
        self.alive.set(false);
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }
}

pub fn install(window: &adw::ApplicationWindow, host: &gtk::Box, controller: Arc<AppController>) {
    let view = install_loader(
        host,
        Rc::new(move |callback| controller.sync_status(callback)),
        false,
    );
    let closing = view.clone();
    window.connect_close_request(move |_| {
        closing.stop();
        adw::glib::Propagation::Proceed
    });
    window.connect_destroy(move |_| view.stop());
}

pub fn install_dialog(
    dialog: &adw::PreferencesDialog,
    host: &gtk::Box,
    controller: Arc<AppController>,
) {
    let view = install_loader(
        host,
        Rc::new(move |callback| controller.sync_status(callback)),
        true,
    );
    let destroyed = view.clone();
    dialog.connect_destroy(move |_| destroyed.stop());
    dialog.connect_closed(move |_| view.stop());
}

fn install_loader(host: &gtk::Box, load: StatusLoader, show_healthy: bool) -> Rc<HealthView> {
    let view = HealthView::new(show_healthy);
    host.append(&view.bar);
    view.request(&load);
    let weak = Rc::downgrade(&view);
    let host = host.downgrade();
    let timer = adw::glib::timeout_add_local(Duration::from_secs(2), move || {
        let Some(view) = weak.upgrade().filter(|view| view.alive.get()) else {
            return adw::glib::ControlFlow::Break;
        };
        if host.upgrade().is_some_and(|host| host.is_mapped()) {
            view.request(&load);
        }
        adw::glib::ControlFlow::Continue
    });
    *view.timer.borrow_mut() = Some(timer);
    view
}

pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    let view = HealthView::new(false);
    let status = SyncStatus {
        queued_mutations: 3,
        queued_karakeep: 1,
        refresh_error: Some("HTTP 401 <not markup>".into()),
        delivery_error: Some("HTTP 503".into()),
        ..Default::default()
    };
    view.update(Ok(status.clone()));
    if view.headline.text() != "Sync needs attention"
        || !view.refresh.text().contains("<not markup>")
        || !view.delivery.text().contains("503")
        || !view.deliveries.property::<bool>("visible")
        || !view.pending.text().contains("2 article changes")
    {
        return Err(adw::glib::bool_error!(
            "Health view lost separate errors or pending counts"
        ));
    }
    view.update(Ok(SyncStatus {
        running: true,
        ..status
    }));
    if view.retry.property::<bool>("sensitive")
        || !view.headline.text().contains("Previous failure")
    {
        return Err(adw::glib::bool_error!(
            "Running sync hid health failure or allowed duplicate retry"
        ));
    }
    view.update(Ok(SyncStatus {
        last_successful_sync_at_ms: Some(0),
        ..Default::default()
    }));
    if view.bar.property::<bool>("visible")
        || view.deliveries.property::<bool>("visible")
        || !view.headline.text().starts_with("Last sync")
        || view.refresh.text().contains("401")
        || view.delivery.text().contains("503")
    {
        return Err(adw::glib::bool_error!("Health view did not recover"));
    }
    view.update(Ok(SyncStatus {
        running: true,
        queued_mutations: 2,
        queued_karakeep: 1,
        ..Default::default()
    }));
    if view.bar.property::<bool>("visible") || view.deliveries.property::<bool>("visible") {
        return Err(adw::glib::bool_error!(
            "Normal sync or queued changes showed failure UI"
        ));
    }
    let pending = Rc::new(RefCell::new(Vec::<StatusCallback>::new()));
    let load: StatusLoader = {
        let pending = pending.clone();
        Rc::new(move |callback| pending.borrow_mut().push(callback))
    };
    view.request(&load);
    view.request(&load);
    if pending.borrow().len() != 1 {
        return Err(adw::glib::bool_error!("Health requests overlapped"));
    }
    view.stop();
    let before = view.headline.text();
    pending.borrow_mut().pop().unwrap()(Ok(SyncStatus::default()));
    if view.headline.text() != before {
        return Err(adw::glib::bool_error!(
            "Late status changed closed health view"
        ));
    }
    // No-account status hides the strip, but must not stop observing setup.
    let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let window = gtk::Window::new();
    window.set_child(Some(&host));
    let monitored = install_loader(&host, load, false);
    window.connect_destroy({
        let monitored = monitored.clone();
        move |_| monitored.stop()
    });
    window.present();
    pending.borrow_mut().pop().unwrap()(Err(BrookletError::InvalidSetup(
        "a configured Miniflux account",
    )));
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while pending.borrow().is_empty() && std::time::Instant::now() < deadline {
        while adw::glib::MainContext::default().pending() {
            adw::glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    if pending.borrow().len() != 1 {
        return Err(adw::glib::bool_error!(
            "Hidden health view stopped watching account setup"
        ));
    }
    pending.borrow_mut().pop().unwrap()(Ok(SyncStatus {
        refresh_error: Some("offline".into()),
        ..Default::default()
    }));
    if !monitored.bar.property::<bool>("visible") {
        return Err(adw::glib::bool_error!(
            "Account setup did not reveal health"
        ));
    }
    window.destroy();
    drop(window);
    if monitored.alive.get() || monitored.timer.borrow().is_some() {
        return Err(adw::glib::bool_error!(
            "Destroyed window retained status monitor"
        ));
    }
    Ok(())
}
