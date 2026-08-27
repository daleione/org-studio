use super::MinimapResizeSession;

pub(in crate::preview) const FONT_PX: f32 = 2.0;
pub(in crate::preview) const LINE_HEIGHT_PX: f32 = 2.6;
pub(in crate::preview) const EDGE_PADDING_PX: f32 = 4.0;
const MANUAL_MIN_PX: f32 = 48.0;
pub(in crate::preview) const MANUAL_MAX_PX: f32 = 480.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::preview) enum WidthChange {
    Preview(f32),
    Commit(f32),
    Reset,
}
const COMPACT_WINDOW_MAX_FRACTION: f32 = 0.20;
const LARGE_WINDOW_MAX_FRACTION: f32 = 0.30;
const LARGE_WINDOW_TRANSITION_END_PX: f32 = 1920.0;
pub(in crate::preview) const AUTO_COMPACT_MAX_PX: f32 = 96.0;
const AUTO_GROW_START_PX: f32 = 1280.0;
const AUTO_GROW_PER_PX: f32 = 0.11;
const AUTO_MAX_PX: f32 = 220.0;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::preview) enum Density {
    Compact,
    Comfortable,
    Large,
    ExtraLarge,
    Maximum,
}

impl Density {
    pub(in crate::preview) fn for_width(width: f32) -> Self {
        if width < 128.0 {
            Self::Compact
        } else if width < 184.0 {
            Self::Comfortable
        } else if width < 280.0 {
            Self::Large
        } else if width < 400.0 {
            Self::ExtraLarge
        } else {
            Self::Maximum
        }
    }

    pub(in crate::preview) fn font_px(self) -> f32 {
        match self {
            Self::Compact => FONT_PX,
            Self::Comfortable => 2.4,
            Self::Large => 2.8,
            Self::ExtraLarge => 3.2,
            Self::Maximum => 3.6,
        }
    }

    pub(in crate::preview) fn line_height(self) -> f32 {
        match self {
            Self::Compact => LINE_HEIGHT_PX,
            Self::Comfortable => 3.2,
            Self::Large => 3.8,
            Self::ExtraLarge => 4.4,
            Self::Maximum => 5.0,
        }
    }

    pub(in crate::preview) fn edge_padding(self) -> f32 {
        match self {
            Self::Compact => EDGE_PADDING_PX,
            Self::Comfortable => 5.0,
            Self::Large => 6.0,
            Self::ExtraLarge => 7.0,
            Self::Maximum => 8.0,
        }
    }
}

pub(in crate::preview) fn automatic_for_viewport(viewport_width: f32) -> f32 {
    let compact = (viewport_width * 0.15).clamp(24.0, AUTO_COMPACT_MAX_PX);
    if viewport_width <= AUTO_GROW_START_PX {
        compact
    } else {
        (AUTO_COMPACT_MAX_PX + (viewport_width - AUTO_GROW_START_PX) * AUTO_GROW_PER_PX)
            .min(AUTO_MAX_PX)
    }
}

pub(in crate::preview) fn for_viewport(viewport_width: f32, preferred: Option<u16>) -> f32 {
    preferred.map_or_else(
        || automatic_for_viewport(viewport_width),
        |width| manual_for_viewport(viewport_width, f32::from(width)),
    )
}

fn manual_window_max(viewport_width: f32) -> f32 {
    let progress = ((viewport_width - AUTO_GROW_START_PX)
        / (LARGE_WINDOW_TRANSITION_END_PX - AUTO_GROW_START_PX))
        .clamp(0.0, 1.0);
    let fraction = COMPACT_WINDOW_MAX_FRACTION
        + (LARGE_WINDOW_MAX_FRACTION - COMPACT_WINDOW_MAX_FRACTION) * progress;
    (viewport_width * fraction).clamp(24.0, MANUAL_MAX_PX)
}

pub(in crate::preview) fn manual_for_viewport(viewport_width: f32, desired: f32) -> f32 {
    let window_max = manual_window_max(viewport_width);
    desired.clamp(MANUAL_MIN_PX.min(window_max), window_max)
}

pub(in crate::preview) fn from_resize_drag(
    viewport_width: f32,
    session: MinimapResizeSession,
    pointer_x: f32,
) -> f32 {
    manual_for_viewport(
        viewport_width,
        session.start_width + session.start_pointer_x - pointer_x,
    )
}
