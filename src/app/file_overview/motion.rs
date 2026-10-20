//! Time-based spatial settling with continuous position and release velocity.
use std::time::{Duration, Instant};

use crate::motion::{Easing, MotionSpec, Tween};

// Separate the initial response from the longer, gentle approach to rest.
// These describe our response, not Safari's private animation parameters.
const SETTLE_SLOW_RATE: f32 = 11.0;
const SETTLE_FAST_RATE: f32 = 80.0;
const POSITION_EPSILON: f32 = 0.0005;
const VELOCITY_EPSILON: f32 = 0.01;
const FOLLOW_LAG: Duration = Duration::from_millis(40);

/// Native input changes the target; the display clock samples this response.
/// Exact critical damping preserves position and speed across input events.
/// Its low-frequency delay is 2/rate, independent of input and display rates.
#[derive(Clone, Copy, Debug)]
pub(super) struct Follow {
    started_at: Instant,
    from: f32,
    target: f32,
    velocity: f32,
    rate: f32,
}

impl Follow {
    pub fn new(now: Instant, value: f32) -> Self {
        Self::with_lag(now, value, FOLLOW_LAG)
    }

    pub fn with_lag(now: Instant, value: f32, lag: Duration) -> Self {
        Self {
            started_at: now,
            from: value,
            target: value,
            velocity: 0.,
            rate: 2. / lag.as_secs_f32().max(0.001),
        }
    }

    pub fn resume(now: Instant, sample: Sample) -> Self {
        let mut follow = Self::new(now, sample.value);
        follow.velocity = sample.velocity;
        follow.rate = bounded_rate(follow.rate, sample.value, sample.velocity);
        follow
    }

    pub fn target(&mut self, now: Instant, target: f32) {
        let sample = self.sample(now);
        self.started_at = now;
        self.from = sample.value;
        self.velocity = sample.velocity;
        self.target = target.clamp(0., 1.);
    }

    pub fn sample(self, now: Instant) -> Sample {
        damped_sample(
            self.started_at,
            self.from,
            self.target,
            self.velocity,
            self.rate,
            now,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Motion {
    Settle {
        started_at: Instant,
        from: f32,
        to: f32,
        velocity: f32,
        rate: f32,
        immediate: bool,
    },
    Fade {
        tween: Tween,
        immediate: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Sample {
    pub value: f32,
    pub velocity: f32,
    pub active: bool,
}

impl Motion {
    pub fn settle(now: Instant, from: f32, to: f32, velocity: f32, reduced: bool) -> Self {
        let velocity = if velocity.is_finite() { velocity } else { 0. };
        // A fast filtered release can be moving faster than the default
        // settling response. Increase damping just enough to retain its speed
        // inside both physical endpoints, rather than abruptly cutting speed.
        let rate = bounded_rate(SETTLE_FAST_RATE, from, velocity);
        let velocity = velocity.clamp(-rate * from, rate * (1. - from));
        Self::Settle {
            started_at: now,
            from,
            to,
            velocity,
            rate,
            immediate: reduced,
        }
    }

    pub fn fade(now: Instant, reduced: bool) -> Self {
        Self::Fade {
            tween: Tween::new(
                now,
                0.,
                1.,
                MotionSpec::new(
                    Duration::from_millis(if reduced { 0 } else { 120 }),
                    Easing::EaseOutCubic,
                ),
            ),
            immediate: reduced,
        }
    }

    pub fn sample(self, now: Instant) -> Sample {
        match self {
            Self::Fade { tween, .. } => {
                let sample = tween.sample(now);
                Sample {
                    value: sample.value,
                    velocity: 0.,
                    active: sample.active,
                }
            }
            Self::Settle {
                started_at,
                from,
                to,
                velocity,
                rate,
                immediate,
            } => {
                if immediate {
                    return Sample {
                        value: to,
                        velocity: 0.,
                        active: false,
                    };
                }
                settled_sample(started_at, from, to, velocity, rate, now)
            }
        }
    }

    pub fn finish_with_reduced_motion(self, now: Instant) -> Self {
        match self {
            Self::Settle { to, .. } => Self::settle(now, to, to, 0., true),
            Self::Fade {
                immediate: false, ..
            } => Self::fade(now, true),
            _ => self,
        }
    }
}

fn bounded_rate(rate: f32, from: f32, velocity: f32) -> f32 {
    if velocity > 0. && from < 1. {
        rate.max(velocity / (1. - from))
    } else if velocity < 0. && from > 0. {
        rate.max(-velocity / from)
    } else {
        rate
    }
}

fn damped_sample(
    started_at: Instant,
    from: f32,
    to: f32,
    velocity: f32,
    rate: f32,
    now: Instant,
) -> Sample {
    let t = now.saturating_duration_since(started_at).as_secs_f32();
    let distance = from - to;
    let coefficient = velocity + rate * distance;
    let decay = (-rate * t).exp();
    let offset = (distance + coefficient * t) * decay;
    let speed = (velocity - rate * coefficient * t) * decay;
    let active = offset.abs() > POSITION_EPSILON || speed.abs() > VELOCITY_EPSILON;
    Sample {
        value: if active { to + offset } else { to },
        velocity: if active { speed } else { 0. },
        active,
    }
}

/// Exact overdamped response. The fast mode accepts release/reversal velocity;
/// the slow mode supplies a longer deceleration without a delayed start.
fn settled_sample(
    started_at: Instant,
    from: f32,
    to: f32,
    velocity: f32,
    fast_rate: f32,
    now: Instant,
) -> Sample {
    let t = now.saturating_duration_since(started_at).as_secs_f32();
    let slow_rate = SETTLE_SLOW_RATE;
    let slow_decay = (-slow_rate * t).exp();
    let fast_decay = (-fast_rate * t).exp();
    let distance = from - to;
    let coefficient = (velocity + slow_rate * distance) / (fast_rate - slow_rate);
    let offset = distance * slow_decay + coefficient * (slow_decay - fast_decay);
    let speed = -slow_rate * distance * slow_decay
        + coefficient * (fast_rate * fast_decay - slow_rate * slow_decay);
    let active = offset.abs() > POSITION_EPSILON || speed.abs() > VELOCITY_EPSILON;
    let (value, speed) = if t == 0. {
        (from, velocity)
    } else {
        (to + offset, speed)
    };
    Sample {
        value: if active { value } else { to },
        velocity: if active { speed } else { 0. },
        active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn following_filters_jitter_without_waiting_for_release() {
        let now = Instant::now();
        let mut follow = Follow::new(now, 0.5);
        let mut displayed = Vec::new();
        for index in 0..60 {
            let time = now + Duration::from_millis(index * 8);
            follow.target(time, if index % 2 == 0 { 0.6 } else { 0.4 });
            displayed.push(follow.sample(time + Duration::from_millis(8)).value);
        }
        let spread = displayed.iter().copied().fold(f32::MIN, f32::max)
            - displayed.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread < 0.04, "displayed spread {spread}, input spread 0.2");
        let hold = now + Duration::from_millis(480);
        follow.target(hold, 0.75);
        let moving = follow.sample(hold + Duration::from_millis(40));
        assert!(moving.value > 0.5 && moving.value < 0.75);
        assert!(moving.velocity > 0.);
        let stopped = follow.sample(hold + Duration::from_millis(400));
        assert_eq!(stopped.value, 0.75);
        assert!(!stopped.active);
    }

    #[test]
    fn following_and_release_preserve_velocity_even_after_a_fast_reversal() {
        let now = Instant::now();
        let mut follow = Follow::with_lag(now, 0., Duration::from_millis(10));
        follow.target(now, 1.);
        let reverse_at = now + Duration::from_millis(10);
        let before = follow.sample(reverse_at);
        follow.target(reverse_at, 0.);
        let after = follow.sample(reverse_at);
        assert_eq!(after.value, before.value);
        assert_eq!(after.velocity, before.velocity);
        assert!(after.velocity > SETTLE_FAST_RATE * (1. - after.value));
        for target in [0., 1.] {
            let release = Motion::settle(reverse_at, after.value, target, after.velocity, false);
            assert_eq!(release.sample(reverse_at).value, after.value);
            assert!((release.sample(reverse_at).velocity - after.velocity).abs() < 0.00001);
            for ms in 0..500 {
                assert!(
                    (0. ..=1.)
                        .contains(&release.sample(reverse_at + Duration::from_millis(ms)).value)
                );
            }
        }
        for ms in 0..500 {
            assert!(
                (0. ..=1.).contains(&follow.sample(reverse_at + Duration::from_millis(ms)).value)
            );
        }
    }

    #[test]
    fn following_depends_on_input_timestamps_not_display_frequency() {
        let now = Instant::now();
        let mut slow = Follow::new(now, 0.);
        let mut fast = slow;
        for (ms, target) in [(0, 0.1), (23, 0.4), (87, 0.35), (111, 0.8), (172, 0.6)] {
            let time = now + Duration::from_millis(ms);
            for offset in 0..16 {
                let _ = fast.sample(time + Duration::from_millis(offset));
            }
            slow.target(time, target);
            fast.target(time, target);
            let a = slow.sample(time);
            let b = fast.sample(time);
            assert_eq!(a.value, b.value);
            assert_eq!(a.velocity, b.velocity);
        }
    }

    #[test]
    fn grabbing_a_moving_page_preserves_position_and_velocity() {
        let now = Instant::now();
        let moving = Motion::settle(now, 0., 1., 0., false);
        let contact = now + Duration::from_millis(75);
        let before = moving.sample(contact);
        let grabbed = Follow::resume(contact, before);
        assert_eq!(grabbed.sample(contact).value, before.value);
        assert_eq!(grabbed.sample(contact).velocity, before.velocity);
        let held = grabbed.sample(contact + Duration::from_millis(500));
        assert_eq!(held.value, before.value);
        assert!(!held.active);
    }

    #[test]
    fn automatic_motion_accelerates_then_decelerates_and_finishes_exactly() {
        let now = Instant::now();
        let m = Motion::settle(now, 0., 1., 0., false);
        assert_eq!(m.sample(now).value, 0.);
        assert_eq!(m.sample(now).velocity, 0.);
        let speeds =
            [10, 40, 150, 300].map(|ms| m.sample(now + Duration::from_millis(ms)).velocity);
        assert!(speeds[1] > speeds[0] && speeds[2] < speeds[1] && speeds[3] < speeds[2]);
        let end = m.sample(now + Duration::from_secs(1));
        assert_eq!(end.value, 1.);
        assert!(!end.active);
    }

    #[test]
    fn retargeting_preserves_position_and_velocity() {
        let now = Instant::now();
        let release = Motion::settle(now, 0.4, 1., 2., false);
        assert_eq!(release.sample(now).velocity, 2.);
        let turn = now + Duration::from_millis(40);
        let sample = release.sample(turn);
        let reverse = Motion::settle(turn, sample.value, 0., sample.velocity, false);
        assert_eq!(reverse.sample(turn).value, sample.value);
        assert!((reverse.sample(turn).velocity - sample.velocity).abs() < 0.00001);
        assert_eq!(reverse.sample(turn + Duration::from_secs(1)).value, 0.);
    }

    #[test]
    fn fast_releases_do_not_overshoot_or_oscillate() {
        let now = Instant::now();
        for from in [0.0, 0.01, 0.4, 0.99, 1.0] {
            for to in [0.0, 1.0] {
                for velocity in [-100., 0., 100.] {
                    let m = Motion::settle(now, from, to, velocity, false);
                    for ms in 0..700 {
                        let value = m.sample(now + Duration::from_millis(ms)).value;
                        assert!(
                            (-0.00001..=1.00001).contains(&value),
                            "{from} → {to}, v={velocity}, t={ms}: {value}"
                        );
                    }
                }
            }
        }
    }
}
