use tracing::{Event, Level, Metadata, Subscriber, field::Visit};
use tracing_subscriber::{
    Layer,
    layer::{Context, Filter, SubscriberExt},
    util::SubscriberInitExt,
};

struct BrookletLogFilter;

impl<S: Subscriber> Filter<S> for BrookletLogFilter {
    fn enabled(&self, metadata: &Metadata<'_>, _: &Context<'_, S>) -> bool {
        metadata.level() <= &Level::INFO
    }

    fn event_enabled(&self, event: &Event<'_>, _: &Context<'_, S>) -> bool {
        if event.metadata().target() != "zbus::proxy" || *event.metadata().level() != Level::WARN {
            return true;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        !is_transient_portal_request_warning(&visitor.message)
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

fn is_transient_portal_request_warning(message: &str) -> bool {
    message.contains("Failed to populate properties cache via GetAll")
        && message.contains("Object does not exist at path")
        && message.contains("/org/freedesktop/portal/desktop/request/")
}

pub fn init() {
    // ashpd creates a short-lived Request proxy for the secret portal. Its
    // response can retire the object before zbus's optional property-cache
    // GetAll runs. Only that exact transient warning is filtered; secret-store
    // failures and all other zbus warnings/errors remain visible.
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_filter(BrookletLogFilter),
        )
        .try_init();
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tracing::{Event, Subscriber};
    use tracing_subscriber::{Layer, layer::Context, prelude::*};

    use super::{BrookletLogFilter, is_transient_portal_request_warning};

    struct EventCount(Arc<AtomicUsize>);

    impl<S: Subscriber> Layer<S> for EventCount {
        fn on_event(&self, _: &Event<'_>, _: Context<'_, S>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn filters_only_the_expired_portal_request_cache_warning() {
        let transient = "Failed to populate properties cache via GetAll: \
            org.freedesktop.DBus.Error.UnknownMethod: Object does not exist at path \
            /org/freedesktop/portal/desktop/request/1_3373/ashpd_h92V5vaCJJ";
        assert!(is_transient_portal_request_warning(transient));
        assert!(!is_transient_portal_request_warning(
            "Failed to populate properties cache via GetAll: permission denied"
        ));
        assert!(!is_transient_portal_request_warning(
            "Object does not exist at path /org/freedesktop/portal/desktop/session/1"
        ));
    }

    #[test]
    fn log_filter_preserves_other_zbus_warnings_and_errors() {
        let count = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry()
            .with(EventCount(count.clone()).with_filter(BrookletLogFilter));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                target: "zbus::proxy",
                "Failed to populate properties cache via GetAll: org.freedesktop.DBus.Error.UnknownMethod: Object does not exist at path /org/freedesktop/portal/desktop/request/1_3373/ashpd_h92V5vaCJJ"
            );
            tracing::warn!(target: "zbus::proxy", "Permission denied by secret portal");
            tracing::error!(target: "zbus::proxy", "Transport disconnected");
        });
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
