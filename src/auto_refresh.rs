//! Automatic refresh decisions use wall time so time asleep counts towards staleness.

pub const REFRESH_INTERVAL_MS: i64 = 5 * 60 * 1000;
const MAX_RETRY_MS: i64 = 30 * 60 * 1000;

#[derive(Default, Debug)]
pub struct AutoRefreshPolicy {
    pub account_ready: bool,
    last_finished_ms: Option<i64>,
    failures: u32,
}

impl AutoRefreshPolicy {
    /// Seed freshness from disk, without overwriting a sync already completed this session.
    pub fn restore_last_success(&mut self, timestamp_ms: Option<i64>) {
        if self.last_finished_ms.is_none() {
            self.last_finished_ms = timestamp_ms;
        }
    }

    pub fn due(
        &self,
        now_ms: i64,
        active: bool,
        available: bool,
        metered: bool,
        busy: bool,
    ) -> bool {
        if !self.account_ready || !active || !available || metered || busy {
            return false;
        }
        let interval = REFRESH_INTERVAL_MS
            .saturating_mul(1_i64 << self.failures.saturating_sub(1).min(3))
            .min(MAX_RETRY_MS);
        self.last_finished_ms.is_none_or(|last| {
            // A backwards clock adjustment must not prevent refresh indefinitely.
            now_ms < last || now_ms.saturating_sub(last) >= interval
        })
    }

    /// Manual refresh also resets freshness/backoff; it is never gated by this policy.
    pub fn finished(&mut self, now_ms: i64, success: bool) {
        self.last_finished_ms = Some(now_ms);
        self.failures = if success {
            0
        } else {
            self.failures.saturating_add(1)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> AutoRefreshPolicy {
        AutoRefreshPolicy {
            account_ready: true,
            ..Default::default()
        }
    }

    #[test]
    fn startup_uses_persisted_freshness_and_suspend_counts() {
        let mut policy = ready();
        assert!(policy.due(0, true, true, false, false));
        policy.restore_last_success(Some(0));
        assert!(!policy.due(REFRESH_INTERVAL_MS - 1, true, true, false, false));
        assert!(policy.due(REFRESH_INTERVAL_MS, true, true, false, false));
        // No ticks while asleep: one decision after resume is enough.
        assert!(policy.due(8 * 60 * 60 * 1000, true, true, false, false));
        policy.finished(8 * 60 * 60 * 1000, true);
        assert!(!policy.due(8 * 60 * 60 * 1000, true, true, false, false));
    }

    #[test]
    fn ineligible_states_wait_without_consuming_freshness() {
        let mut policy = AutoRefreshPolicy::default();
        assert!(!policy.due(0, true, true, false, false));
        policy.account_ready = true;
        for (active, available, metered, busy) in [
            (false, true, false, false),
            (true, false, false, false),
            (true, true, true, false),
            (true, true, false, true),
        ] {
            assert!(!policy.due(0, active, available, metered, busy));
        }
        assert!(policy.due(0, true, true, false, false));
    }

    #[test]
    fn failures_back_off_and_manual_success_resets_the_interval() {
        let mut policy = ready();
        let mut now = 0;
        for minutes in [5, 10, 20, 30, 30] {
            policy.finished(now, false);
            let delay = minutes * 60 * 1000;
            assert!(!policy.due(now + delay - 1, true, true, false, false));
            assert!(policy.due(now + delay, true, true, false, false));
            now += delay;
        }
        policy.finished(now, true);
        policy.restore_last_success(Some(0));
        assert!(!policy.due(now + REFRESH_INTERVAL_MS - 1, true, true, false, false));
        assert!(policy.due(now + REFRESH_INTERVAL_MS, true, true, false, false));
        assert!(policy.due(now - 1, true, true, false, false));
    }
}
