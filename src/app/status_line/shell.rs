use super::FLOATING_STATUS_HEIGHT;
use crate::app::{PaneSide, WorkspaceWindow};
use crate::motion::{Easing, MotionSpec, Tween};
use gpui::{Context, Window};
use std::time::{Duration, Instant};
#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
pub(crate) const SHELL_MOTION: MotionSpec =
    MotionSpec::new(Duration::from_millis(360), Easing::EaseOutCubic);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellKind {
    Prefix,
    Buffers,
    Search,
}

#[derive(Clone, Copy)]
pub(crate) struct ShellRequest {
    pub kind: ShellKind,
    pub pane: PaneSide,
    pub target: ShellShape,
    pub available: f32,
    pub returning: bool,
}

/// The statusline owns the animation; features only describe the content they need.
#[derive(Default)]
pub(crate) struct ShellHost {
    pub motion: ShellMotion,
    owner: Option<ShellKind>,
    pane: Option<PaneSide>,
}

impl ShellHost {
    pub fn owns(&self, kind: ShellKind, pane: PaneSide) -> bool {
        self.owner == Some(kind) && self.pane == Some(pane)
    }

    pub fn update(&mut self, requests: &[ShellRequest], now: Instant, animate: bool) -> bool {
        // Active content replaces a closing presentation. Only the previous owner may
        // continue drawing its outgoing snapshot while the shell returns to the statusline.
        let request = requests
            .iter()
            .find(|r| !r.returning)
            .or_else(|| requests.iter().find(|r| self.owns(r.kind, r.pane)));
        let Some(request) = request else {
            *self = Self::default();
            return false;
        };
        if self.pane.is_some_and(|pane| pane != request.pane) {
            self.motion = ShellMotion::default();
        }
        self.pane = Some(request.pane);
        self.owner = Some(request.kind);
        self.motion
            .update(request.target, request.available, now, animate);
        let active = self.motion.sample(request.available, now).1;
        if request.returning && !active {
            self.owner = None;
        }
        active
    }
}

impl WorkspaceWindow {
    pub(crate) fn status_shell_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let requests = [
            self.prefix_shell_request(window, cx),
            self.buffer_shell_request(window),
            self.search_shell_request(window, cx),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if self
            .status
            .shell
            .update(&requests, Instant::now(), !cx.reduce_motion())
        {
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
        self.finish_prefix_return(self.status.shell.owner == Some(ShellKind::Prefix));
        self.finish_search_return(self.status.shell.owner == Some(ShellKind::Search));
        if self.status.shell.owner != Some(ShellKind::Buffers) {
            self.buffers.returning = false;
        }
    }
}
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
