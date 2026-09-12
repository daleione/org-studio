#[cfg(test)]
use crate::app::status_line::shell::SHELL_MOTION;
#[cfg(test)]
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) struct BarLayout {
    pub narrow: bool,
    pub controls_width: f32,
    pub column_gap: f32,
    pub mode_item_width: f32,
    pub replacement_height: f32,
}
impl BarLayout {
    pub fn new(available: f32, language: crate::i18n::Language) -> Self {
        let narrow = available < 420.;
        Self {
            narrow,
            // Reserve the result slot independently of its current label. Switching
            // from scope to progress or a match count must not resize the input.
            controls_width: match (language, narrow) {
                (_, true) => 160.,
                (crate::i18n::Language::English, false) => 212.,
                (_, false) => 196.,
            },
            column_gap: if !narrow && language == crate::i18n::Language::English {
                8.
            } else {
                4.
            },
            mode_item_width: match (language, narrow) {
                (crate::i18n::Language::English, true) => 46.,
                (crate::i18n::Language::English, false) => 52.,
                (_, true) => 36.,
                (_, false) => 44.,
            },
            replacement_height: if narrow { 76. } else { 41. },
        }
    }
    pub fn mode_width(&self) -> f32 {
        self.mode_item_width * 2. + 6.
    }
}
pub(super) fn content_width(
    query: f32,
    replacement: Option<f32>,
    available: f32,
    layout: BarLayout,
) -> f32 {
    let reserved = layout.mode_width() + layout.controls_width + 58. + 2. * layout.column_gap;
    (query.max(replacement.unwrap_or(0.)) + reserved)
        .clamp(480., 960.)
        .min(available.max(1.))
}
#[cfg(test)]
use crate::app::status_line::shell::ShellMotion;
pub(super) use crate::app::status_line::shell::ShellShape;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_shell_retargets_all_dimensions_without_jumping() {
        let now = Instant::now();
        let search = ShellShape {
            width: 480.,
            height: 109.,
            expansion_height: 41.,
            content_opacity: 1.,
        };
        let mut motion = ShellMotion::default();
        motion.update(search, 1200., now, true);
        assert_eq!(motion.sample(1200., now).0, ShellShape::status(1200.));
        let opened = now + SHELL_MOTION.duration();
        assert_eq!(motion.sample(1200., opened), (search, false));
        motion.update(ShellShape::status(1200.), 1200., opened, true);
        let midway = opened + Duration::from_millis(150);
        let shape = motion.sample(1200., midway).0;
        let find = ShellShape {
            height: 42.,
            expansion_height: 0.,
            ..search
        };
        motion.update(find, 1200., midway, true);
        assert_eq!(motion.sample(1200., midway).0, shape);
        assert_eq!(
            motion.sample(1200., midway + SHELL_MOTION.duration()).0,
            find
        );
        motion.update(ShellShape::status(320.), 320., midway, false);
        assert_eq!(
            motion.sample(320., midway),
            (ShellShape::status(320.), false)
        );
    }
    #[test]
    fn search_width_follows_content_and_reserves_controls() {
        let layout = BarLayout::new(1200., crate::i18n::Language::Chinese);
        assert_eq!(content_width(26., None, 1200., layout), 480.);
        assert_eq!(content_width(400., None, 1200., layout), 756.);
        assert_eq!(content_width(26., Some(500.), 1200., layout), 856.);
        assert_eq!(content_width(4000., None, 1200., layout), 960.);
        assert_eq!(content_width(4000., None, 320., layout), 320.);
    }
}
