use std::f32::consts::PI;
use std::sync::Arc;

use gpui::{
    Entity, Hsla, MouseButton, ParentElement, PathBuilder, SharedString, Styled, TextRun, Window,
    canvas, div, font, point, prelude::*, px, rgb,
};
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

use super::{ContentRoute, WorkspaceWindow};
use crate::{
    i18n::Language,
    navigation::PaneId,
    preview::{DocumentFormat, preview_style},
    settings::StatusLineSettings,
    theme::current_theme,
};

mod host;
mod model;
mod popover;
#[cfg(test)]
use host::reading_progress;
use model::*;
pub(crate) use model::{CachedStatusLayout, StatusLineLayout, StatusLineSnapshot, StatusPopover};
pub(crate) use popover::render_status_popover;

pub(crate) struct StatusLineHost {
    settings: StatusLineSettings,
    popover: Option<StatusPopover>,
    layout_cache: std::cell::RefCell<std::collections::HashMap<PaneId, CachedStatusLayout>>,
}

impl StatusLineHost {
    pub(crate) fn new(settings: StatusLineSettings) -> Self {
        Self {
            settings,
            popover: None,
            layout_cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }

    pub(crate) fn settings(&self) -> StatusLineSettings {
        self.settings
    }

    pub(crate) fn dismiss_popover(&mut self) -> bool {
        self.popover.take().is_some()
    }

    pub(crate) fn clear_layout_cache(&self) {
        self.layout_cache.borrow_mut().clear();
    }

    pub(crate) fn popover_for(&self, pane: PaneId) -> Option<StatusPopover> {
        self.popover
            .as_ref()
            .filter(|popover| popover.pane == pane)
            .cloned()
    }
}

pub(crate) const STATUS_LINE_HEIGHT: f32 = 30.0;
const OUTLINE_FULL_RESERVE: f32 = 260.0;
const OUTLINE_COMPACT_WIDTH: f32 = 150.0;
const POSITION_FULL_CHROME: f32 = 40.0;
const POSITION_COMPACT_CHROME: f32 = 24.0;
const PROGRESS_FULL_CHROME: f32 = 42.0;
const PROGRESS_COMPACT_CHROME: f32 = 24.0;
const READING_STYLE_CHROME: f32 = 24.0;

// Status colors deliberately stay independent from document syntax colors. A status should
// communicate state consistently even when the active theme uses red for its first heading.
const STATUS_CLEAN: u32 = 0x2f7049;
const STATUS_CLEAN_HOVER: u32 = 0x285f3e;
const STATUS_CLEAN_FOREGROUND: u32 = 0xf4f6fa;
const STATUS_DIRTY: u32 = 0xd5a24d;
const STATUS_DIRTY_HOVER: u32 = 0xc49342;
const STATUS_DIRTY_FOREGROUND: u32 = 0x3f2d16;
const STATUS_WORKING_TEXT: u32 = 0x6f718f;
const STATUS_SUCCESS_TEXT: u32 = 0x557b61;
const STATUS_ERROR_TEXT: u32 = 0xa14f5d;
const STATUS_PROGRESS: u32 = 0x5e7f84;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModeColors {
    background: u32,
    hover: u32,
    foreground: u32,
}

fn mode_colors(dirty: bool) -> ModeColors {
    if dirty {
        ModeColors {
            background: STATUS_DIRTY,
            hover: STATUS_DIRTY_HOVER,
            foreground: STATUS_DIRTY_FOREGROUND,
        }
    } else {
        ModeColors {
            background: STATUS_CLEAN,
            hover: STATUS_CLEAN_HOVER,
            foreground: STATUS_CLEAN_FOREGROUND,
        }
    }
}

fn status_tone_color(tone: StatusTone) -> u32 {
    match tone {
        StatusTone::Working => STATUS_WORKING_TEXT,
        StatusTone::Success => STATUS_SUCCESS_TEXT,
        StatusTone::Error => STATUS_ERROR_TEXT,
    }
}

impl StatusLineSnapshot {
    fn layout_key(&self, width: f32, settings: StatusLineSettings) -> StatusLayoutKey {
        StatusLayoutKey {
            width_bits: width.to_bits(),
            settings,
            host: self.host,
            surface: self.surface,
            reading_style: self.reading_style,
            language: self.language,
            outline: self.outline.is_some(),
            position_reserve: self
                .position
                .map(|position| position_reserve_text(position, false, self.language)),
            progress: self.progress.is_some(),
            statistics: self.statistics.clone(),
            format: self.format,
        }
    }

    #[cfg(test)]
    pub(crate) fn layout(&self, width: f32, settings: StatusLineSettings) -> StatusLineLayout {
        self.layout_with_measure(width, settings, &text_width)
    }

    pub(crate) fn layout_in_window(
        &self,
        width: f32,
        settings: StatusLineSettings,
        window: &Window,
    ) -> StatusLineLayout {
        self.layout_with_measure(width, settings, &|text| measured_text_width(text, window))
    }

    fn layout_with_measure(
        &self,
        width: f32,
        settings: StatusLineSettings,
        measure: &impl Fn(&str) -> f32,
    ) -> StatusLineLayout {
        let mut layout = StatusLineLayout {
            mode: Variant::Full,
            reading_style: if self.reading_style.is_some() {
                Variant::Full
            } else {
                Variant::Hidden
            },
            outline: if settings.outline && self.outline.is_some() {
                Variant::Full
            } else {
                Variant::Hidden
            },
            position: if settings.position && self.position.is_some() {
                Variant::Full
            } else {
                Variant::Hidden
            },
            progress: if settings.progress && self.progress.is_some() {
                Variant::Full
            } else {
                Variant::Hidden
            },
            statistics: if settings.statistics && self.statistics.is_some() {
                Variant::Full
            } else {
                Variant::Hidden
            },
            format: if settings.format && self.format.is_some() {
                Variant::Full
            } else {
                Variant::Hidden
            },
            outline_max_width: 0.0,
            overflow: Arc::from([]),
        };
        let available = (width - 18.0).max(100.0);
        if layout.width(self, measure) > available {
            // Preserve the short character count until space is genuinely scarce. Long contextual
            // information compacts first; position deliberately survives longest because it is also
            // the stable contract needed by the future editor host.
            for step in [
                Degrade::HideFormat,
                Degrade::CompactOutline,
                Degrade::CompactProgress,
                Degrade::CompactPosition,
                Degrade::CompactMode,
                Degrade::CompactReadingStyle,
                Degrade::HideOutline,
                Degrade::HideStatistics,
                Degrade::HideProgress,
                Degrade::HidePosition,
            ] {
                layout.apply(step);
                if layout.width(self, measure) <= available {
                    break;
                }
            }
        }
        layout.overflow = CONFIGURABLE_SEGMENTS
            .into_iter()
            .filter(|segment| {
                segment.enabled(settings)
                    && self.has_segment(*segment)
                    && layout.variant(*segment) == Variant::Hidden
            })
            .collect::<Vec<_>>()
            .into();
        layout.outline_max_width = layout.resolved_outline_width(self, available, measure);
        layout
    }

    fn mode_label(&self) -> &'static str {
        match self.host {
            StatusHost::Dired => "FILES",
            StatusHost::Agenda => "AGENDA",
            StatusHost::Reading | StatusHost::Editor => match self.surface {
                crate::app::PaneSurface::Editor => "EDITOR",
                crate::app::PaneSurface::Reading => "READING",
            },
        }
    }
}

#[derive(Clone, Copy)]
enum Degrade {
    HideStatistics,
    HideFormat,
    CompactProgress,
    CompactPosition,
    CompactOutline,
    CompactMode,
    CompactReadingStyle,
    HideProgress,
    HideOutline,
    HidePosition,
}

impl StatusLineLayout {
    fn apply(&mut self, step: Degrade) {
        match step {
            Degrade::HideStatistics if self.statistics != Variant::Hidden => {
                self.statistics = Variant::Hidden
            }
            Degrade::HideFormat if self.format != Variant::Hidden => self.format = Variant::Hidden,
            Degrade::CompactProgress if self.progress == Variant::Full => {
                self.progress = Variant::Compact
            }
            Degrade::CompactPosition if self.position == Variant::Full => {
                self.position = Variant::Compact
            }
            Degrade::CompactOutline if self.outline == Variant::Full => {
                self.outline = Variant::Compact
            }
            Degrade::CompactMode if self.mode == Variant::Full => self.mode = Variant::Compact,
            Degrade::CompactReadingStyle if self.reading_style == Variant::Full => {
                self.reading_style = Variant::Compact
            }
            Degrade::HideProgress if self.progress != Variant::Hidden => {
                self.progress = Variant::Hidden
            }
            Degrade::HideOutline if self.outline != Variant::Hidden => {
                self.outline = Variant::Hidden
            }
            Degrade::HidePosition if self.position != Variant::Hidden => {
                self.position = Variant::Hidden
            }
            _ => {}
        }
    }

    fn width(&self, snapshot: &StatusLineSnapshot, measure: &impl Fn(&str) -> f32) -> f32 {
        let mode = match self.mode {
            Variant::Full => measure(snapshot.mode_label()) + 28.0,
            Variant::Compact => 30.0,
            Variant::Hidden => 0.0,
        };
        let reading_style = match (self.reading_style, snapshot.reading_style) {
            (Variant::Full, Some(id)) => {
                measure(preview_style(id).name(snapshot.language)) + READING_STYLE_CHROME
            }
            (Variant::Compact, Some(_)) => {
                measure(match snapshot.language {
                    Language::Chinese => "样式",
                    Language::English => "Style",
                }) + READING_STYLE_CHROME
            }
            _ => 0.0,
        };
        let outline = match (self.outline, snapshot.outline.as_deref()) {
            (Variant::Full, Some(_)) => OUTLINE_FULL_RESERVE,
            (Variant::Compact, Some(_)) => OUTLINE_COMPACT_WIDTH,
            _ => 0.0,
        };
        let position = match (self.position, snapshot.position) {
            (Variant::Full, Some(value)) => {
                position_slot_width(value, false, snapshot.language, measure)
            }
            (Variant::Compact, Some(value)) => {
                position_slot_width(value, true, snapshot.language, measure)
            }
            _ => 0.0,
        };
        let progress = match (self.progress, snapshot.progress) {
            (Variant::Full, Some(_)) => progress_slot_width(false, measure),
            (Variant::Compact, Some(_)) => progress_slot_width(true, measure),
            _ => 0.0,
        };
        let statistics = match (self.statistics, snapshot.statistics.as_deref()) {
            (Variant::Full, Some(value)) => measure(value) + 18.0,
            _ => 0.0,
        };
        let format = match (self.format, snapshot.format) {
            (Variant::Full, Some(value)) => measure(format_label(value)) + 18.0,
            _ => 0.0,
        };
        // More is mandatory. The flexible center keeps a small hit target even when empty.
        mode + reading_style + outline + position + progress + statistics + format + 44.0 + 16.0
    }

    fn resolved_outline_width(
        &self,
        snapshot: &StatusLineSnapshot,
        available: f32,
        measure: &impl Fn(&str) -> f32,
    ) -> f32 {
        let reserved = match self.outline {
            Variant::Full => OUTLINE_FULL_RESERVE,
            Variant::Compact => OUTLINE_COMPACT_WIDTH,
            Variant::Hidden => return 0.0,
        };
        let remaining = (available - (self.width(snapshot, measure) - reserved)).max(0.0);
        if self.outline == Variant::Compact {
            remaining.min(OUTLINE_COMPACT_WIDTH)
        } else {
            remaining
        }
    }
}

#[cfg(test)]
fn text_width(text: &str) -> f32 {
    UnicodeWidthStr::width(text) as f32 * 6.15
}

fn measured_text_width(text: &str, window: &Window) -> f32 {
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font("Menlo"),
        color: Hsla::default(),
        ..Default::default()
    };
    f32::from(
        window
            .text_system()
            .shape_line(text, px(10.0), &[run], None)
            .width,
    )
}

pub(crate) fn reading_style_popover_left(
    snapshot: &StatusLineSnapshot,
    layout: &StatusLineLayout,
    window: &Window,
) -> f32 {
    match layout.mode {
        // Mode margin + padding + dot + gap + measured label, followed by the
        // small gap before the Reading style selector.
        Variant::Full => measured_text_width(snapshot.mode_label(), window) + 39.0,
        Variant::Compact => 29.0,
        Variant::Hidden => 8.0,
    }
}

fn leaf(path: &str) -> &str {
    path.rsplit(" / ").next().unwrap_or(path)
}

fn format_label(format: DocumentFormat) -> &'static str {
    match format {
        DocumentFormat::Org => "Org",
        DocumentFormat::Markdown => "MD",
    }
}

fn position_text(position: StatusPosition, compact: bool, language: Language) -> String {
    match position {
        StatusPosition::ReadingSource { line, .. } => {
            if compact {
                line.to_string()
            } else {
                match language {
                    Language::Chinese => format!("源 {line}"),
                    Language::English => format!("Src {line}"),
                }
            }
        }
        StatusPosition::EditorCaret { line, column } => {
            if compact {
                line.to_string()
            } else {
                format!("{line}:{column}")
            }
        }
        StatusPosition::DiredSelection { selected, total } => {
            if compact {
                selected.to_string()
            } else {
                format!("{selected} / {total}")
            }
        }
        StatusPosition::AgendaSelection { selected, total } => {
            if compact {
                selected.to_string()
            } else {
                format!("{selected} / {total}")
            }
        }
    }
}

fn position_reserve_text(position: StatusPosition, compact: bool, language: Language) -> String {
    let digits = |value: u64| value.max(1).ilog10() as usize + 1;
    match position {
        StatusPosition::ReadingSource { total_lines, .. } => {
            let number = "9".repeat(digits(total_lines));
            if compact {
                number
            } else {
                match language {
                    Language::Chinese => format!("源 {number}"),
                    Language::English => format!("Src {number}"),
                }
            }
        }
        StatusPosition::EditorCaret { line, column } => {
            if compact {
                "9".repeat(digits(line))
            } else {
                format!(
                    "{}:{}",
                    "9".repeat(digits(line)),
                    "9".repeat(digits(column))
                )
            }
        }
        StatusPosition::DiredSelection { total, .. } => {
            let number = "9".repeat(digits(total as u64));
            if compact {
                number
            } else {
                format!("{number} / {number}")
            }
        }
        StatusPosition::AgendaSelection { total, .. } => {
            let number = "9".repeat(digits(total as u64));
            if compact {
                number
            } else {
                format!("{number} / {number}")
            }
        }
    }
}

fn position_slot_width(
    position: StatusPosition,
    compact: bool,
    language: Language,
    measure: &impl Fn(&str) -> f32,
) -> f32 {
    measure(&position_reserve_text(position, compact, language))
        + if compact {
            POSITION_COMPACT_CHROME
        } else {
            POSITION_FULL_CHROME
        }
}

fn progress_slot_width(compact: bool, measure: &impl Fn(&str) -> f32) -> f32 {
    measure("100%")
        + if compact {
            PROGRESS_COMPACT_CHROME
        } else {
            PROGRESS_FULL_CHROME
        }
}

pub(crate) fn render_status_line(
    snapshot: &StatusLineSnapshot,
    layout: StatusLineLayout,
    entity: Entity<WorkspaceWindow>,
    window: &Window,
) -> gpui::AnyElement {
    let theme = current_theme();
    let pane_id = snapshot.pane.0;
    let mode_colors = mode_colors(snapshot.dirty);
    let mode = status_button(entity.clone(), pane_id, StatusSegment::Mode)
        .mx(px(4.0))
        .my(px(4.0))
        .h(px(22.0))
        .px(px(if layout.mode == Variant::Compact {
            7.0
        } else {
            9.0
        }))
        .gap(px(6.0))
        .rounded(px(6.0))
        .bg(rgb(mode_colors.background))
        .text_color(rgb(mode_colors.foreground))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .hover(move |style| {
            style
                .bg(rgb(mode_colors.hover))
                .text_color(rgb(mode_colors.foreground))
        })
        .child(
            div()
                .size(px(5.0))
                .rounded_full()
                .bg(rgb(mode_colors.foreground)),
        )
        .when(layout.mode == Variant::Full, |button| {
            button.child(snapshot.mode_label())
        });

    let left = div()
        .h_full()
        .flex()
        .items_center()
        .flex_none()
        .child(mode)
        .when_some(snapshot.reading_style, |left, style_id| {
            left.child(
                status_button(entity.clone(), pane_id, StatusSegment::ReadingStyle)
                    .ml(px(2.0))
                    .mr(px(2.0))
                    .my(px(4.0))
                    .h(px(22.0))
                    .px(px(6.0))
                    .gap(px(4.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.background))
                    .text_color(rgb(theme.foreground))
                    .child(if layout.reading_style == Variant::Compact {
                        match snapshot.language {
                            Language::Chinese => "样式",
                            Language::English => "Style",
                        }
                    } else {
                        preview_style(style_id).name(snapshot.language)
                    })
                    .child(
                        div()
                            .text_size(px(8.0))
                            .text_color(rgb(theme.foreground_dim))
                            .child("▾"),
                    ),
            )
        })
        .when(layout.outline != Variant::Hidden, |left| {
            match snapshot.outline.as_deref() {
                Some(outline) => left.child(
                    status_button(entity.clone(), pane_id, StatusSegment::Outline)
                        .max_w(px(layout.outline_max_width))
                        .child("◎")
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(if layout.outline == Variant::Compact {
                                    leaf(outline).to_owned()
                                } else {
                                    outline.to_owned()
                                }),
                        ),
                ),
                None => left,
            }
        });

    let center_color = snapshot
        .transient
        .as_ref()
        .map_or(theme.foreground_dim, |message| {
            status_tone_color(message.tone)
        });
    let center = div()
        .min_w(px(12.0))
        .flex_1()
        .h_full()
        .px(px(5.0))
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_color(rgb(center_color))
        .when_some(snapshot.transient.as_ref(), |center, message| {
            center.child(message.text.to_string())
        });

    let mut right = div().h_full().flex().items_center().flex_none();
    if layout.statistics != Variant::Hidden
        && let Some(statistics) = snapshot.statistics.as_deref()
    {
        right = right.child(
            status_button(entity.clone(), pane_id, StatusSegment::Statistics)
                .child(statistics.to_owned()),
        );
    }
    if layout.position != Variant::Hidden
        && let Some(position) = snapshot.position
    {
        let compact = layout.position == Variant::Compact;
        let slot_width = position_slot_width(position, compact, snapshot.language, &|text| {
            measured_text_width(text, window)
        });
        right = right.child(
            status_button(entity.clone(), pane_id, StatusSegment::Position)
                .w(px(slot_width))
                .overflow_hidden()
                .when(layout.position == Variant::Full, |button| button.child("⌖"))
                .child(position_text(position, compact, snapshot.language)),
        );
    }
    if layout.progress != Variant::Hidden
        && let Some(progress) = snapshot.progress
    {
        let compact = layout.progress == Variant::Compact;
        let slot_width = progress_slot_width(compact, &|text| measured_text_width(text, window));
        right = right.child(
            status_button(entity.clone(), pane_id, StatusSegment::Progress)
                .w(px(slot_width))
                .overflow_hidden()
                .when(layout.progress == Variant::Full, |button| {
                    button.child(progress_ring(progress))
                })
                .child(format!("{progress}%")),
        );
    }
    if layout.format != Variant::Hidden
        && let Some(format) = snapshot.format
    {
        right = right.child(
            status_button(entity.clone(), pane_id, StatusSegment::Format)
                .child(format_label(format)),
        );
    }
    let overflow = layout.overflow.clone();
    let more_entity = entity.clone();
    right = right.child(
        status_button(entity.clone(), pane_id, StatusSegment::More)
            .child("•••")
            .when(!layout.overflow.is_empty(), |button| {
                button.child(
                    div()
                        .min_w(px(14.0))
                        .h(px(14.0))
                        .px(px(3.0))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgb(theme.code_boundary_background))
                        .text_size(px(8.0))
                        .child(layout.overflow.len().to_string()),
                )
            })
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let overflow = overflow.clone();
                more_entity.update(cx, |this, cx| {
                    this.activate_status_segment(
                        PaneId(pane_id),
                        StatusSegment::More,
                        Some(overflow),
                        cx,
                    )
                });
            }),
    );

    let customize_entity = entity;
    div()
        .id(format!("pane-{pane_id}-status-line"))
        .h(px(STATUS_LINE_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .overflow_hidden()
        .border_t_1()
        .border_color(rgb(theme.border))
        .bg(rgb(theme.background_alt))
        .text_color(rgb(theme.foreground_dim))
        .text_size(px(10.0))
        .on_mouse_down(MouseButton::Right, move |_, _, cx| {
            cx.stop_propagation();
            customize_entity.update(cx, |this, cx| {
                this.status.popover = Some(StatusPopover {
                    pane: PaneId(pane_id),
                    content: StatusPopoverContent::Customize,
                });
                cx.notify();
            });
        })
        .child(left)
        .child(center)
        .child(right)
        .into_any_element()
}

fn status_button(
    entity: Entity<WorkspaceWindow>,
    pane_id: u64,
    segment: StatusSegment,
) -> gpui::Stateful<gpui::Div> {
    let theme = current_theme();
    let button = div()
        .id(format!("status-{pane_id}-segment-{}", segment as usize))
        .debug_selector(|| format!("status-{pane_id}-segment-{}", segment as usize))
        .h_full()
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .cursor_pointer()
        .border_l_1()
        .border_color(rgb(theme.border));
    let button = if segment == StatusSegment::Mode {
        button
    } else {
        button.hover(|style| {
            style
                .bg(rgb(theme.code_boundary_background))
                .text_color(rgb(theme.foreground))
        })
    };
    if segment == StatusSegment::More {
        return button;
    }
    button.on_click(move |_, _, cx| {
        cx.stop_propagation();
        entity.update(cx, |this, cx| {
            this.activate_status_segment(PaneId(pane_id), segment, None, cx)
        });
    })
}

fn progress_ring(progress: u8) -> impl gpui::IntoElement {
    let theme = current_theme();
    let background = rgb(theme.border);
    let foreground = rgb(STATUS_PROGRESS);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let center_x = bounds.origin.x + bounds.size.width / 2.0;
            let center_y = bounds.origin.y + bounds.size.height / 2.0;
            let radius = px(5.5);
            let stroke = px(1.5);

            let mut track = PathBuilder::stroke(stroke);
            track.move_to(point(center_x + radius, center_y));
            track.arc_to(
                point(radius, radius),
                px(0.0),
                false,
                true,
                point(center_x - radius, center_y),
            );
            track.arc_to(
                point(radius, radius),
                px(0.0),
                false,
                true,
                point(center_x + radius, center_y),
            );
            track.close();
            if let Ok(path) = track.build() {
                window.paint_path(path, background);
            }

            let fraction = f32::from(progress) / 100.0;
            if fraction <= 0.0 {
                return;
            }
            let mut arc = PathBuilder::stroke(stroke);
            if fraction >= 0.999 {
                arc.move_to(point(center_x + radius, center_y));
                arc.arc_to(
                    point(radius, radius),
                    px(0.0),
                    false,
                    true,
                    point(center_x - radius, center_y),
                );
                arc.arc_to(
                    point(radius, radius),
                    px(0.0),
                    false,
                    true,
                    point(center_x + radius, center_y),
                );
                arc.close();
            } else {
                let start = point(center_x, center_y - radius);
                let angle = -PI / 2.0 + fraction * 2.0 * PI;
                let end = point(
                    center_x + radius * angle.cos(),
                    center_y + radius * angle.sin(),
                );
                arc.move_to(start);
                arc.arc_to(point(radius, radius), px(0.0), fraction > 0.5, true, end);
            }
            if let Ok(path) = arc.build() {
                window.paint_path(path, foreground);
            }
        },
    )
    .size(px(15.0))
}

impl WorkspaceWindow {
    fn agenda_status_snapshot(&self) -> Option<StatusLineSnapshot> {
        let (selected, total, progress, errors) = self.agenda.status_counts();
        Some(StatusLineSnapshot {
            pane: model::AGENDA_PANE_ID,
            language: self.language,
            host: StatusHost::Agenda,
            surface: crate::app::PaneSurface::Editor,
            dirty: false,
            reading_style: None,
            outline: Some(self.agenda.mode_name().into()),
            position: Some(StatusPosition::AgendaSelection { selected, total }),
            progress: (progress < 100).then_some(progress),
            statistics: Some(
                if errors == 0 {
                    format!("{total} tasks")
                } else {
                    format!("{total} tasks · {errors} errors")
                }
                .into(),
            ),
            document_statistics: None,
            format: None,
            transient: None,
        })
    }

    pub(crate) fn status_snapshot(&self, cx: &gpui::App) -> Option<StatusLineSnapshot> {
        match self.content_route {
            ContentRoute::Document => {
                self.document_status_snapshot(self.document_workspace.active_pane, cx)
            }
            ContentRoute::FileManager => self.dired_status_snapshot(DIRED_PANE_ID),
            ContentRoute::Agenda | ContentRoute::AgendaText => self.agenda_status_snapshot(),
        }
    }

    pub(crate) fn status_layout(
        &self,
        snapshot: &StatusLineSnapshot,
        width: f32,
        window: &Window,
    ) -> StatusLineLayout {
        let key = snapshot.layout_key(width, self.status.settings);
        if let Some(cached) = self.status.layout_cache.borrow().get(&snapshot.pane)
            && cached.key == key
        {
            return cached.layout.clone();
        }
        let layout = snapshot.layout_in_window(width, self.status.settings, window);
        self.status.layout_cache.borrow_mut().insert(
            snapshot.pane,
            CachedStatusLayout {
                key,
                layout: layout.clone(),
            },
        );
        layout
    }

    fn activate_status_segment(
        &mut self,
        pane: PaneId,
        segment: StatusSegment,
        overflow: Option<Arc<[StatusSegment]>>,
        cx: &mut gpui::Context<Self>,
    ) {
        match segment {
            StatusSegment::Mode if self.content_route == ContentRoute::Document => {
                self.status.popover = None;
                self.toggle_pane_surface(pane_side_for_status(pane), cx);
            }
            StatusSegment::ReadingStyle => {
                self.status.popover = Some(StatusPopover {
                    pane,
                    content: StatusPopoverContent::ReadingStyle,
                });
            }
            StatusSegment::More => {
                self.status.popover = Some(StatusPopover {
                    pane,
                    content: more_popover_content(overflow),
                });
            }
            _ => {
                self.status.popover = Some(StatusPopover {
                    pane,
                    content: StatusPopoverContent::Info(segment),
                });
            }
        }
        cx.notify();
    }

    fn toggle_status_segment(&mut self, segment: StatusSegment, cx: &mut gpui::Context<Self>) {
        match segment {
            StatusSegment::Outline => self.status.settings.outline = !self.status.settings.outline,
            StatusSegment::Position => {
                self.status.settings.position = !self.status.settings.position
            }
            StatusSegment::Progress => {
                self.status.settings.progress = !self.status.settings.progress
            }
            StatusSegment::Statistics => {
                self.status.settings.statistics = !self.status.settings.statistics
            }
            StatusSegment::Format => self.status.settings.format = !self.status.settings.format,
            StatusSegment::Mode | StatusSegment::ReadingStyle | StatusSegment::More => return,
        }
        self.save_preview_settings();
        cx.notify();
    }
}

fn pane_side_for_status(pane: PaneId) -> crate::app::PaneSide {
    if pane == RIGHT_DOCUMENT_PANE_ID {
        crate::app::PaneSide::Right
    } else {
        crate::app::PaneSide::Left
    }
}

fn more_popover_content(overflow: Option<Arc<[StatusSegment]>>) -> StatusPopoverContent {
    overflow.filter(|segments| !segments.is_empty()).map_or(
        StatusPopoverContent::Customize,
        StatusPopoverContent::Overflow,
    )
}

fn format_character_count(characters: u64, language: Language) -> String {
    let value = if characters < 1_000 {
        characters.to_string()
    } else if characters < 1_000_000 {
        format!("{:.1}k", characters as f64 / 1_000.0)
    } else {
        format!("{:.1}m", characters as f64 / 1_000_000.0)
    };
    match language {
        Language::Chinese => format!("{value} 字"),
        Language::English => format!("{value} chars"),
    }
}

fn format_byte_count(bytes: u64) -> String {
    if bytes < 1_000 {
        format!("{bytes} B")
    } else if bytes < 1_000_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    }
}

fn format_integer(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

fn render_document_statistics(
    statistics: DocumentStatistics,
    language: Language,
) -> gpui::AnyElement {
    let theme = current_theme();
    let labels = match language {
        Language::Chinese => ["字符", "行数", "字节"],
        Language::English => ["Characters", "Lines", "Bytes"],
    };
    let values = [
        format_integer(statistics.characters),
        format_integer(statistics.lines),
        format!(
            "{} · {}",
            format_integer(statistics.bytes),
            format_byte_count(statistics.bytes)
        ),
    ];
    let mut content = div().px(px(12.0)).pb(px(9.0));
    for (index, (label, value)) in labels.into_iter().zip(values).enumerate() {
        content = content.child(
            div()
                .h(px(28.0))
                .flex()
                .items_center()
                .when(index > 0, |row| {
                    row.border_t_1().border_color(rgb(theme.border))
                })
                .child(
                    div()
                        .flex_1()
                        .text_color(rgb(theme.foreground_dim))
                        .child(label),
                )
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(theme.foreground))
                        .child(value),
                ),
        );
    }
    content.into_any_element()
}

fn segment_title(segment: StatusSegment, language: Language) -> &'static str {
    match (language, segment) {
        (Language::Chinese, StatusSegment::Mode) => "工作模式",
        (Language::Chinese, StatusSegment::ReadingStyle) => "阅读主题",
        (Language::Chinese, StatusSegment::Outline) => "大纲位置",
        (Language::Chinese, StatusSegment::Position) => "位置",
        (Language::Chinese, StatusSegment::Progress) => "文档进度",
        (Language::Chinese, StatusSegment::Statistics) => "文档统计",
        (Language::Chinese, StatusSegment::Format) => "文件格式",
        (Language::Chinese, StatusSegment::More) => "更多",
        (Language::English, StatusSegment::Mode) => "Working mode",
        (Language::English, StatusSegment::ReadingStyle) => "Reading theme",
        (Language::English, StatusSegment::Outline) => "Outline",
        (Language::English, StatusSegment::Position) => "Position",
        (Language::English, StatusSegment::Progress) => "Document progress",
        (Language::English, StatusSegment::Statistics) => "Document statistics",
        (Language::English, StatusSegment::Format) => "File format",
        (Language::English, StatusSegment::More) => "More",
    }
}

fn info_text(
    segment: StatusSegment,
    snapshot: Option<&StatusLineSnapshot>,
    language: Language,
) -> String {
    if segment == StatusSegment::Mode
        && snapshot.is_some_and(|snapshot| snapshot.host == StatusHost::Editor)
    {
        return match language {
            Language::Chinese => "当前为编辑界面。位置和进度来自编辑器的实时光标与 viewport；点击可打开或关闭右侧预览。".to_owned(),
            Language::English => {
                "Editing is active. Position and progress use the editor's live caret and viewport; click to toggle the right preview.".to_owned()
            }
        };
    }
    if segment == StatusSegment::Mode
        && snapshot.is_some_and(|snapshot| snapshot.host == StatusHost::Reading)
    {
        return match language {
            Language::Chinese => {
                "当前为 Preview。位置和进度来自预览内容映射与 viewport。".to_owned()
            }
            Language::English => {
                "Preview is active. Position and progress use the preview mapping and viewport."
                    .to_owned()
            }
        };
    }
    if segment == StatusSegment::Position
        && let Some(StatusPosition::ReadingSource { line, .. }) =
            snapshot.and_then(|snapshot| snapshot.position)
    {
        return match language {
            Language::Chinese => {
                format!("当前可视内容映射到源文件第 {line} 行；Preview 不伪造光标列。")
            }
            Language::English => format!(
                "The visible preview maps to source line {line}; Preview does not invent a caret column."
            ),
        };
    }
    let value = snapshot.and_then(|snapshot| match segment {
        StatusSegment::Outline => snapshot.outline.as_deref().map(str::to_owned),
        StatusSegment::Position => snapshot
            .position
            .map(|value| position_text(value, false, language)),
        StatusSegment::Progress => snapshot.progress.map(|value| format!("{value}%")),
        StatusSegment::Statistics => snapshot.statistics.as_deref().map(str::to_owned),
        StatusSegment::Format => snapshot.format.map(format_label).map(str::to_owned),
        StatusSegment::Mode => Some(snapshot.mode_label().to_owned()),
        StatusSegment::ReadingStyle => snapshot
            .reading_style
            .map(|id| preview_style(id).name(language).to_owned()),
        StatusSegment::More => None,
    });
    match (language, value) {
        (Language::Chinese, Some(value)) => format!("当前值：{value}"),
        (Language::English, Some(value)) => format!("Current value: {value}"),
        (Language::Chinese, None) => "当前 Pane 暂无可显示的信息。".to_owned(),
        (Language::English, None) => "No value is available for this pane.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> StatusLineSnapshot {
        StatusLineSnapshot {
            pane: PaneId(7),
            language: Language::Chinese,
            host: StatusHost::Reading,
            surface: crate::app::PaneSurface::Reading,
            dirty: false,
            reading_style: Some(crate::preview::PreviewStyleId::Base),
            outline: Some("性能优化 / Minimap".into()),
            position: Some(StatusPosition::ReadingSource {
                line: 259,
                total_lines: 9_842,
            }),
            progress: Some(78),
            statistics: Some("8.8k 字".into()),
            document_statistics: Some(DocumentStatistics {
                characters: 8_842,
                lines: 259,
                bytes: 10_240,
            }),
            format: Some(DocumentFormat::Org),
            transient: None,
        }
    }

    #[test]
    fn status_palette_reserves_red_for_actual_errors() {
        let clean = mode_colors(false);
        let dirty = mode_colors(true);

        assert_eq!(clean.background, STATUS_CLEAN);
        assert_eq!(dirty.background, STATUS_DIRTY);
        assert_ne!(clean.background, dirty.background);
        assert_ne!(clean.background, current_theme().heading[0]);
        assert_ne!(dirty.background, current_theme().heading[0]);
        assert_ne!(STATUS_PROGRESS, current_theme().heading[0]);
        assert_eq!(status_tone_color(StatusTone::Error), STATUS_ERROR_TEXT);
    }

    #[test]
    fn responsive_layout_degrades_without_hiding_mandatory_segments() {
        let settings = StatusLineSettings::default();
        let wide = snapshot().layout(900.0, settings);
        assert_eq!(wide.mode, Variant::Full);
        assert_eq!(wide.statistics, Variant::Full);
        assert!(wide.overflow.is_empty());

        let split = snapshot().layout(430.0, settings);
        assert_eq!(split.statistics, Variant::Full);
        assert_ne!(split.mode, Variant::Hidden);
        assert_ne!(split.position, Variant::Hidden);

        let narrow = snapshot().layout(210.0, settings);
        assert_ne!(narrow.mode, Variant::Hidden);
        assert_eq!(narrow.reading_style, Variant::Compact);
        assert_ne!(narrow.position, Variant::Hidden);
        assert!(narrow.overflow.len() >= 2);
        assert!(!narrow.overflow.contains(&StatusSegment::Mode));
        assert!(!narrow.overflow.contains(&StatusSegment::More));
    }

    #[test]
    fn reading_style_popover_is_dismissed_before_escape_changes_surface() {
        let mut host = StatusLineHost::new(StatusLineSettings::default());
        host.popover = Some(StatusPopover {
            pane: PaneId(7),
            content: StatusPopoverContent::ReadingStyle,
        });

        assert!(host.dismiss_popover());
        assert!(host.popover.is_none());
        assert!(!host.dismiss_popover());
    }

    #[test]
    fn overflow_preserves_hidden_actions_and_targets_the_originating_pane() {
        assert_ne!(DOCUMENT_PANE_ID, DIRED_PANE_ID);
        let hidden: Arc<[StatusSegment]> =
            Arc::from([StatusSegment::Outline, StatusSegment::Statistics]);
        let popover = StatusPopover {
            pane: PaneId(7),
            content: more_popover_content(Some(hidden.clone())),
        };
        assert_eq!(popover.pane, PaneId(7));
        assert_eq!(popover.content, StatusPopoverContent::Overflow(hidden));
        assert_eq!(
            more_popover_content(Some(Arc::from([]))),
            StatusPopoverContent::Customize
        );
    }

    #[gpui::test]
    fn clicking_the_rendered_more_segment_opens_its_pane_overflow(cx: &mut gpui::TestAppContext) {
        use gpui::{AppContext, Context, IntoElement, Modifiers, Render, point};

        let app = cx.update(|cx| cx.new(|_| WorkspaceWindow::with_split_layout(true)));
        let snapshot = snapshot();
        let pane = snapshot.pane;
        struct StatusHarness {
            app: Entity<WorkspaceWindow>,
            snapshot: StatusLineSnapshot,
        }
        impl Render for StatusHarness {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let layout =
                    self.snapshot
                        .layout_in_window(210.0, StatusLineSettings::default(), window);
                render_status_line(&self.snapshot, layout, self.app.clone(), window)
            }
        }
        let (_harness, cx) = cx.add_window_view(|_, _| StatusHarness {
            app: app.clone(),
            snapshot: snapshot.clone(),
        });
        let bounds = cx
            .debug_bounds("status-7-segment-6")
            .expect("More segment should be painted");
        let center = point(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        );
        cx.simulate_mouse_move(center, None, Modifiers::default());
        cx.simulate_click(center, Modifiers::default());

        let popover = cx.update(|_, cx| app.read(cx).status.popover.clone().unwrap());
        assert_eq!(popover.pane, pane);
        assert!(matches!(
            popover.content,
            StatusPopoverContent::Overflow(ref segments) if !segments.is_empty()
        ));
    }

    #[gpui::test]
    fn clicking_a_status_surface_control_only_changes_its_own_pane(cx: &mut gpui::TestAppContext) {
        use gpui::{AppContext, Context, IntoElement, Modifiers, Render, point};

        let app = cx.update(|cx| cx.new(|_| WorkspaceWindow::with_split_layout(false)));
        let mut snapshot = snapshot();
        snapshot.host = StatusHost::Editor;
        struct StatusHarness {
            app: Entity<WorkspaceWindow>,
            snapshot: StatusLineSnapshot,
        }
        impl Render for StatusHarness {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let layout =
                    self.snapshot
                        .layout_in_window(900.0, StatusLineSettings::default(), window);
                render_status_line(&self.snapshot, layout, self.app.clone(), window)
            }
        }
        let (_harness, cx) = cx.add_window_view(|_, _| StatusHarness {
            app: app.clone(),
            snapshot,
        });
        let bounds = cx
            .debug_bounds("status-7-segment-0")
            .expect("left status control should be painted");
        let center = point(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        );
        cx.simulate_mouse_move(center, None, Modifiers::default());
        cx.simulate_click(center, Modifiers::default());

        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(
                app.document_workspace.surface(crate::app::PaneSide::Left),
                crate::app::PaneSurface::Reading
            );
            assert!(app.status.popover.is_none());
        });
    }

    #[test]
    fn preview_and_editor_positions_have_distinct_semantics() {
        assert_eq!(
            position_text(
                StatusPosition::ReadingSource {
                    line: 259,
                    total_lines: 9_842,
                },
                false,
                Language::Chinese,
            ),
            "源 259"
        );
        assert_eq!(
            position_text(
                StatusPosition::EditorCaret {
                    line: 259,
                    column: 4,
                },
                false,
                Language::English,
            ),
            "259:4"
        );
    }

    #[gpui::test]
    fn editing_status_reads_the_editor_instead_of_the_auxiliary_preview(
        cx: &mut gpui::TestAppContext,
    ) {
        let path = std::env::temp_dir().join(format!(
            "org-studio-source-status-{}.org",
            std::process::id()
        ));
        std::fs::write(&path, "first\nsecond\n").unwrap();
        let loaded = crate::preview::load_document(path.clone()).unwrap();
        let _ = std::fs::remove_file(path);
        let window = cx.open_window(gpui::size(px(900.0), px(700.0)), |_, _| {
            WorkspaceWindow::with_split_layout(true)
        });
        let editor = window
            .update(cx, |app, _, cx| {
                app.language = Language::Chinese;
                app.generation = 1;
                assert!(app.apply_load_result(1, Ok(loaded), cx));
                app.state
                    .ready()
                    .unwrap()
                    .editors
                    .left
                    .clone()
                    .expect("left editor exists")
            })
            .unwrap();
        cx.run_until_parked();
        let root = window.entity(cx).unwrap();
        let callback_count = cx.update(|cx| {
            cx.with_window(root.entity_id(), |window, cx| {
                window.simulate_next_frame(cx)
            })
            .unwrap()
        });
        assert!(callback_count > 0);
        cx.run_until_parked();
        editor.update(cx, |editor, cx| {
            editor.set_selection(
                crate::document::Selection::caret(crate::document::ByteOffset(8)),
                cx,
            );
        });
        cx.run_until_parked();

        let snapshot = window
            .update(cx, |app, _, cx| app.status_snapshot(cx).unwrap())
            .unwrap();
        assert_eq!(snapshot.host, StatusHost::Editor);
        assert_eq!(
            snapshot.position,
            Some(StatusPosition::EditorCaret { line: 2, column: 3 })
        );
        assert_eq!(snapshot.progress, Some(100));
        assert_eq!(snapshot.statistics.as_deref(), Some("13 字"));
        assert!(snapshot.outline.is_none());
    }

    #[test]
    fn reading_progress_tracks_the_source_bottom_line() {
        assert_eq!(reading_progress(0, 100, false), 0);
        assert_eq!(reading_progress(7, 100, false), 7);
        assert_eq!(reading_progress(55, 100, false), 55);
        assert_eq!(reading_progress(999, 1_000, false), 99);
        assert_eq!(reading_progress(100, 100, false), 99);
        assert_eq!(reading_progress(120, 100, false), 99);
        assert_eq!(reading_progress(3, 100, true), 100);
        assert_eq!(reading_progress(0, 0, false), 0);
    }

    #[test]
    fn character_statistics_are_localized_and_grouped() {
        assert_eq!(format_character_count(842, Language::Chinese), "842 字");
        assert_eq!(format_character_count(8_842, Language::Chinese), "8.8k 字");
        assert_eq!(
            format_character_count(8_842, Language::English),
            "8.8k chars"
        );
        assert_eq!(format_integer(8_842_019), "8,842,019");
    }

    #[test]
    fn position_reserves_the_total_line_digit_count() {
        let first = StatusPosition::ReadingSource {
            line: 1,
            total_lines: 999,
        };
        let last = StatusPosition::ReadingSource {
            line: 999,
            total_lines: 999,
        };
        assert_eq!(
            position_reserve_text(first, false, Language::Chinese),
            "源 999"
        );
        assert_eq!(
            position_reserve_text(first, false, Language::Chinese),
            position_reserve_text(last, false, Language::Chinese)
        );
        assert_eq!(position_reserve_text(first, true, Language::Chinese), "999");
        assert_eq!(
            position_slot_width(first, false, Language::Chinese, &text_width),
            position_slot_width(last, false, Language::Chinese, &text_width)
        );
    }

    #[test]
    fn progress_width_does_not_change_at_digit_boundaries() {
        let mut first = snapshot();
        first.progress = Some(1);
        let first_layout = first.layout(900.0, StatusLineSettings::default());
        let mut last = first.clone();
        last.progress = Some(100);
        let last_layout = last.layout(900.0, StatusLineSettings::default());
        assert_eq!(
            first_layout.width(&first, &text_width),
            last_layout.width(&last, &text_width)
        );
    }

    #[test]
    fn full_outline_uses_available_space_instead_of_the_compact_cap() {
        let mut snapshot = snapshot();
        snapshot.outline = Some(
            "一段超过旧版二百六十像素限制但在宽 Pane 中能够完整显示的大纲路径 / 当前标题".into(),
        );
        let layout = snapshot.layout(1_400.0, StatusLineSettings::default());
        assert_eq!(layout.outline, Variant::Full);
        assert!(layout.outline_max_width > OUTLINE_FULL_RESERVE);
    }

    #[test]
    fn scrolling_changes_values_without_changing_status_geometry() {
        let mut first = snapshot();
        first.outline = Some("短标题".into());
        first.position = Some(StatusPosition::ReadingSource {
            line: 1,
            total_lines: 9_842,
        });
        first.progress = Some(7);

        let mut later = first.clone();
        later.outline =
            Some("一个在滚动后出现的明显更长标题 / 但它不应改变状态栏的响应式分档".into());
        later.position = Some(StatusPosition::ReadingSource {
            line: 9_842,
            total_lines: 9_842,
        });
        later.progress = Some(100);

        assert_eq!(
            first.layout(900.0, StatusLineSettings::default()),
            later.layout(900.0, StatusLineSettings::default())
        );
    }
}
