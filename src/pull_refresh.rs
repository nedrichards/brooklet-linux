//! Intentional, single-touch pull-to-refresh; offsets are GTK logical pixels.
const DIRECTION_THRESHOLD: f64 = 8.0;
const REFRESH_THRESHOLD: f64 = 72.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullState {
    #[default]
    Idle,
    Pending,
    Pulling,
    Armed,
}

#[derive(Default)]
pub struct PullRefresh {
    state: PullState,
}

impl PullRefresh {
    pub fn begin(&mut self, at_top: bool, enabled: bool) -> PullState {
        self.state = if at_top && enabled {
            PullState::Pending
        } else {
            PullState::Idle
        };
        self.state
    }

    pub fn update(&mut self, x: f64, y: f64) -> PullState {
        if self.state == PullState::Idle {
            return self.state;
        }
        if !x.is_finite() || !y.is_finite() {
            self.cancel();
        } else if self.state == PullState::Pending && y.abs().max(x.abs()) >= DIRECTION_THRESHOLD {
            self.state = if y > 0.0 && y > x.abs() {
                PullState::Pulling
            } else {
                PullState::Idle
            };
        }
        if matches!(self.state, PullState::Pulling | PullState::Armed) {
            self.state = if y >= REFRESH_THRESHOLD && y > x.abs() {
                PullState::Armed
            } else {
                PullState::Pulling
            };
        }
        self.state
    }

    pub fn finish(&mut self, released: bool, enabled: bool) -> bool {
        let refresh = released && enabled && self.state == PullState::Armed;
        self.cancel();
        refresh
    }

    pub fn cancel(&mut self) {
        self.state = PullState::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deliberate_pull_refreshes_once_on_release() {
        let mut pull = PullRefresh::default();
        assert_eq!(pull.begin(true, true), PullState::Pending);
        assert_eq!(pull.update(0.0, 71.0), PullState::Pulling);
        assert_eq!(pull.update(0.0, 72.0), PullState::Armed);
        assert!(pull.finish(true, true));
        assert!(!pull.finish(true, true));
    }

    #[test]
    fn taps_short_pulls_and_other_directions_do_not_refresh() {
        for (x, y) in [
            (0.0, 0.0),
            (0.0, 7.0),
            (0.0, 71.0),
            (0.0, -90.0),
            (90.0, 10.0),
            (90.0, 90.0),
        ] {
            let mut pull = PullRefresh::default();
            pull.begin(true, true);
            pull.update(x, y);
            assert!(!pull.finish(true, true), "{x}, {y}");
        }
    }

    #[test]
    fn scrolling_to_top_during_a_gesture_does_not_arm_it() {
        let mut pull = PullRefresh::default();
        pull.begin(false, true);
        assert_eq!(pull.update(0.0, 200.0), PullState::Idle);
        assert!(!pull.finish(true, true));
    }

    #[test]
    fn reversal_and_sideways_motion_disarm() {
        for (x, y) in [(0.0, 30.0), (0.0, -10.0), (150.0, 90.0)] {
            let mut pull = PullRefresh::default();
            pull.begin(true, true);
            pull.update(0.0, 90.0);
            assert_eq!(pull.update(x, y), PullState::Pulling);
            assert!(!pull.finish(true, true));
        }
    }

    #[test]
    fn cancellation_and_multi_touch_end_do_not_refresh() {
        let mut pull = PullRefresh::default();
        pull.begin(true, true);
        pull.update(0.0, 90.0);
        assert!(!pull.finish(false, true));
        pull.begin(true, true);
        pull.update(0.0, 90.0);
        pull.cancel();
        assert!(!pull.finish(true, true));
    }

    #[test]
    fn busy_sync_at_begin_or_release_prevents_refresh() {
        let mut pull = PullRefresh::default();
        pull.begin(true, false);
        assert_eq!(pull.update(0.0, 90.0), PullState::Idle);
        assert!(!pull.finish(true, true));
        pull.begin(true, true);
        pull.update(0.0, 90.0);
        assert!(!pull.finish(true, false));
        assert!(!pull.finish(true, true));
    }

    #[test]
    fn invalid_offsets_cancel() {
        for (x, y) in [(f64::NAN, 90.0), (0.0, f64::INFINITY)] {
            let mut pull = PullRefresh::default();
            pull.begin(true, true);
            pull.update(x, y);
            assert!(!pull.finish(true, true));
        }
    }
}
