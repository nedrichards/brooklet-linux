//! Outgoing changes are independent of article freshness and window activity.

use std::{future::Future, pin::Pin, time::Duration};

use tokio::{sync::watch, time::Instant};

use crate::{error::BrookletError, model::FailureKind};

const DEBOUNCE: Duration = Duration::from_secs(2);
const MAX_DEBOUNCE: Duration = Duration::from_secs(10);
const FIRST_RETRY: Duration = Duration::from_secs(5);
const MAX_RETRY: Duration = Duration::from_secs(30 * 60);

pub type LocalWrite = (
    Pin<Box<dyn Future<Output = Result<(), BrookletError>> + Send>>,
    tokio::sync::oneshot::Sender<Result<(), BrookletError>>,
);

/// Submit on the UI thread so persistence follows user action order, rather than
/// the order in which independently spawned tasks happen to reach SQLite.
pub fn local_writes(
    runtime: &tokio::runtime::Handle,
    notifications: watch::Sender<u64>,
) -> tokio::sync::mpsc::UnboundedSender<LocalWrite> {
    let (sender, mut writes) = tokio::sync::mpsc::unbounded_channel::<LocalWrite>();
    runtime.spawn(async move {
        while let Some((write, reply)) = writes.recv().await {
            let result = write.await;
            if result.is_ok() {
                notifications.send_modify(|generation| *generation = generation.wrapping_add(1));
            }
            let _ = reply.send(result);
        }
    });
    sender
}

/// A watch channel coalesces notifications without dropping writes made in flight.
/// Check persisted work on startup, then sleep until a local commit or a retry.
/// New edits never shorten failure backoff, avoiding request storms when offline.
pub async fn deliver<F, Fut>(mut changes: watch::Receiver<u64>, mut flush: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, BrookletError>>,
{
    let mut retry = FIRST_RETRY;
    let mut retry_at = None;
    loop {
        if let Some(deadline) = retry_at.take() {
            tokio::time::sleep_until(deadline).await;
        } else {
            let first = Instant::now();
            let limit = first + MAX_DEBOUNCE;
            let mut deadline = first + DEBOUNCE;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    changed = changes.changed() => {
                        if changed.is_err() { return; }
                        deadline = (Instant::now() + DEBOUNCE).min(limit);
                    }
                }
            }
        }

        // Mark notifications preceding this snapshot as consumed. Commits made
        // while uploading remain visible to changed(), even after a successful flush.
        changes.borrow_and_update();
        match flush().await {
            Ok(more) => {
                retry = FIRST_RETRY;
                if more {
                    continue;
                }
                if changes.changed().await.is_err() {
                    return;
                }
            }
            Err(error) => {
                tracing::warn!(failure_kind = ?error.failure_kind(), "outgoing delivery failed; changes remain queued");
                let delay = if error.failure_kind() == FailureKind::Retryable {
                    retry
                } else {
                    // Keep credentials/certificate/request failures inspectable,
                    // but do not hammer a server that cannot accept the request.
                    MAX_RETRY
                };
                retry_at = Some(Instant::now() + delay);
                retry = retry.saturating_mul(2).min(MAX_RETRY);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    async fn settle() {
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }

    async fn advance(seconds: u64) {
        tokio::time::advance(Duration::from_secs(seconds)).await;
        settle().await;
    }

    fn notify(sender: &watch::Sender<u64>) {
        sender.send_modify(|value| *value += 1);
    }

    fn failure(status: u16) -> BrookletError {
        BrookletError::Http {
            status,
            kind: crate::model::classify_http_status(status),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn startup_checks_persisted_work_and_bursts_wait_for_two_quiet_seconds() {
        let (sender, receiver) = watch::channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let task = tokio::spawn(deliver(receiver, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Ok(false) }
        }));
        settle().await;
        advance(2).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        notify(&sender);
        settle().await;
        advance(1).await;
        notify(&sender);
        settle().await;
        advance(1).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        advance(1).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn continuous_edits_cannot_postpone_delivery_past_ten_seconds() {
        let (sender, receiver) = watch::channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let task = tokio::spawn(deliver(receiver, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Ok(false) }
        }));
        settle().await;
        for _ in 0..9 {
            advance(1).await;
            notify(&sender);
            settle().await;
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
        advance(1).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn transient_failures_back_off_despite_edits_and_success_resets_backoff() {
        let (sender, receiver) = watch::channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let task = tokio::spawn(deliver(receiver, move || {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 2 {
                    Ok(false)
                } else {
                    Err(failure(503))
                }
            }
        }));
        settle().await;
        advance(2).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        notify(&sender);
        advance(4).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        advance(1).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        advance(9).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        advance(1).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        notify(&sender);
        settle().await;
        advance(2).await;
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        advance(5).await;
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn exponential_retry_is_capped_at_thirty_minutes() {
        let (_sender, receiver) = watch::channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let task = tokio::spawn(deliver(receiver, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Err(failure(503)) }
        }));
        settle().await;
        advance(2).await;
        for (index, delay) in [5, 10, 20, 40, 80, 160, 320, 640, 1280, 1800, 1800]
            .into_iter()
            .enumerate()
        {
            advance(delay - 1).await;
            assert_eq!(calls.load(Ordering::SeqCst), index + 1);
            advance(1).await;
            assert_eq!(calls.load(Ordering::SeqCst), index + 2);
        }
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn authentication_failures_use_slow_retries_even_with_new_edits() {
        let (sender, receiver) = watch::channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let task = tokio::spawn(deliver(receiver, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Err(failure(401)) }
        }));
        settle().await;
        advance(2).await;
        notify(&sender);
        advance(1799).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        advance(1).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_commit_during_upload_is_not_lost_even_if_flush_reports_empty() {
        let (sender, receiver) = watch::channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let (release, gated) = tokio::sync::oneshot::channel();
        let mut gate = Some(gated);
        let task = tokio::spawn(deliver(receiver, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let gate = gate.take();
            async move {
                if let Some(gate) = gate {
                    gate.await.unwrap();
                }
                Ok(false)
            }
        }));
        settle().await;
        advance(2).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        notify(&sender);
        release.send(()).unwrap();
        settle().await;
        advance(2).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn newly_pending_work_drains_again_without_a_second_notification() {
        let (_sender, receiver) = watch::channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let task = tokio::spawn(deliver(receiver, move || {
            let call = counter.fetch_add(1, Ordering::SeqCst);
            async move { Ok(call == 0) }
        }));
        settle().await;
        advance(2).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        advance(2).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test]
    async fn shutdown_barrier_waits_for_writes_even_after_their_callbacks_are_dropped() {
        let (notifications, _) = watch::channel(0);
        let sender = local_writes(&tokio::runtime::Handle::current(), notifications);
        let (release, gated) = tokio::sync::oneshot::channel();
        let (reply, callback) = tokio::sync::oneshot::channel();
        let committed = Arc::new(AtomicUsize::new(0));
        let written = committed.clone();
        sender
            .send((
                Box::pin(async move {
                    gated.await.unwrap();
                    written.store(1, Ordering::SeqCst);
                    Ok(())
                }),
                reply,
            ))
            .unwrap();
        drop(callback);
        let (reply, mut drained) = tokio::sync::oneshot::channel();
        sender.send((Box::pin(async { Ok(()) }), reply)).unwrap();
        drop(sender);
        settle().await;
        assert!(matches!(
            drained.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        drained.await.unwrap().unwrap();
        assert_eq!(committed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn local_commits_keep_invocation_order_and_notify_only_after_success() {
        let (notifications, mut changes) = watch::channel(0);
        let sender = local_writes(&tokio::runtime::Handle::current(), notifications);
        let order = Arc::new(Mutex::new(Vec::new()));
        let (release, gated) = tokio::sync::oneshot::channel();
        let (first_reply, first) = tokio::sync::oneshot::channel();
        let first_order = order.clone();
        sender
            .send((
                Box::pin(async move {
                    gated.await.unwrap();
                    first_order.lock().unwrap().push(1);
                    Ok(())
                }),
                first_reply,
            ))
            .unwrap();
        let (second_reply, second) = tokio::sync::oneshot::channel();
        let second_order = order.clone();
        sender
            .send((
                Box::pin(async move {
                    second_order.lock().unwrap().push(2);
                    Err(failure(503))
                }),
                second_reply,
            ))
            .unwrap();
        settle().await;
        assert!(order.lock().unwrap().is_empty());
        assert!(!changes.has_changed().unwrap());
        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        assert!(second.await.unwrap().is_err());
        assert_eq!(*order.lock().unwrap(), [1, 2]);
        assert_eq!(*changes.borrow_and_update(), 1);
    }
}
