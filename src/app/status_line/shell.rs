use super::FLOATING_STATUS_HEIGHT;
use crate::motion::{Easing, MotionSpec, Tween};
use std::time::{Duration, Instant};
pub(crate) const SHELL_MOTION: MotionSpec =
    MotionSpec::new(Duration::from_millis(360), Easing::EaseOutCubic);
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShellShape {
    pub width: f32,
    pub height: f32,
    pub expansion_height: f32,
    pub content_opacity: f32,
}
impl ShellShape {
    pub fn status(width: f32) -> Self {
        Self {
            width,
            height: FLOATING_STATUS_HEIGHT,
            expansion_height: 0.,
            content_opacity: 0.,
        }
    }
    fn interpolate(self, to: Self, p: f32) -> Self {
        Self {
            width: self.width + (to.width - self.width) * p,
            height: self.height + (to.height - self.height) * p,
            expansion_height: self.expansion_height
                + (to.expansion_height - self.expansion_height) * p,
            content_opacity: self.content_opacity + (to.content_opacity - self.content_opacity) * p,
        }
    }
}
#[derive(Default)]
pub(crate) struct ShellMotion {
    target: Option<ShellShape>,
    from: Option<ShellShape>,
    progress: Option<Tween>,
}
impl ShellMotion {
    pub fn update(&mut self, target: ShellShape, available: f32, now: Instant, animate: bool) {
        if self.target == Some(target) && animate {
            return;
        }
        let from = self.sample(available, now).0;
        self.target = Some(target);
        self.from = Some(from);
        self.progress = (animate && from != target).then(|| Tween::new(now, 0., 1., SHELL_MOTION));
    }
    pub fn sample(&self, available: f32, now: Instant) -> (ShellShape, bool) {
        let target = self.target.unwrap_or_else(|| ShellShape::status(available));
        let (mut shape, active) = self.progress.map_or((target, false), |motion| {
            let sample = motion.sample(now);
            (
                self.from
                    .unwrap_or(target)
                    .interpolate(target, sample.value),
                sample.active,
            )
        });
        shape.width = shape.width.clamp(1., available.max(1.));
        (shape, active)
    }
}
