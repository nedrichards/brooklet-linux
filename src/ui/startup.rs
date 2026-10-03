use adw::{gio, prelude::*};
use brooklet::error::BrookletError;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

type Loader<T> = Arc<dyn Fn() -> Result<T, BrookletError> + Send + Sync>;
type Finish<T> = Rc<dyn Fn(T) -> Result<(), BrookletError>>;

struct FailureWindow {
    window: adw::ApplicationWindow,
    details: gtk::Label,
    retry: gtk::Button,
    spinner: adw::Spinner,
}

pub(crate) struct Startup<T> {
    application: adw::glib::WeakRef<adw::Application>,
    path: PathBuf,
    load: Loader<T>,
    finish: Finish<T>,
    running: Cell<bool>,
    ready: Cell<bool>,
    closed: Cell<bool>,
    failure: RefCell<Option<FailureWindow>>,
}

pub(crate) fn install<T: Send + 'static>(
    application: &adw::Application,
    path: PathBuf,
    load: Loader<T>,
    finish: Finish<T>,
) -> Rc<Startup<T>> {
    let startup = Rc::new(Startup {
        application: application.downgrade(),
        path,
        load,
        finish,
        running: Cell::new(false),
        ready: Cell::new(false),
        closed: Cell::new(false),
        failure: RefCell::new(None),
    });
    application.connect_activate({
        let startup = startup.clone();
        move |_| {
            if startup.ready.get() || startup.closed.get() {
                return;
            }
            if let Some(failure) = startup.failure.borrow().as_ref() {
                failure.window.present();
            } else {
                startup.retry();
            }
        }
    });
    application.connect_shutdown({
        let startup = Rc::downgrade(&startup);
        move |_| {
            if let Some(startup) = startup.upgrade() {
                startup.closed.set(true);
            }
        }
    });
    startup
}

impl<T: Send + 'static> Startup<T> {
    fn retry(self: &Rc<Self>) {
        if self.closed.get() || self.ready.get() || self.running.replace(true) {
            return;
        }
        let Some(application) = self.application.upgrade() else {
            return;
        };
        if let Some(failure) = self.failure.borrow().as_ref() {
            failure.retry.set_sensitive(false);
            failure.spinner.set_visible(true);
        }
        // Keep the initial launch alive while there is no window yet.
        let hold = application.hold();
        let startup = self.clone();
        let load = self.load.clone();
        adw::glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || load())
                .await
                .unwrap_or_else(|_| {
                    Err(BrookletError::Storage(std::io::Error::other(
                        "Startup worker stopped unexpectedly",
                    )))
                });
            startup.running.set(false);
            if startup.closed.get() {
                return;
            }
            let result = result.and_then(|prepared| (startup.finish)(prepared));
            match result {
                Ok(()) => {
                    startup.ready.set(true);
                    if let Some(failure) = startup.failure.borrow_mut().take() {
                        failure.window.destroy();
                    }
                    if let Some(application) = startup.application.upgrade() {
                        application.activate();
                    }
                }
                Err(error) => startup.show_failure(&error),
            }
            drop(hold);
        });
    }

    fn show_failure(self: &Rc<Self>, error: &BrookletError) {
        let Some(application) = self.application.upgrade() else {
            return;
        };
        let details = error_details(&self.path, error);
        tracing::warn!(?error, "local data could not be opened");
        if let Some(failure) = self.failure.borrow().as_ref() {
            failure.details.set_text(&details);
            failure.retry.set_sensitive(true);
            failure.spinner.set_visible(false);
            failure.window.present();
            return;
        }
        let window = adw::ApplicationWindow::builder()
            .application(&application)
            .title("Brooklet")
            .default_width(540)
            .default_height(460)
            .build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        let status = adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .title("Couldn’t Open Local Data")
            .description("Check permissions, available disk space, or whether another process is using the database, then try again. Your data has not been reset.")
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.set_halign(gtk::Align::Center);
        let quit = gtk::Button::with_label("Quit");
        let retry = gtk::Button::with_label("Try Again");
        retry.add_css_class("suggested-action");
        let spinner = adw::Spinner::new();
        spinner.set_size_request(24, 24);
        spinner.set_visible(false);
        buttons.append(&quit);
        buttons.append(&retry);
        buttons.append(&spinner);
        content.append(&buttons);
        let label = gtk::Label::builder()
            .label(&details)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .xalign(0.0)
            .use_markup(false)
            .build();
        let expander = gtk::Expander::builder()
            .label("Error Details")
            .child(&label)
            .build();
        content.append(&expander);
        status.set_child(Some(&content));
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&status)
            .build();
        view.set_content(Some(&scroller));
        window.set_content(Some(&view));
        retry.connect_clicked({
            let startup = Rc::downgrade(self);
            move |_| {
                if let Some(startup) = startup.upgrade() {
                    startup.retry();
                }
            }
        });
        quit.connect_clicked({
            let window = window.downgrade();
            move |_| {
                if let Some(window) = window.upgrade() {
                    window.close();
                }
            }
        });
        window.connect_close_request({
            let startup = Rc::downgrade(self);
            move |_| {
                if let Some(startup) = startup.upgrade() {
                    startup.closed.set(true);
                    startup.failure.borrow_mut().take();
                    if let Some(application) = startup.application.upgrade() {
                        application.quit();
                    }
                }
                adw::glib::Propagation::Proceed
            }
        });
        window.present();
        *self.failure.borrow_mut() = Some(FailureWindow {
            window,
            details: label,
            retry,
            spinner,
        });
    }
}

fn error_details(path: &std::path::Path, error: &BrookletError) -> String {
    use std::error::Error;
    let mut details = format!("Database: {}\n\n{error}", path.display());
    let mut cause = error.source();
    while let Some(error) = cause {
        details.push_str(&format!("\n{error}"));
        cause = error.source();
    }
    details
}

pub fn smoke_test() -> Result<(), adw::glib::BoolError> {
    adw::init()?;
    smoke::run()
}

mod smoke {
    use super::*;
    use brooklet::storage::sqlite::SqliteRepository;
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    fn wait(predicate: impl Fn() -> bool) -> Result<(), adw::glib::BoolError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        let context = adw::glib::MainContext::default();
        while !predicate() {
            while context.pending() {
                context.iteration(false);
            }
            if std::time::Instant::now() >= deadline {
                return Err(adw::glib::bool_error!("Startup check timed out"));
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok(())
    }
    fn check(value: bool, message: &str) -> Result<(), adw::glib::BoolError> {
        if value {
            Ok(())
        } else {
            Err(adw::glib::bool_error!("{message}"))
        }
    }
    fn application(id: &str) -> Result<adw::Application, adw::glib::BoolError> {
        let app = adw::Application::new(Some(id), gio::ApplicationFlags::NON_UNIQUE);
        app.register(None::<&gio::Cancellable>)
            .map_err(|error| adw::glib::bool_error!("{error}"))?;
        Ok(app)
    }
    fn button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.label().as_deref() == Some(label)
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(button) = button(&widget, label) {
                return Some(button);
            }
            child = widget.next_sibling();
        }
        None
    }
    pub(super) fn run() -> Result<(), adw::glib::BoolError> {
        let directory = std::env::temp_dir().join(format!(
            "brooklet-startup-smoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).map_err(|error| adw::glib::bool_error!("{error}"))?;
        let path = directory.join("brooklet.db");
        let bytes = b"broken database <not markup>";
        std::fs::write(&path, bytes).map_err(|error| adw::glib::bool_error!("{error}"))?;
        let calls = Arc::new(AtomicUsize::new(0));
        let finished = Rc::new(Cell::new(0));
        let app = application("com.nedrichards.brooklet.StartupTest")?;
        let startup = install(
            &app,
            path.clone(),
            {
                let path = path.clone();
                let calls = calls.clone();
                Arc::new(move || {
                    calls.fetch_add(1, Ordering::AcqRel);
                    SqliteRepository::open(&path)
                })
            },
            {
                let finished = finished.clone();
                Rc::new(move |_| {
                    finished.set(finished.get() + 1);
                    Ok(())
                })
            },
        );
        app.activate();
        wait(|| startup.failure.borrow().is_some())?;
        check(
            std::fs::read(&path).unwrap() == bytes && finished.get() == 0,
            "Failed startup replaced data or initialized services",
        )?;
        let window = startup.failure.borrow().as_ref().unwrap().window.clone();
        let retry = startup.failure.borrow().as_ref().unwrap().retry.clone();
        check(
            startup
                .failure
                .borrow()
                .as_ref()
                .unwrap()
                .details
                .text()
                .contains("not a database"),
            "Missing underlying startup error",
        )?;
        app.activate();
        check(
            calls.load(Ordering::Acquire) == 1 && app.windows().len() == 1,
            "Activation duplicated startup attempts or windows",
        )?;
        retry.emit_clicked();
        retry.emit_clicked();
        wait(|| !startup.running.get())?;
        check(
            calls.load(Ordering::Acquire) == 2
                && startup.failure.borrow().as_ref().unwrap().window == window,
            "Retry duplicated requests or replaced failure window",
        )?;
        check(
            std::fs::read(&path).unwrap() == bytes,
            "Retry destroyed failed database",
        )?;
        // Simulate repair outside the app. Retry must reopen the same path.
        std::fs::rename(&path, directory.join("preserved-broken.db")).unwrap();
        retry.emit_clicked();
        wait(|| startup.ready.get())?;
        check(
            finished.get() == 1 && startup.failure.borrow().is_none() && app.windows().is_empty(),
            "Successful retry failed to transition exactly once",
        )?;
        app.activate();
        check(
            calls.load(Ordering::Acquire) == 3,
            "Ready activation reopened the database",
        )?;

        let app = application("com.nedrichards.brooklet.StartupCloseTest")?;
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let entered = Arc::new(AtomicUsize::new(0));
        let finished = Rc::new(Cell::new(false));
        let startup = install(
            &app,
            path.clone(),
            {
                let gate = gate.clone();
                let entered = entered.clone();
                Arc::new(move || {
                    let attempt = entered.fetch_add(1, Ordering::AcqRel);
                    if attempt == 0 {
                        return Err(BrookletError::InvalidSetup("test failure"));
                    }
                    let (lock, signal) = &*gate;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = signal.wait(released).unwrap();
                    }
                    Ok(())
                })
            },
            {
                let finished = finished.clone();
                Rc::new(move |_| {
                    finished.set(true);
                    Ok(())
                })
            },
        );
        app.activate();
        wait(|| startup.failure.borrow().is_some())?;
        let window = startup.failure.borrow().as_ref().unwrap().window.clone();
        let retry = startup.failure.borrow().as_ref().unwrap().retry.clone();
        retry.emit_clicked();
        wait(|| entered.load(Ordering::Acquire) == 2)?;
        check(
            !retry.is_sensitive() && startup.running.get(),
            "Retry not guarded during blocking startup",
        )?;
        // The GTK context remains responsive while the backend is blocked.
        let heartbeat = Rc::new(Cell::new(false));
        adw::glib::idle_add_local_once({
            let heartbeat = heartbeat.clone();
            move || heartbeat.set(true)
        });
        wait(|| heartbeat.get())?;
        window.close();
        check(
            startup.closed.get(),
            "Closing recovery did not cancel startup transition",
        )?;
        let (lock, signal) = &*gate;
        *lock.lock().unwrap() = true;
        signal.notify_all();
        wait(|| !startup.running.get())?;
        check(
            !finished.get() && app.windows().is_empty(),
            "Late startup result resurrected a closed window or started services",
        )?;

        // The production loader/finish wiring must recover into the real app.
        let path = directory.join("production.db");
        std::fs::write(&path, bytes).unwrap();
        let app = crate::application::startup_smoke_application(
            path.clone(),
            directory.join("images.db"),
        )?;
        app.activate();
        wait(|| app.active_window().is_some())?;
        let failure = app.active_window().unwrap();
        let retry = button(failure.upcast_ref(), "Try Again")
            .ok_or_else(|| adw::glib::bool_error!("Production startup did not show recovery"))?;
        check(
            app.lookup_action("sync").is_none(),
            "Failed startup installed normal actions",
        )?;
        std::fs::rename(&path, directory.join("preserved-production.db")).unwrap();
        retry.emit_clicked();
        wait(|| {
            app.lookup_action("sync").is_some()
                && app.active_window().is_some_and(|window| window != failure)
        })
        .map_err(|_| {
            adw::glib::bool_error!(
                "Production recovery timed out: sync={} shortcuts={} windows={:?}",
                app.lookup_action("sync").is_some(),
                app.lookup_action("show-shortcuts").is_some(),
                app.windows()
                    .iter()
                    .map(|window| (window.title(), window.is_visible(), window == &failure))
                    .collect::<Vec<_>>()
            )
        })?;
        let normal = app.active_window().unwrap();
        check(
            app.windows().len() == 1 && button(normal.upcast_ref(), "Try Again").is_none(),
            "Recovery retained failure UI or duplicated normal windows",
        )?;
        app.activate();
        check(
            app.windows().len() == 1 && app.active_window().as_ref() == Some(&normal),
            "Normal activation duplicated recovered window",
        )?;
        let normal_adw = normal.clone().downcast::<adw::ApplicationWindow>().unwrap();
        wait(|| normal_adw.visible_dialog().is_some())?;
        normal_adw.visible_dialog().unwrap().close();
        wait(|| normal_adw.visible_dialog().is_none())?;
        normal.close();
        wait(|| app.windows().is_empty())
            .map_err(|_| adw::glib::bool_error!("Recovered window did not close"))?;
        Ok(())
    }
}
