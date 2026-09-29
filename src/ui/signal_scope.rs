//! Disconnect dialog-owned callbacks even when sibling controls capture each other.
use adw::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub struct SignalScope {
    closed: Cell<bool>,
    signals: RefCell<Vec<(glib::WeakRef<glib::Object>, glib::SignalHandlerId)>>,
}

impl SignalScope {
    pub fn for_dialog(dialog: &impl IsA<adw::Dialog>) -> Rc<Self> {
        let dialog = dialog.as_ref();
        let scope = Rc::new(Self {
            closed: Cell::new(false),
            signals: RefCell::new(Vec::new()),
        });
        dialog.connect_closed({
            let scope = scope.clone();
            move |_| scope.close()
        });
        dialog.connect_destroy({
            let scope = scope.clone();
            move |_| scope.close()
        });
        scope
    }

    pub fn track(&self, object: &impl IsA<glib::Object>, signal: glib::SignalHandlerId) {
        if self.closed.get() {
            object.disconnect(signal);
        } else {
            self.signals
                .borrow_mut()
                .push((object.as_ref().downgrade(), signal));
        }
    }

    fn close(&self) {
        self.closed.set(true);
        for (object, signal) in self.signals.borrow_mut().drain(..) {
            if let Some(object) = object.upgrade() {
                object.disconnect(signal);
            }
        }
    }
}

pub fn smoke_test() -> Result<(), glib::BoolError> {
    let dialog = adw::Dialog::new();
    let button = gtk::Button::with_label("Synthetic dialog control");
    dialog.set_child(Some(&button));
    let scope = SignalScope::for_dialog(&dialog);
    scope.track(
        &button,
        button.connect_clicked({
            let retained = button.clone();
            move |_| retained.set_sensitive(false)
        }),
    );
    let weak = button.downgrade();
    dialog.emit_by_name::<()>("closed", &[]);
    drop(button);
    drop(dialog);
    if weak.upgrade().is_some() {
        return Err(glib::bool_error!(
            "Closed dialog retained its control callbacks"
        ));
    }
    Ok(())
}
