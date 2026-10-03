use crate::model::SyncStatus;

/// The combined error remains available to older consumers. New views keep
/// refresh and delivery failures independent, including while work is running.
pub fn headline(status: &SyncStatus) -> String {
    if status.refresh_error.is_some() || status.delivery_error.is_some() || status.error.is_some() {
        return if status.running {
            "Syncing · Previous failure needs attention"
        } else {
            "Sync needs attention"
        }
        .into();
    }
    if status.running {
        return "Syncing…".into();
    }
    if status.queued_mutations > 0 {
        return format!("{} changes waiting to send", status.queued_mutations);
    }
    match status.last_successful_sync_at_ms {
        Some(time) => jiff::Timestamp::from_millisecond(time)
            .map(|time| format!("Last sync {time}"))
            .unwrap_or_else(|_| "Last sync time unavailable".into()),
        None => "Not synced yet".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_does_not_hide_unresolved_errors() {
        let status = SyncStatus {
            running: true,
            refresh_error: Some("offline".into()),
            ..Default::default()
        };
        assert!(headline(&status).contains("Previous failure"));
        assert!(
            headline(&SyncStatus {
                running: false,
                ..status
            })
            .contains("attention")
        );
    }

    #[test]
    fn successful_pull_does_not_hide_pending_delivery_failure() {
        let status = SyncStatus {
            last_successful_sync_at_ms: Some(0),
            delivery_error: Some("key rejected".into()),
            queued_mutations: 1,
            ..Default::default()
        };
        assert_eq!(headline(&status), "Sync needs attention");
    }

    #[test]
    fn pending_work_is_not_reported_as_synced() {
        let status = SyncStatus {
            last_successful_sync_at_ms: Some(0),
            queued_mutations: 2,
            ..Default::default()
        };
        assert_eq!(headline(&status), "2 changes waiting to send");
        assert!(headline(&SyncStatus::default()).contains("Not synced"));
        assert!(
            headline(&SyncStatus {
                queued_mutations: 0,
                ..status
            })
            .starts_with("Last sync")
        );
    }
}
