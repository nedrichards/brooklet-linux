//! Upper-edge feedback for inputs that do not get GTK's kinetic overscroll.
use std::cell::{Cell, OnceCell};

use adw::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct InboxBounce {
        pub offset: Cell<f32>,
        pub animation: OnceCell<adw::TimedAnimation>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InboxBounce {
        const NAME: &'static str = "BrookletInboxBounce";
        type Type = super::InboxBounce;
        type ParentType = adw::Bin;
    }
    impl ObjectImpl for InboxBounce {}
    impl WidgetImpl for InboxBounce {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            snapshot.push_clip(&gtk::graphene::Rect::new(
                0.0,
                0.0,
                widget.width() as f32,
                widget.height() as f32,
            ));
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(0.0, self.offset.get()));
            self.parent_snapshot(snapshot);
            snapshot.restore();
            snapshot.pop();
        }
    }
    impl BinImpl for InboxBounce {}
}

glib::wrapper! {
    pub struct InboxBounce(ObjectSubclass<imp::InboxBounce>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl InboxBounce {
    fn bounce(&self) {
        // Adwaita also skips animations when unmapped or animations are disabled.
        // Avoid restarting the animation on every notch of a spinning wheel.
        if let Some(animation) = self.imp().animation.get()
            && animation.state() != adw::AnimationState::Playing
        {
            animation.play();
        }
    }

    fn reset(&self) {
        if let Some(animation) = self.imp().animation.get() {
            animation.reset();
        }
    }
}

pub fn install(builder: &gtk::Builder) {
    let overlay: gtk::Overlay = builder.object("inbox_pull_overlay").unwrap();
    let scroller: gtk::ScrolledWindow = builder.object("inbox_scroller").unwrap();
    let bounce: InboxBounce = glib::Object::new();
    overlay.set_child(gtk::Widget::NONE);
    bounce.set_child(Some(&scroller));
    overlay.set_child(Some(&bounce));

    let weak = bounce.downgrade();
    let target = adw::CallbackAnimationTarget::new(move |progress| {
        if let Some(bounce) = weak.upgrade() {
            // A small pull down, followed by a gentle return to the same position.
            let offset = if progress >= 1.0 {
                0.0
            } else {
                48.0 * (std::f64::consts::PI * progress).sin() * (-2.0 * progress).exp()
            };
            bounce.imp().offset.set(offset as f32);
            bounce.queue_draw();
        }
    });
    let animation = adw::TimedAnimation::new(&bounce, 0.0, 1.0, 360, target);
    animation.set_easing(adw::Easing::Linear);
    bounce.imp().animation.set(animation).unwrap();
    let weak = bounce.downgrade();
    bounce.connect_unmap(move |_| {
        if let Some(bounce) = weak.upgrade() {
            bounce.reset();
        }
    });
    let weak = bounce.downgrade();
    scroller
        .vadjustment()
        .connect_value_changed(move |adjustment| {
            if adjustment.value() > adjustment.lower() + 0.5
                && let Some(bounce) = weak.upgrade()
            {
                bounce.reset();
            }
        });

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = scroller.downgrade();
    scroll.connect_scroll(move |controller, _, dy| {
        // Smooth scrolling and touch already receive GTK's native overscroll.
        if dy < 0.0 && controller.unit() == gtk::gdk::ScrollUnit::Wheel {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(scroller) = weak.upgrade() {
                    feedback_at_top(&scroller);
                }
            });
        }
        glib::Propagation::Proceed
    });
    scroller.add_controller(scroll);
}

pub fn feedback_at_top(scroller: &gtk::ScrolledWindow) {
    let adjustment = scroller.vadjustment();
    if adjustment.value() <= adjustment.lower() + 0.5
        && let Some(bounce) = scroller.parent().and_downcast::<InboxBounce>()
    {
        bounce.bounce();
    }
}

/// Check the mapped animation and reduced-motion path alongside pull-to-refresh.
pub fn smoke_test(builder: &gtk::Builder) -> Result<(), String> {
    let scroller: gtk::ScrolledWindow = builder.object("inbox_scroller").unwrap();
    let bounce = scroller.parent().and_downcast::<InboxBounce>().unwrap();
    let settings = bounce.settings();
    let animations = settings.is_gtk_enable_animations();
    settings.set_gtk_enable_animations(true);
    let result = (|| {
        scroller
            .vadjustment()
            .set_value(scroller.vadjustment().lower());
        feedback_at_top(&scroller);
        super::test_support::wait_for("visible Inbox overscroll", || {
            bounce.imp().offset.get() > 1.0
        })
        .map_err(|error| error.to_string())?;
        super::test_support::wait_for("Inbox overscroll to finish", || {
            bounce.imp().animation.get().unwrap().state() == adw::AnimationState::Finished
                && bounce.imp().offset.get() == 0.0
        })
        .map_err(|error| error.to_string())?;
        settings.set_gtk_enable_animations(false);
        feedback_at_top(&scroller);
        super::test_support::frames(&bounce, 3).map_err(|error| error.to_string())?;
        if bounce.imp().offset.get() != 0.0 {
            return Err("Inbox overscroll ignored reduced motion".into());
        }
        Ok(())
    })();
    settings.set_gtk_enable_animations(animations);
    result
}
