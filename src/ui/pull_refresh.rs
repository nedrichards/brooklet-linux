use std::{cell::RefCell, rc::Rc};

use adw::{gio, glib, prelude::*};
use brooklet::pull_refresh::{PullRefresh, PullState};

struct PullUi {
    pull: RefCell<PullRefresh>,
    scroller: glib::WeakRef<gtk::ScrolledWindow>,
    hint: glib::WeakRef<gtk::Label>,
    action: gio::SimpleAction,
}

impl PullUi {
    fn begin(&self) -> PullState {
        let at_top = self.scroller.upgrade().is_some_and(|scroller| {
            let adjustment = scroller.vadjustment();
            scroller.is_mapped() && adjustment.value() <= adjustment.lower() + 0.5
        });
        self.pull
            .borrow_mut()
            .begin(at_top, self.action.is_enabled())
    }

    fn update(&self, x: f64, y: f64) -> PullState {
        let state = self.pull.borrow_mut().update(x, y);
        if let Some(hint) = self.hint.upgrade() {
            hint.set_label(if state == PullState::Armed {
                "Release to refresh"
            } else {
                "Pull to refresh"
            });
            hint.set_visible(matches!(state, PullState::Pulling | PullState::Armed));
        }
        state
    }

    fn finish(&self, released: bool) {
        let refresh = self
            .pull
            .borrow_mut()
            .finish(released, self.action.is_enabled());
        self.hide();
        if refresh {
            self.action.activate(None);
        }
    }

    fn cancel(&self) {
        self.pull.borrow_mut().cancel();
        self.hide();
    }

    fn hide(&self) {
        if let Some(hint) = self.hint.upgrade() {
            hint.set_visible(false);
        }
    }
}

pub fn install(builder: &gtk::Builder, action: &gio::SimpleAction) {
    install_gesture(builder, action);
}

fn install_gesture(
    builder: &gtk::Builder,
    action: &gio::SimpleAction,
) -> (gtk::GestureDrag, Rc<PullUi>) {
    let overlay: gtk::Overlay = builder
        .object("inbox_pull_overlay")
        .expect("inbox_pull_overlay");
    let scroller: gtk::ScrolledWindow = builder.object("inbox_scroller").expect("inbox_scroller");
    let hint: gtk::Label = builder.object("inbox_pull_hint").expect("inbox_pull_hint");
    let state = Rc::new(PullUi {
        pull: RefCell::new(PullRefresh::default()),
        scroller: scroller.downgrade(),
        hint: hint.downgrade(),
        action: action.clone(),
    });
    let gesture = gtk::GestureDrag::new();
    gesture.set_touch_only(true);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    gesture.connect_drag_begin({
        let state = state.clone();
        move |gesture, _, _| {
            // Reserve touches beginning at the top before the scroller claims them.
            // Denying an upward/horizontal drag (or a tap on release) lets GTK
            // replay its begin event and retain ordinary scrolling/row activation.
            let sequence_state = if state.begin() == PullState::Pending {
                gtk::EventSequenceState::Claimed
            } else {
                gtk::EventSequenceState::Denied
            };
            gesture.set_state(sequence_state);
        }
    });
    gesture.connect_drag_update({
        let state = state.clone();
        move |gesture, x, y| {
            if state.update(x, y) == PullState::Idle {
                gesture.set_state(gtk::EventSequenceState::Denied);
            }
        }
    });
    gesture.connect_drag_end({
        let state = state.clone();
        move |gesture, x, y| {
            let pull_state = state.update(x, y);
            // drag-end also fires on cancellation or additional touch points.
            let released = gesture
                .current_event()
                .is_some_and(|event| event.event_type() == gtk::gdk::EventType::TouchEnd);
            if pull_state == PullState::Pending {
                gesture.set_state(gtk::EventSequenceState::Denied);
            }
            state.finish(released);
        }
    });
    gesture.connect_cancel({
        let state = state.clone();
        move |_, _| state.cancel()
    });
    overlay.connect_unmap({
        let state = state.clone();
        move |_| state.cancel()
    });
    action.connect_enabled_notify({
        let state = Rc::downgrade(&state);
        move |action| {
            if !action.is_enabled()
                && let Some(state) = state.upgrade()
            {
                state.cancel();
            }
        }
    });
    overlay.add_controller(gesture.clone());
    (gesture, state)
}

/// Exercise the production Blueprint/controller/action wiring under a virtual display.
/// Gesture signals and release handling are driven directly, without a physical device.
pub fn smoke_test(builder: &gtk::Builder) -> Result<(), String> {
    use std::cell::Cell;
    let calls = Rc::new(Cell::new(0));
    let action = gio::SimpleAction::new("pull-test-sync", None);
    action.connect_activate({
        let calls = calls.clone();
        move |action, _| {
            calls.set(calls.get() + 1);
            action.set_enabled(false);
        }
    });
    let (gesture, state) = install_gesture(builder, &action);
    let hint: gtk::Label = builder.object("inbox_pull_hint").unwrap();
    let scroller: gtk::ScrolledWindow = builder.object("inbox_scroller").unwrap();
    if !gesture.is_touch_only() || gesture.propagation_phase() != gtk::PropagationPhase::Capture {
        return Err("pull controller must capture touchscreen events only".into());
    }
    scroller.set_visible(true);
    if state.begin() != PullState::Pending {
        return Err("mapped Inbox at its upper edge must accept a pull".into());
    }
    gesture.emit_by_name::<()>("drag-update", &[&0.0_f64, &80.0_f64]);
    if !hint.is_visible() || hint.label() != "Release to refresh" || calls.get() != 0 {
        return Err("armed pull must show feedback without syncing".into());
    }
    state.finish(true);
    state.finish(true);
    if calls.get() != 1 || hint.is_visible() {
        return Err("release must hide feedback and activate sync exactly once".into());
    }
    action.set_enabled(true);
    state.begin();
    state.update(0.0, 80.0);
    action.set_enabled(false);
    action.set_enabled(true);
    state.finish(true);
    if calls.get() != 1 || hint.is_visible() {
        return Err("a concurrent sync must cancel an armed pull".into());
    }
    state.begin();
    state.update(0.0, 80.0);
    gesture.emit_by_name::<()>("cancel", &[&None::<gtk::gdk::EventSequence>]);
    state.finish(true);
    if calls.get() != 1 || hint.is_visible() {
        return Err("cancelled touch must not sync".into());
    }
    state.begin();
    state.update(0.0, 80.0);
    // GTK may emit drag-end for reasons other than a finger release.
    gesture.emit_by_name::<()>("drag-end", &[&0.0_f64, &80.0_f64]);
    if calls.get() != 1 || hint.is_visible() {
        return Err("drag-end without a touch release must not sync".into());
    }
    let adjustment = scroller.vadjustment();
    adjustment.configure(40.0, 0.0, 1000.0, 1.0, 100.0, 100.0);
    if state.begin() != PullState::Idle {
        return Err("a pull away from the upper edge must remain ordinary scrolling".into());
    }
    adjustment.set_value(0.0);
    state.begin();
    state.update(0.0, 80.0);
    scroller.set_visible(false);
    state.finish(true);
    let overlay: gtk::Overlay = builder.object("inbox_pull_overlay").unwrap();
    if calls.get() != 1 || hint.is_visible() || overlay.is_visible() {
        return Err("hiding the Inbox must cancel the pull and its overlay".into());
    }
    Ok(())
}
