//! Coalesce UI reload requests: one running operation and one latest follow-up.
use std::{cell::RefCell, rc::Rc};

type Operation = Box<dyn FnOnce(Box<dyn FnOnce()>)>;
#[derive(Default)]
struct State {
    running: bool,
    pending: Option<Operation>,
}

#[derive(Default)]
pub struct ReloadQueue {
    state: RefCell<State>,
}
impl ReloadQueue {
    /// The operation must call `done` on success, failure, or stale completion.
    pub fn request(self: &Rc<Self>, operation: impl FnOnce(Box<dyn FnOnce()>) + 'static) {
        let operation: Operation = Box::new(operation);
        {
            let mut state = self.state.borrow_mut();
            if state.running {
                state.pending = Some(operation);
                return;
            }
            state.running = true;
        }
        self.start(operation);
    }

    fn start(self: &Rc<Self>, operation: Operation) {
        let queue = self.clone();
        operation(Box::new(move || {
            let pending = {
                let mut state = queue.state.borrow_mut();
                let pending = state.pending.take();
                state.running = pending.is_some();
                pending
            };
            if let Some(pending) = pending {
                queue.start(pending);
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn burst_runs_only_first_and_latest_and_accepts_later_requests() {
        let queue = Rc::new(ReloadQueue::default());
        let started = Rc::new(RefCell::new(Vec::new()));
        let completions = Rc::new(RefCell::new(Vec::new()));
        for id in 0..100 {
            let started = started.clone();
            let completions = completions.clone();
            queue.request(move |done| {
                started.borrow_mut().push(id);
                completions.borrow_mut().push(done);
            });
        }
        assert_eq!(*started.borrow(), [0]);
        let done = completions.borrow_mut().pop().unwrap();
        done();
        assert_eq!(*started.borrow(), [0, 99]);
        let done = completions.borrow_mut().pop().unwrap();
        done();
        let started = started.clone();
        queue.request(move |done| {
            started.borrow_mut().push(100);
            done();
        });
        assert!(!queue.state.borrow().running);
    }

    #[test]
    fn independent_views_and_synchronous_completions_do_not_block_each_other() {
        let first = Rc::new(ReloadQueue::default());
        let second = Rc::new(ReloadQueue::default());
        let finish = Rc::new(RefCell::new(None));
        first.request({
            let finish = finish.clone();
            move |done| *finish.borrow_mut() = Some(done)
        });
        let calls = Rc::new(RefCell::new(0));
        for _ in 0..3 {
            let calls = calls.clone();
            second.request(move |done| {
                *calls.borrow_mut() += 1;
                done();
            });
        }
        assert_eq!(*calls.borrow(), 3);
        assert!(first.state.borrow().running);
        finish.borrow_mut().take().unwrap()();
        assert!(!first.state.borrow().running);
    }
}
