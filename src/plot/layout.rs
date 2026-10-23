use super::{PlotEntry, PlotKind, PlotLayout, PlotPlan, PlotPreparation};
use crate::{
    org_syntax::attributes::{ImageAttributeSpec, ImageLength},
    preview::ImageSizing,
};

const MIN_CANVAS_WIDTH: f32 = 180.0;
const MAX_CANVAS_WIDTH: f32 = 2048.0;
const MIN_CANVAS_HEIGHT: f32 = 160.0;
const MAX_CANVAS_HEIGHT: f32 = 1600.0;

pub(super) fn display_scale(font_size: f32) -> f32 {
    (font_size / 16.0).clamp(0.25, 4.0)
}

pub(super) fn canvas_size(width: f32, height: f32) -> (u32, u32) {
    (
        width.round().clamp(MIN_CANVAS_WIDTH, MAX_CANVAS_WIDTH) as u32,
        height.round().clamp(MIN_CANVAS_HEIGHT, MAX_CANVAS_HEIGHT) as u32,
    )
}

pub(super) fn display_size(
    source: (u32, u32),
    attributes: &ImageAttributeSpec,
    sizing: &ImageSizing,
) -> (f32, f32) {
    let scale = display_scale(sizing.font_size);
    let length = |value| match value {
        ImageLength::Px(value) => value,
        ImageLength::Percent(value) => sizing.max_width * value / 100.0,
        ImageLength::Em(value) => sizing.font_size * value,
    };
    let (width, height) = if let Some(scale) = attributes.scale {
        (source.0 as f32 * scale, source.1 as f32 * scale)
    } else {
        (
            attributes
                .width
                .map(length)
                .unwrap_or(source.0 as f32 * scale),
            attributes
                .height
                .map(length)
                .unwrap_or(source.1 as f32 * scale),
        )
    };
    (
        width
            .max(1.0)
            .min(sizing.max_width.max(1.0))
            .min(MAX_CANVAS_WIDTH * scale),
        height.clamp(MIN_CANVAS_HEIGHT * scale, MAX_CANVAS_HEIGHT * scale),
    )
}

impl PlotEntry {
    pub(crate) fn image_size(&self, sizing: &ImageSizing) -> (f32, f32) {
        self.image_size_with_attributes(sizing, &self.source.image_attributes)
    }

    pub(crate) fn image_size_with_attributes(
        &self,
        sizing: &ImageSizing,
        attributes: &ImageAttributeSpec,
    ) -> (f32, f32) {
        let attributes = self.display.lock().unwrap().attributes(attributes);
        let (width, height) = display_size(self.dimensions(), &attributes, sizing);
        if let PlotPreparation::Ready(plot) = &self.prepared {
            let scale = display_scale(sizing.font_size);
            let minimum = minimum_height(&plot.plan, width / scale).min(MAX_CANVAS_HEIGHT) * scale;
            (width, height.max(minimum))
        } else {
            (width, height)
        }
    }

    pub(crate) fn render_request(
        &self,
        sizing: &ImageSizing,
        style: super::PlotStyle,
    ) -> Option<super::PlotRenderRequest> {
        let PlotPreparation::Ready(plot) = &self.prepared else {
            return None;
        };
        Some(super::PlotRenderRequest::for_display(
            plot.clone(),
            style,
            self.image_size(sizing),
            sizing.font_size,
        ))
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        match &self.prepared {
            PlotPreparation::Ready(plot) => plot.natural_size,
            _ => super::parse::parse_spec(&self.source.options)
                .map(|spec| spec.layout)
                .unwrap_or_default()
                .dimensions(),
        }
    }
}

/// Defaults depend on the data's shape, never its fluctuating numeric values.
pub(super) fn natural_size(plan: &PlotPlan) -> (u32, u32) {
    let (width, height) = match plan.layout {
        PlotLayout::Radar => (360, 320),
        PlotLayout::Grid => (
            (plan.rows as u32 * 48 + 120).clamp(480, 960),
            (plan.series.len() as u32 * 36 + 120).clamp(250, 720),
        ),
        PlotLayout::Cartesian => {
            let categories = matches!(plan.kind, PlotKind::Histograms | PlotKind::BoxPlot);
            let width = if plan.time_axis.is_some() {
                720
            } else if categories {
                (plan.rows as u32 * 54 + 120).clamp(480, 960)
            } else {
                (plan.rows as u32 * 16 + 120).clamp(480, 800)
            };
            let longest = plan
                .x_ticks
                .iter()
                .map(|tick| tick.label.chars().count())
                .max()
                .unwrap_or(0);
            let label_extra = if categories {
                longest.saturating_sub(8).min(24) as u32 * 2
            } else {
                0
            };
            (
                width,
                (if plan.time_axis.is_some() { 320 } else { 250 }) + label_extra,
            )
        }
    };
    canvas_size(
        width as f32,
        (height as f32).max(minimum_height(plan, width as f32)),
    )
}

pub(super) fn minimum_height(plan: &PlotPlan, width: f32) -> f32 {
    let available = (width - 56.0).max(80.0);
    let text_width = |text: &str| {
        text.chars()
            .map(|ch| if ch.is_ascii() { 6.0 } else { 12.0 })
            .sum::<f32>()
    };
    let title = plan
        .title
        .as_ref()
        .filter(|title| !title.is_empty())
        .map_or(0.0, |title| {
            (text_width(title) * 1.75 / available).ceil() * 25.0 + 16.0
        });
    let mut legend = 0.0;
    if plan.series.len() > 1 && plan.layout != PlotLayout::Grid && plan.kind != PlotKind::BoxPlot {
        let mut used = 0.0;
        legend = 32.0;
        for series in &plan.series {
            let item = text_width(&series.label) + 32.0;
            if used > 0.0 && used + item > available {
                legend += 22.0;
                used = 0.0;
            }
            used += item;
        }
    }
    160.0 + title + legend + if plan.time_axis.is_some() { 20.0 } else { 0.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlotResizeAxis {
    Width,
    Height,
    Both,
}

impl PlotResizeAxis {
    pub(crate) fn cursor(self) -> gpui::CursorStyle {
        match self {
            Self::Width => gpui::CursorStyle::ResizeLeftRight,
            Self::Height => gpui::CursorStyle::ResizeUpDown,
            Self::Both => gpui::CursorStyle::ResizeUpLeftDownRight,
        }
    }

    pub(crate) fn resize(self, start: (f32, f32), delta: (f32, f32), max_width: f32) -> (f32, f32) {
        let width = if self == Self::Height {
            start.0
        } else {
            let max = max_width.max(1.0);
            (start.0 + delta.0).clamp(240.0_f32.min(start.0).min(max), max)
        };
        let height = if self == Self::Width {
            start.1
        } else {
            // Authored sizes outside normal drag limits must not jump on first movement.
            (start.1 + delta.1).clamp(180.0_f32.min(start.1), 1200.0_f32.max(start.1))
        };
        (width, height)
    }
}
