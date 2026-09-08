use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Easing {
    Linear,
    EaseOutCubic,
}

impl Easing {
    fn apply(self, progress: f32) -> f32 {
        let progress = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => progress,
            Self::EaseOutCubic => 1.0 - (1.0 - progress).powi(3),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MotionSpec {
    duration: Duration,
    easing: Easing,
}

impl MotionSpec {
    pub(crate) const fn new(duration: Duration, easing: Easing) -> Self {
        Self { duration, easing }
    }

    pub(crate) const fn duration(self) -> Duration {
        self.duration
    }

    pub(crate) fn ease(self, progress: f32) -> f32 {
        self.easing.apply(progress)
    }

    pub(crate) fn sample(self, started_at: Instant, now: Instant) -> MotionSample {
        let progress = if self.duration().is_zero() {
            1.0
        } else {
            now.saturating_duration_since(started_at).as_secs_f32() / self.duration().as_secs_f32()
        };
        let progress = Easing::Linear.apply(progress);
        MotionSample {
            progress,
            eased: self.ease(progress),
            active: progress < 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MotionSample {
    pub(crate) progress: f32,
    eased: f32,
    active: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tween {
    started_at: Instant,
    from: f32,
    to: f32,
    spec: MotionSpec,
}

impl Tween {
    pub(crate) const fn new(started_at: Instant, from: f32, to: f32, spec: MotionSpec) -> Self {
        Self {
            started_at,
            from,
            to,
            spec,
        }
    }

    pub(crate) fn sample(self, now: Instant) -> TweenSample {
        let motion = self.spec.sample(self.started_at, now);
        TweenSample {
            value: self.from + (self.to - self.from) * motion.eased,
            active: motion.active,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TweenSample {
    pub(crate) value: f32,
    pub(crate) active: bool,
}

pub(crate) const FOLD_MOTION: MotionSpec =
    MotionSpec::new(Duration::from_millis(160), Easing::EaseOutCubic);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_sample_clamps_time_and_preserves_endpoints() {
        let started_at = Instant::now();
        let before = FOLD_MOTION.sample(started_at, started_at - Duration::from_millis(1));
        assert_eq!(before.progress, 0.0);
        assert_eq!(before.eased, 0.0);
        assert!(before.active);

        let finished = FOLD_MOTION.sample(started_at, started_at + FOLD_MOTION.duration());
        assert_eq!(finished.progress, 1.0);
        assert_eq!(finished.eased, 1.0);
        assert!(!finished.active);
    }

    #[test]
    fn ease_out_moves_faster_than_linear_progress() {
        assert!(FOLD_MOTION.ease(0.5) > Easing::Linear.apply(0.5));
    }

    #[test]
    fn tween_interpolates_and_reports_completion() {
        let started_at = Instant::now();
        let tween = Tween::new(started_at, 20.0, 100.0, FOLD_MOTION);
        assert_eq!(
            tween.sample(started_at),
            TweenSample {
                value: 20.0,
                active: true,
            }
        );
        let halfway = tween.sample(started_at + FOLD_MOTION.duration() / 2);
        assert!(halfway.value > 60.0 && halfway.value < 100.0);
        assert!(halfway.active);
        assert_eq!(
            tween.sample(started_at + FOLD_MOTION.duration()),
            TweenSample {
                value: 100.0,
                active: false,
            }
        );
    }
}
