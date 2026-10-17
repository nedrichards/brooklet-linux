//! A reproducible desktop for the isolated GTK CLI regression fixtures.
use adw::{glib, prelude::*};
use std::{cell::Cell, rc::Rc, time::Duration};

pub struct Desktop {
    settings: gtk::Settings,
    font: String,
    dpi: i32,
    animations: bool,
}

impl Desktop {
    pub fn install() -> Self {
        let settings = gtk::Settings::default().expect("GTK regression display");
        let desktop = Self {
            font: settings.property("gtk-font-name"),
            dpi: settings.property("gtk-xft-dpi"),
            animations: settings.is_gtk_enable_animations(),
            settings,
        };
        // The host's enlarged text changes sp breakpoints and can collapse a
        // fixture intended to exercise two panes. Test scaling explicitly,
        // rather than inheriting arbitrary desktop preferences.
        desktop
            .settings
            .set_property("gtk-font-name", "Adwaita Sans 11");
        desktop.settings.set_property("gtk-xft-dpi", 96 * 1024);
        desktop.settings.set_gtk_enable_animations(false);
        desktop
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        self.settings.set_property("gtk-font-name", &self.font);
        self.settings.set_property("gtk-xft-dpi", self.dpi);
        self.settings.set_gtk_enable_animations(self.animations);
    }
}

pub fn wait_for(description: &str, predicate: impl Fn() -> bool) -> Result<(), glib::BoolError> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !predicate() {
        if std::time::Instant::now() >= deadline {
            return Err(glib::bool_error!("Timed out waiting for {description}"));
        }
        pump(Duration::from_millis(10));
    }
    Ok(())
}

pub fn pump(duration: Duration) {
    let loop_ = glib::MainLoop::new(None, false);
    let quit = loop_.clone();
    glib::timeout_add_local_once(duration, move || quit.quit());
    loop_.run();
}

pub fn frames(widget: &impl IsA<gtk::Widget>, count: u32) -> Result<(), glib::BoolError> {
    let seen = Rc::new(Cell::new(0));
    let tick = widget.add_tick_callback({
        let seen = seen.clone();
        move |_, _| {
            seen.set(seen.get() + 1);
            glib::ControlFlow::Continue
        }
    });
    let result = wait_for("mapped GTK frames", || {
        widget.is_mapped() && seen.get() >= count
    });
    tick.remove();
    result
}
