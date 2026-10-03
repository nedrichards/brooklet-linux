//! Viewport-driven images. Widget and signal captures are weak; jobs die with the session.
use adw::{gio, glib, prelude::*};
use brooklet::controller::AppController;
use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

const RESIDENT_BYTES: usize = 64 * 1024 * 1024;
const IMAGE_PIXELS: u64 = 2 * 1024 * 1024;
const SOURCE_PIXELS: u64 = 32 * 1024 * 1024;

// GLib can destroy a cancelled task after Job::drop's runtime guard has exited.
// Keep the Tokio context attached to the future's poll AND destruction, since
// Glycin's destructors spawn asynchronous cleanup tasks too.
struct RuntimeFuture<F> {
    future: Option<Pin<Box<F>>>,
    runtime: tokio::runtime::Handle,
}

impl<F> RuntimeFuture<F> {
    fn new(runtime: tokio::runtime::Handle, future: F) -> Self {
        Self {
            future: Some(Box::pin(future)),
            runtime,
        }
    }
}

impl<F: Future> Future for RuntimeFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let _entered = this.runtime.enter();
        this.future
            .as_mut()
            .expect("future remains present")
            .as_mut()
            .poll(context)
    }
}

impl<F> Drop for RuntimeFuture<F> {
    fn drop(&mut self) {
        let _entered = self.runtime.enter();
        // Fields are otherwise dropped after this guard, outside Tokio.
        drop(self.future.take());
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Waiting,
    Loading,
    Loaded(usize),
    Deferred,
    Failed,
}

struct Slot {
    url: String,
    alt: String,
    container: glib::WeakRef<gtk::Box>,
    placeholder: Vec<gtk::Widget>,
    state: State,
}

struct Job {
    fetch: tokio::task::AbortHandle,
    cancel: gio::Cancellable,
    decode: Option<glib::JoinHandle<()>>,
    runtime: tokio::runtime::Handle,
}

impl Drop for Job {
    fn drop(&mut self) {
        // Gio cancellation calls Glycin's cleanup synchronously, including
        // spawning Tokio cleanup tasks. Enter the backend context for it too.
        let _entered = self.runtime.enter();
        self.fetch.abort();
        self.cancel.cancel();
        if let Some(task) = &self.decode {
            task.abort();
        }
    }
}

pub struct Images {
    controller: Arc<AppController>,
    content: glib::WeakRef<gtk::Box>,
    scroller: glib::WeakRef<gtk::ScrolledWindow>,
    generation: Rc<Cell<u64>>,
    expected: u64,
    entry_id: i64,
    restoring: Rc<Cell<bool>>,
    slots: RefCell<Vec<Slot>>,
    jobs: RefCell<std::collections::HashMap<usize, Job>>,
    queued: Cell<bool>,
    resident: Cell<usize>,
    signals: RefCell<Vec<glib::SignalHandlerId>>,
    adjustment: glib::WeakRef<gtk::Adjustment>,
}

impl Drop for Images {
    fn drop(&mut self) {
        if let Some(adjustment) = self.adjustment.upgrade() {
            for signal in self.signals.get_mut().drain(..) {
                adjustment.disconnect(signal);
            }
        }
        self.jobs.get_mut().clear();
    }
}

impl Images {
    pub fn new(
        controller: Arc<AppController>,
        content: &gtk::Box,
        scroller: &gtk::ScrolledWindow,
        generation: Rc<Cell<u64>>,
        entry_id: i64,
        restoring: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        let images = Rc::new(Self {
            controller,
            content: content.downgrade(),
            scroller: scroller.downgrade(),
            expected: generation.get(),
            generation,
            entry_id,
            restoring,
            slots: RefCell::new(Vec::new()),
            jobs: RefCell::new(std::collections::HashMap::new()),
            queued: Cell::new(false),
            resident: Cell::new(0),
            signals: RefCell::new(Vec::new()),
            adjustment: scroller.vadjustment().downgrade(),
        });
        // Neither adjustment signal owns the session or any widgets.
        let weak = Rc::downgrade(&images);
        let signal = scroller.vadjustment().connect_value_changed(move |_| {
            if let Some(images) = weak.upgrade() {
                for slot in images.slots.borrow_mut().iter_mut() {
                    if slot.state == State::Deferred {
                        slot.state = State::Waiting;
                    }
                }
                images.queue();
            }
        });
        images.signals.borrow_mut().push(signal);
        let weak = Rc::downgrade(&images);
        let signal = scroller.vadjustment().connect_changed(move |_| {
            if let Some(images) = weak.upgrade() {
                images.queue();
            }
        });
        images.signals.borrow_mut().push(signal);
        images
    }

    pub fn add(self: &Rc<Self>, slots: Vec<super::reader::ImageSlot>) {
        self.slots
            .borrow_mut()
            .extend(slots.into_iter().map(|slot| {
                Slot {
                    url: slot.url,
                    alt: slot.alt,
                    placeholder: slot
                        .container
                        .observe_children()
                        .iter::<adw::glib::Object>()
                        .filter_map(Result::ok)
                        .filter_map(|object| object.downcast::<gtk::Widget>().ok())
                        .collect(),
                    container: slot.container.downgrade(),
                    state: State::Waiting,
                }
            }));
        self.queue();
    }

    pub fn queue(self: &Rc<Self>) {
        if self.queued.replace(true) {
            return;
        }
        let Some(scroller) = self.scroller.upgrade() else {
            self.queued.set(false);
            return;
        };
        let weak = Rc::downgrade(self);
        scroller.add_tick_callback(move |_, _| {
            if let Some(images) = weak.upgrade() {
                images.queued.set(false);
                images.start_visible();
            }
            glib::ControlFlow::Break
        });
    }

    fn distance(&self, container: &gtk::Box, scroller: &gtk::ScrolledWindow) -> Option<f32> {
        if !container.is_mapped() || container.height() == 0 {
            return None;
        }
        let bounds = container.compute_bounds(scroller)?;
        let viewport = scroller.height() as f32;
        Some(if bounds.y() > viewport {
            bounds.y() - viewport
        } else if bounds.y() + bounds.height() < 0.0 {
            -(bounds.y() + bounds.height())
        } else {
            0.0
        })
    }

    fn start_visible(self: &Rc<Self>) {
        if self.generation.get() != self.expected || self.restoring.get() {
            return;
        }
        let Some(scroller) = self
            .scroller
            .upgrade()
            .filter(|scroller| scroller.is_mapped())
        else {
            return;
        };
        // Cancel work whose image is no longer near the viewport.
        let obsolete = self
            .jobs
            .borrow()
            .keys()
            .copied()
            .filter(|index| {
                self.slots.borrow()[*index]
                    .container
                    .upgrade()
                    .and_then(|container| self.distance(&container, &scroller))
                    .is_none_or(|distance| distance > scroller.height() as f32)
            })
            .collect::<Vec<_>>();
        for index in obsolete {
            self.jobs.borrow_mut().remove(&index);
            self.slots.borrow_mut()[index].state = State::Waiting;
        }
        let mut candidates = self
            .slots
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                if slot.state != State::Waiting {
                    return None;
                }
                let distance = self.distance(&slot.container.upgrade()?, &scroller)?;
                (distance <= scroller.height() as f32).then_some((index, distance))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (index, _) in candidates {
            // Bound bytes before fetching, rather than queue downloaded buffers behind decoders.
            if self.jobs.borrow().len() >= 2 {
                break;
            }
            self.slots.borrow_mut()[index].state = State::Loading;
            let url = self.slots.borrow()[index].url.clone();
            let cancel = gio::Cancellable::new();
            let weak = Rc::downgrade(self);
            let request = self.controller.fetch_image(url, move |result| {
                let Some(images) = weak.upgrade() else {
                    return;
                };
                if !images.jobs.borrow().contains_key(&index) {
                    return;
                }
                let Ok(bytes) = result else {
                    images.finish(index, None);
                    return;
                };
                let cancel = images.jobs.borrow()[&index].cancel.clone();
                let controller = images.controller.clone();
                let handle = controller.backend_handle();
                let width = images.scroller.upgrade().map_or(760, |scroller| {
                    scroller.width().max(1) * scroller.scale_factor()
                }) as u32;
                let weak = Rc::downgrade(&images);
                let task = glib::MainContext::default().spawn_local(async move {
                    let decode = RuntimeFuture::new(handle, async move {
                        let decoders = controller.image_decoders();
                        let _permit = decoders
                            .acquire()
                            .await
                            .expect("image decoders remain open");
                        if cancel.is_cancelled() {
                            return None;
                        }
                        let mut loader = glycin::Loader::new_vec(bytes);
                        // Glycin's helper callbacks also need a Tokio context, not
                        // only the future we poll on the GTK thread.
                        loader.main_context_selector(glycin::MainContextSelector::Managed);
                        loader.cancellable(cancel.clone()).limits(
                            glycin::Limits::default()
                                .timeout(Duration::from_secs(15))
                                .max_dimensions((4096, 4096)),
                        );
                        let mut image = loader.load().await.ok()?;
                        let details = image.details();
                        let (width, height) =
                            decode_size(details.width(), details.height(), width)?;
                        let frame = image
                            .specific_frame(glycin::FrameRequest::new().scale(width, height))
                            .await
                            .ok()?;
                        if u64::from(frame.width()) * u64::from(frame.height()) > IMAGE_PIXELS
                            || frame.buf_slice().len() > RESIDENT_BYTES / 2
                        {
                            return None;
                        }
                        Some((frame.texture(), frame.buf_slice().len()))
                    });
                    let result = decode.await;
                    if let Some(images) = weak.upgrade() {
                        images.finish(index, result);
                    }
                });
                images
                    .jobs
                    .borrow_mut()
                    .get_mut(&index)
                    .expect("job remains pending")
                    .decode = Some(task);
            });
            self.jobs.borrow_mut().insert(
                index,
                Job {
                    fetch: request,
                    cancel,
                    decode: None,
                    runtime: self.controller.backend_handle(),
                },
            );
        }
    }

    fn finish(self: &Rc<Self>, index: usize, result: Option<(gtk::gdk::Texture, usize)>) {
        // Removing a completed job must not destroy the source currently running.
        if let Some(mut job) = self.jobs.borrow_mut().remove(&index) {
            job.decode.take();
        }
        if self.generation.get() != self.expected {
            return;
        }
        let Some((texture, bytes)) = result else {
            self.slots.borrow_mut()[index].state = State::Failed;
            self.queue();
            return;
        };
        let (Some(content), Some(scroller), Some(container)) = (
            self.content.upgrade(),
            self.scroller.upgrade(),
            self.slots.borrow()[index].container.upgrade(),
        ) else {
            return;
        };
        let scroll = scroller.vadjustment().value();
        let place = super::reader::position_from_offset(&content, self.entry_id, scroll as i32);
        // Evict distant textures before installing a new one; retain their layout height.
        let mut slots = self.slots.borrow_mut();
        let mut distant = slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| match slot.state {
                State::Loaded(bytes) => Some((
                    index,
                    bytes,
                    slot.container
                        .upgrade()
                        .and_then(|container| self.distance(&container, &scroller))
                        .unwrap_or(f32::MAX),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        distant.sort_by(|a, b| b.2.total_cmp(&a.2));
        for (old, cost, distance) in distant {
            if self.resident.get() + bytes <= RESIDENT_BYTES {
                break;
            }
            if distance == 0.0 {
                continue;
            }
            if let Some(container) = slots[old].container.upgrade() {
                container.set_height_request(container.height());
                while let Some(child) = container.first_child() {
                    container.remove(&child);
                }
                for placeholder in &slots[old].placeholder {
                    container.append(placeholder);
                }
            }
            slots[old].state = State::Deferred;
            self.resident.set(self.resident.get() - cost);
        }
        if self.resident.get() + bytes > RESIDENT_BYTES {
            slots[index].state = State::Deferred;
        } else {
            container.set_height_request(-1);
            while let Some(child) = container.first_child() {
                container.remove(&child);
            }
            container.append(&super::reader::picture(&texture, &slots[index].alt));
            slots[index].state = State::Loaded(bytes);
            self.resident.set(self.resident.get() + bytes);
        }
        drop(slots);
        super::reader::preserve_after_image(
            &content,
            &scroller,
            self.generation.clone(),
            self.expected,
            scroll,
            place,
        );
        self.queue();
    }
}

fn decode_size(width: u32, height: u32, display_width: u32) -> Option<(u32, u32)> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > SOURCE_PIXELS {
        return None;
    }
    let scale = (display_width.clamp(1, 1600) as f64 / width as f64)
        .min(1.0)
        .min((IMAGE_PIXELS as f64 / pixels as f64).sqrt())
        .min(4096.0 / width.max(height) as f64);
    Some((
        (width as f64 * scale).floor().max(1.0) as u32,
        (height as f64 * scale).floor().max(1.0) as u32,
    ))
}

/// Synthetic cached gallery: exercises scheduling and real decoding without network.
pub fn smoke_test(controller: Arc<AppController>) -> Result<(), glib::BoolError> {
    let window = adw::Window::builder()
        .default_width(400)
        .default_height(400)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let scroller = gtk::ScrolledWindow::builder().child(&content).build();
    window.set_content(Some(&scroller));
    let generation = Rc::new(Cell::new(1));
    let images = Images::new(
        controller,
        &content,
        &scroller,
        generation,
        1,
        Rc::new(Cell::new(false)),
    );
    let slots = (0..40)
        .map(|index| {
            let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
            container.set_height_request(200);
            container.append(&gtk::Label::new(Some("Synthetic cached image")));
            content.append(&container);
            super::reader::ImageSlot {
                url: format!("https://example.invalid/image/{index}"),
                container,
                alt: "Synthetic image".into(),
            }
        })
        .collect();
    images.add(slots);
    window.present();
    let main_loop = glib::MainLoop::new(None, false);
    let failed = Rc::new(Cell::new(true));
    let concurrency = Rc::new(Cell::new(0));
    window.add_tick_callback({
        let weak = Rc::downgrade(&images);
        let failed = failed.clone();
        let concurrency = concurrency.clone();
        let main_loop = main_loop.clone();
        move |_, _| {
            let Some(images) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            concurrency.set(concurrency.get().max(images.jobs.borrow().len()));
            if images
                .slots
                .borrow()
                .iter()
                .any(|slot| matches!(slot.state, State::Loaded(_)))
            {
                failed.set(
                    images.slots.borrow()[39].state != State::Waiting
                        || concurrency.get() > 2
                        || images.resident.get() > RESIDENT_BYTES,
                );
                main_loop.quit();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        }
    });
    let expired = Rc::new(Cell::new(false));
    let deadline = glib::timeout_add_local_once(Duration::from_secs(5), {
        let main_loop = main_loop.clone();
        let expired = expired.clone();
        move || {
            expired.set(true);
            main_loop.quit();
        }
    });
    main_loop.run();
    if !expired.get() {
        deadline.remove();
    }
    let weak = Rc::downgrade(&images);
    drop(images);
    window.destroy();
    if weak.upgrade().is_some() || failed.get() {
        return Err(glib::bool_error!(
            "Cached gallery decoding, viewport bounds, or session teardown failed"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CleanupProbe {
        dropped: Rc<Cell<bool>>,
        ready: bool,
    }

    impl Future for CleanupProbe {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            assert!(tokio::runtime::Handle::try_current().is_ok());
            if self.ready {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }

    impl Drop for CleanupProbe {
        fn drop(&mut self) {
            // Model Glycin's destructor: cleanup must be able to spawn even
            // when GLib releases a task outside any Tokio poll or cancel guard.
            tokio::spawn(async {});
            self.dropped.set(true);
        }
    }

    #[test]
    fn unpolled_decode_destroys_future_inside_runtime() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dropped = Rc::new(Cell::new(false));
        let future = RuntimeFuture::new(
            runtime.handle().clone(),
            CleanupProbe {
                dropped: dropped.clone(),
                ready: false,
            },
        );
        drop(future);
        assert!(dropped.get());
        assert!(tokio::runtime::Handle::try_current().is_err());
    }

    #[test]
    fn cancelled_job_destroys_decode_inside_runtime() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let context = glib::MainContext::new();
        let _owner = context.acquire().unwrap();
        let dropped = Rc::new(Cell::new(false));
        let task = context.spawn_local(RuntimeFuture::new(
            runtime.handle().clone(),
            CleanupProbe {
                dropped: dropped.clone(),
                ready: false,
            },
        ));
        context.iteration(false);
        let job = Job {
            fetch: runtime.spawn(std::future::pending::<()>()).abort_handle(),
            cancel: gio::Cancellable::new(),
            decode: Some(task),
            runtime: runtime.handle().clone(),
        };
        assert!(tokio::runtime::Handle::try_current().is_err());
        drop(job);
        assert!(dropped.get());
        assert!(tokio::runtime::Handle::try_current().is_err());
    }

    #[test]
    fn completed_decode_destroys_future_inside_runtime() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let context = glib::MainContext::new();
        let _owner = context.acquire().unwrap();
        let dropped = Rc::new(Cell::new(false));
        let task = context.spawn_local(RuntimeFuture::new(
            runtime.handle().clone(),
            CleanupProbe {
                dropped: dropped.clone(),
                ready: true,
            },
        ));
        context.iteration(false);
        drop(task);
        assert!(dropped.get());
        assert!(tokio::runtime::Handle::try_current().is_err());
    }

    #[test]
    fn decode_dimensions_bound_pixels_and_preserve_aspect_ratio() {
        assert_eq!(decode_size(800, 400, 400), Some((400, 200)));
        assert_eq!(decode_size(100, 50, 760), Some((100, 50)));
        assert_eq!(decode_size(0, 50, 760), None);
        assert_eq!(decode_size(10000, 10000, 760), None);
        for (width, height) in [(6000, 4000), (1000, 30000), (30000, 1000)] {
            let (x, y) = decode_size(width, height, 1600).unwrap();
            assert!(u64::from(x) * u64::from(y) <= IMAGE_PIXELS);
            assert!(x <= 4096 && y <= 4096);
        }
    }
}
