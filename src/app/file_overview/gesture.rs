use std::time::{Duration, Instant};

use gpui::TouchPhase;

const CLAIM: f32 = 0.030459207; // -ln(0.97)

/// Integrates native incremental magnification without accumulating overshoot.
#[derive(Clone, Debug)]
pub(super) struct Gesture {
    distance: f32,
    sampled_at: Instant,
    last: Instant,
    velocity: f32,
    outward: bool,
    end_scale: f32,
}

impl Gesture {
    pub fn new(outward: bool, now: Instant) -> Self {
        Self {
            distance: 0.0,
            sampled_at: now,
            last: now,
            velocity: 0.0,
            outward,
            end_scale: if outward { 2. } else { 0.5 },
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        let scale = if self.outward {
            scale.max(1.001)
        } else {
            scale.clamp(0.001, 0.999)
        };
        self.velocity *= (self.end_scale - 1.) / (scale - 1.);
        self.end_scale = scale;
        self.distance = self.distance.min(self.extent());
    }

    /// Start a new contact at the currently presented size, with no jump back
    /// to the endpoint from which the original gesture began.
    pub fn seek(&mut self, progress: f32) {
        let scale = 1. + (self.end_scale - 1.) * progress.clamp(0., 1.);
        self.distance = CLAIM + scale.ln().abs();
    }

    fn extent(&self) -> f32 {
        CLAIM + self.end_scale.ln().abs()
    }

    pub fn update(&mut self, delta: f32, now: Instant) {
        if !delta.is_finite() {
            return;
        }
        let increment = (1.0 + delta).clamp(0.5, 1.5).ln() * if self.outward { 1.0 } else { -1.0 };
        let previous = self.progress();
        let minimum = if self.claimed() { CLAIM } else { 0. };
        self.distance = (self.distance + increment).clamp(minimum, self.extent());
        let elapsed = now.saturating_duration_since(self.sampled_at).as_secs_f32();
        self.sampled_at = now;
        let change = self.progress() - previous;
        if change.abs() > 0.0001 && elapsed > 0.0 {
            let next_velocity = change / elapsed.max(0.008);
            // A reversal should be recognized immediately, rather than hidden
            // behind the forward samples in a moving average.
            self.velocity = if self.velocity.signum() != next_velocity.signum() {
                next_velocity
            } else {
                self.velocity * 0.35 + next_velocity * 0.65
            };
            self.last = now;
        }
    }

    pub fn claimed(&self) -> bool {
        self.distance >= CLAIM
    }
    pub fn progress(&self) -> f32 {
        if self.distance >= self.extent() {
            return 1.;
        }
        let scale = ((self.distance - CLAIM).max(0.) * if self.outward { 1. } else { -1. }).exp();
        ((scale - 1.) / (self.end_scale - 1.)).clamp(0., 1.)
    }

    pub fn commits(&self, phase: TouchPhase, now: Instant) -> bool {
        if phase != TouchPhase::Ended {
            return false;
        }
        let velocity = self.velocity(now);
        if velocity < -0.35 {
            return false;
        }
        self.progress() >= 0.45 || self.progress() >= 0.18 && velocity > 1.2
    }

    pub fn velocity(&self, now: Instant) -> f32 {
        if now.saturating_duration_since(self.last) <= Duration::from_millis(100) {
            self.velocity
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_deltas_are_accumulated_and_only_end_can_commit() {
        let now = Instant::now();
        let mut g = Gesture::new(false, now);
        g.set_scale(0.68);
        g.update(-0.01, now + Duration::from_millis(16));
        assert!(!g.claimed());
        g.update(-0.20, now + Duration::from_millis(32));
        assert!(g.progress() > 0.45);
        assert!(g.commits(TouchPhase::Ended, now + Duration::from_millis(200)));
        assert!(!g.commits(TouchPhase::Moved, now + Duration::from_millis(200)));
    }

    #[test]
    fn reversing_after_the_endpoint_has_no_hidden_overshoot() {
        let now = Instant::now();
        let mut g = Gesture::new(false, now);
        for i in 1..10 {
            g.update(-0.3, now + Duration::from_millis(i * 16));
        }
        assert_eq!(g.progress(), 1.0);
        g.update(0.08, now + Duration::from_millis(160));
        assert!(g.progress() < 1.0);
        assert!(!g.commits(TouchPhase::Ended, now + Duration::from_millis(161)));
    }

    #[test]
    fn reversing_from_the_start_does_not_repeat_the_claim_dead_zone() {
        let now = Instant::now();
        let mut gesture = Gesture::new(false, now);
        gesture.set_scale(0.3);
        gesture.update(-0.1, now + Duration::from_millis(16));
        assert!(gesture.claimed());
        for i in 2..10 {
            gesture.update(0.4, now + Duration::from_millis(i * 16));
        }
        assert_eq!(gesture.progress(), 0.);
        gesture.update(-0.01, now + Duration::from_millis(160));
        assert!(gesture.progress() > 0.);
    }

    #[test]
    fn outward_gesture_uses_the_opposite_direction_and_ignores_invalid_values() {
        let now = Instant::now();
        let mut g = Gesture::new(true, now);
        g.set_scale(1.5);
        g.update(f32::NAN, now);
        g.update(f32::INFINITY, now);
        assert_eq!(g.progress(), 0.0);
        g.update(0.3, now + Duration::from_millis(100));
        assert!(g.commits(TouchPhase::Ended, now + Duration::from_millis(300)));
    }

    #[test]
    fn reversal_after_holding_at_the_endpoint_uses_recent_samples() {
        let now = Instant::now();
        let mut g = Gesture::new(false, now);
        g.update(-0.4, now + Duration::from_millis(16));
        for i in 2..80 {
            g.update(-0.02, now + Duration::from_millis(i * 16));
        }
        g.update(0.02, now + Duration::from_millis(1280));
        assert!(g.progress() > 0.9);
        assert!(!g.commits(TouchPhase::Ended, now + Duration::from_millis(1281)));
    }

    #[test]
    fn the_same_native_pinch_changes_page_size_equally_in_every_grid() {
        let now = Instant::now();
        for (outward, scales, claim, delta, expected) in [
            (false, [0.25, 0.5, 0.75], -0.03, -0.1, 0.9),
            (true, [1.5, 2., 4.], 1. / 0.97 - 1., 0.1, 1.1),
        ] {
            for scale in scales {
                let mut gesture = Gesture::new(outward, now);
                gesture.set_scale(scale);
                gesture.update(claim, now + Duration::from_millis(16));
                gesture.update(delta, now + Duration::from_millis(32));
                let page_scale = 1. + (scale - 1.) * gesture.progress();
                assert!((page_scale - expected).abs() < 0.00001);
            }
        }
    }

    #[test]
    fn grabbing_and_reversing_uses_the_current_page_size() {
        let now = Instant::now();
        for (outward, scale, delta) in [(false, 0.3, 0.1), (true, 3., -0.1)] {
            for progress in [0.1, 0.4, 0.8] {
                let mut gesture = Gesture::new(outward, now);
                gesture.set_scale(scale);
                gesture.seek(progress);
                assert!((gesture.progress() - progress).abs() < 0.00001);
                gesture.update(delta, now + Duration::from_millis(16));
                let actual_scale = 1. + (scale - 1.) * gesture.progress();
                let expected = ((1. + (scale - 1.) * progress) * (1. + delta))
                    .clamp(scale.min(1.), scale.max(1.));
                assert!((actual_scale - expected).abs() < 0.00001);
                assert!(!gesture.commits(TouchPhase::Ended, now + Duration::from_millis(17)));
            }
        }
    }
}
