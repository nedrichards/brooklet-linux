use std::{cell::RefCell, rc::Rc, sync::Arc};

use adw::prelude::*;
use brooklet::{controller::AppController, model::Account};

struct ReconnectDialog {
    dialog: adw::Dialog,
    token: adw::PasswordEntryRow,
    submit: gtk::Button,
    error: gtk::Label,
    progress: gtk::Box,
}

impl ReconnectDialog {
    fn new(account: &Account) -> Self {
        let dialog = adw::Dialog::builder()
            .title("Reconnect to Miniflux")
            .content_width(480)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
        body.set_margin_top(24);
        body.set_margin_bottom(24);
        body.set_margin_start(24);
        body.set_margin_end(24);
        let group = adw::PreferencesGroup::new();
        group.set_description(Some("Enter a replacement API token for this account. Your cached articles, reader positions, and pending changes will be kept."));
        let account_row = adw::ActionRow::builder()
            .title(&account.username)
            .subtitle(&account.server_url)
            .use_markup(false)
            .build();
        group.add(&account_row);
        let token = adw::PasswordEntryRow::builder().title("API token").build();
        group.add(&token);
        body.append(&group);
        let error = gtk::Label::new(None);
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        error.add_css_class("error");
        error.set_accessible_role(gtk::AccessibleRole::Alert);
        body.append(&error);
        let progress = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        progress.set_halign(gtk::Align::Center);
        progress.append(&adw::Spinner::new());
        progress.append(&gtk::Label::new(Some("Checking your account…")));
        progress.set_visible(false);
        body.append(&progress);
        let submit = gtk::Button::with_label("Reconnect");
        submit.set_halign(gtk::Align::Center);
        submit.add_css_class("suggested-action");
        submit.set_sensitive(false);
        body.append(&submit);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&body));
        dialog.set_child(Some(&toolbar));
        Self {
            dialog,
            token,
            submit,
            error,
            progress,
        }
    }

    fn connect(&self, controller: Arc<AppController>, on_complete: impl Fn(Account) + 'static) {
        let signals = super::signal_scope::SignalScope::for_dialog(&self.dialog);
        signals.track(
            &self.token,
            self.token.connect_changed({
                let submit = self.submit.clone();
                let error = self.error.clone();
                move |token| {
                    submit.set_sensitive(token.is_sensitive() && !token.text().trim().is_empty());
                    error.set_visible(false);
                }
            }),
        );
        signals.track(
            &self.token,
            self.token.connect_entry_activated({
                let submit = self.submit.clone();
                move |_| {
                    if submit.is_sensitive() {
                        submit.emit_clicked();
                    }
                }
            }),
        );
        signals.track(
            &self.submit,
            self.submit.connect_clicked({
                let token = self.token.clone();
                let submit = self.submit.clone();
                let dialog = self.dialog.clone();
                let error = self.error.clone();
                let progress = self.progress.clone();
                let on_complete = Rc::new(on_complete);
                move |_| {
                    if !submit.is_sensitive() {
                        return;
                    }
                    let secret = token.text().to_string();
                    token.set_text("");
                    token.set_sensitive(false);
                    submit.set_sensitive(false);
                    dialog.set_can_close(false);
                    error.set_visible(false);
                    progress.set_visible(true);
                    controller.reconnect(secret, {
                        let token = token.clone();
                        let dialog = dialog.clone();
                        let error = error.clone();
                        let progress = progress.clone();
                        let on_complete = on_complete.clone();
                        move |result| {
                            progress.set_visible(false);
                            dialog.set_can_close(true);
                            match result {
                                Ok(account) => {
                                    dialog.close();
                                    on_complete(account);
                                }
                                Err(failure) => {
                                    error.set_label(&failure.setup_message());
                                    error.set_visible(true);
                                    token.set_sensitive(true);
                                    token.grab_focus();
                                }
                            }
                        }
                    });
                }
            }),
        );
    }
}

pub fn present(
    parent: &adw::ApplicationWindow,
    controller: Arc<AppController>,
    account: &Account,
    on_complete: impl Fn(Account) + 'static,
) {
    let form = ReconnectDialog::new(account);
    form.connect(controller, on_complete);
    form.dialog.present(Some(parent));
    form.token.grab_focus();
}

/// A sync using the old token may still be finishing. Retry once it completes
/// rather than losing the reconnect request while the action is disabled.
pub fn sync_when_ready(window: &adw::ApplicationWindow) {
    let Some(action) = window
        .application()
        .and_then(|app| app.lookup_action("sync"))
    else {
        return;
    };
    if action.is_enabled() {
        action.activate(None);
        return;
    }
    let signal = Rc::new(RefCell::new(None));
    let window = window.downgrade();
    let id = action.connect_enabled_notify({
        let signal = signal.clone();
        move |action| {
            if !action.is_enabled() {
                return;
            }
            if let Some(id) = signal.borrow_mut().take() {
                action.disconnect(id);
            }
            // Finish the old sync's UI callback before starting the next one;
            // otherwise it could hide the new spinner or overwrite its state.
            let window = window.clone();
            adw::glib::idle_add_local_once(move || {
                if let Some(window) = window.upgrade() {
                    sync_when_ready(&window);
                }
            });
        }
    });
    *signal.borrow_mut() = Some(id);
}

pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    use async_trait::async_trait;
    use brooklet::{
        error::BrookletError,
        services::traits::{Repository, SecretStore},
        setup::{SetupRequest, SetupService},
        storage::sqlite::SqliteRepository,
        sync::{AccountSyncService, ReqwestMinifluxApiFactory},
    };
    use std::{
        cell::Cell,
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

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
            Err(adw::glib::bool_error!("Reconnect callback timed out"))
        }
    }
    struct Setup {
        repo: Arc<SqliteRepository>,
        gate: Arc<tokio::sync::Semaphore>,
        calls: AtomicUsize,
    }
    #[async_trait]
    impl SetupService for Setup {
        async fn existing_account(&self) -> Result<Option<Account>, BrookletError> {
            self.repo.account().await
        }
        async fn configure(&self, _: SetupRequest) -> Result<Account, BrookletError> {
            unreachable!()
        }
        async fn reconnect(&self, _: String) -> Result<Account, BrookletError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            self.gate.acquire().await.unwrap().forget();
            if call == 0 {
                return Err(BrookletError::Http {
                    status: 401,
                    kind: brooklet::model::classify_http_status(401),
                });
            }
            self.repo
                .account()
                .await?
                .ok_or(BrookletError::InvalidSetup("the existing account"))
        }
    }
    struct Secrets;
    #[async_trait]
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
    let account = Account {
        id: 1,
        server_url: "https://miniflux.example".into(),
        username: "reader".into(),
        server_version: "2.3.2".into(),
    };
    let repo =
        Arc::new(SqliteRepository::open_in_memory().map_err(|e| adw::glib::bool_error!("{e}"))?);
    let runtime = tokio::runtime::Runtime::new().map_err(|e| adw::glib::bool_error!("{e}"))?;
    runtime
        .block_on(repo.save_account(&account))
        .map_err(|e| adw::glib::bool_error!("{e}"))?;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let setup = Arc::new(Setup {
        repo: repo.clone(),
        gate: gate.clone(),
        calls: AtomicUsize::new(0),
    });
    let directory =
        std::env::temp_dir().join(format!("brooklet-reconnect-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|e| adw::glib::bool_error!("{e}"))?;
    let cache = brooklet::services::image_cache::ImageCache::new(directory.join("images.db"), 1024)
        .map_err(|e| adw::glib::bool_error!("{e}"))?;
    let controller = Arc::new(
        AppController::with_image_cache(
            setup.clone(),
            Arc::new(AccountSyncService::new(
                repo.clone(),
                Arc::new(Secrets),
                Arc::new(ReqwestMinifluxApiFactory),
            )),
            cache,
        )
        .map_err(|e| adw::glib::bool_error!("{e}"))?,
    );
    let app = adw::Application::new(
        Some("com.nedrichards.brooklet.ReconnectTest"),
        adw::gio::ApplicationFlags::NON_UNIQUE,
    );
    app.register(None::<&adw::gio::Cancellable>)
        .map_err(|e| adw::glib::bool_error!("{e}"))?;
    let window = adw::ApplicationWindow::builder().application(&app).build();
    window.present();
    let form = ReconnectDialog::new(&account);
    let completed = Rc::new(Cell::new(false));
    form.connect(controller.clone(), {
        let completed = completed.clone();
        move |_| completed.set(true)
    });
    form.dialog.present(Some(&window));
    form.token.set_text("  ");
    if form.submit.is_sensitive() {
        return Err(adw::glib::bool_error!("Blank token allowed reconnect"));
    }
    form.token.set_text("invalid");
    form.submit.emit_clicked();
    form.submit.emit_clicked();
    wait_until(|| setup.calls.load(Ordering::SeqCst) == 1)?;
    if !form.token.text().is_empty()
        || form.token.is_sensitive()
        || form.submit.is_sensitive()
        || form.dialog.can_close()
    {
        return Err(adw::glib::bool_error!(
            "Reconnect did not clear the token and prevent duplicate submission"
        ));
    }
    gate.add_permits(1);
    wait_until(|| form.error.is_visible())?;
    if !form.token.is_sensitive() || !form.dialog.can_close() || completed.get() {
        return Err(adw::glib::bool_error!(
            "Rejected reconnect did not permit retry"
        ));
    }
    form.token.set_text("replacement");
    form.token.emit_by_name::<()>("entry-activated", &[]);
    gate.add_permits(1);
    wait_until(|| completed.get())?;
    if setup.calls.load(Ordering::SeqCst) != 2 || form.error.is_visible() {
        return Err(adw::glib::bool_error!(
            "Reconnect retry did not complete once"
        ));
    }

    form.dialog.emit_by_name::<()>("closed", &[]);

    // Prove logout waits for an already-running reconnect and then removes data.
    let reconnected = Rc::new(Cell::new(false));
    controller.reconnect("serialised".into(), {
        let done = reconnected.clone();
        move |result| done.set(result.is_ok())
    });
    wait_until(|| setup.calls.load(Ordering::SeqCst) == 3)?;
    let logged_out = Rc::new(Cell::new(false));
    controller.disconnect({
        let done = logged_out.clone();
        move |result| done.set(result.is_ok())
    });
    gate.add_permits(1);
    wait_until(|| reconnected.get() && logged_out.get())?;
    if runtime
        .block_on(repo.account())
        .map_err(|e| adw::glib::bool_error!("{e}"))?
        .is_some()
    {
        return Err(adw::glib::bool_error!(
            "Logout left a repaired account behind"
        ));
    }

    let action = adw::gio::SimpleAction::new("sync", None);
    let retries = Rc::new(Cell::new(0));
    action.connect_activate({
        let retries = retries.clone();
        move |_, _| retries.set(retries.get() + 1)
    });
    app.add_action(&action);
    action.set_enabled(false);
    sync_when_ready(&window);
    if retries.get() != 0 {
        return Err(adw::glib::bool_error!("Reconnect invoked a disabled sync"));
    }
    action.set_enabled(true);
    wait_until(|| retries.get() == 1)?;
    action.set_enabled(false);
    action.set_enabled(true);
    if retries.get() != 1 {
        return Err(adw::glib::bool_error!(
            "Reconnect did not retry busy sync exactly once"
        ));
    }
    window.close();
    drop(controller);
    let _ = std::fs::remove_dir_all(directory);
    Ok(())
}
