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
mod icons;
pub(crate) mod shell;
use icons::{StatusIcon, status_icon};
mod model;
mod popover;
#[cfg(test)]
use host::reading_progress;
use model::*;
pub(crate) use model::{CachedStatusLayout, StatusLineLayout, StatusLineSnapshot, StatusPopover};
pub(crate) use popover::render_status_popover;

pub(crate) struct StatusLineHost {
    shell: shell::ShellHost,
    settings: StatusLineSettings,
    popover: Option<StatusPopover>,
    layout_cache: std::cell::RefCell<std::collections::HashMap<PaneId, CachedStatusLayout>>,
}

impl StatusLineHost {
    pub(crate) fn new(settings: StatusLineSettings) -> Self {
        Self {
            shell: Default::default(),
            settings,
            popover: None,
            layout_cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }

    pub(crate) fn settings(&self) -> StatusLineSettings {
        self.settings
    }

    pub(crate) fn shell_owns(&self, kind: shell::ShellKind, pane: super::PaneSide) -> bool {
        self.shell.owns(kind, pane)
    }

    pub(crate) fn sample_shell(
        &self,
        available: f32,
        now: std::time::Instant,
    ) -> shell::ShellShape {
        self.shell.motion.sample(available, now).0
    }

    #[cfg(test)]
    pub(crate) fn set_shell_shape_for_test(
        &mut self,
        shape: shell::ShellShape,
        available: f32,
        now: std::time::Instant,
        animate: bool,
    ) {
        self.shell.motion.update(shape, available, now, animate);
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

pub(crate) const STATUS_LINE_HEIGHT: f32 = 38.0;
pub(crate) const FLOATING_STATUS_INSET: f32 = 20.0;
pub(crate) const FLOATING_STATUS_BOTTOM: f32 = 12.0;
pub(crate) const FLOATING_STATUS_HEIGHT: f32 = 42.0;
pub(crate) const FLOATING_STATUS_CLEARANCE: f32 = 66.0;
const OUTLINE_FULL_RESERVE: f32 = 260.0;
const OUTLINE_COMPACT_WIDTH: f32 = 150.0;
// Fixed slots are shared by layout budgeting and rendering. Values never resize them.
const MODE_FULL_WIDTH: f32 = 86.0;
const MODE_COMPACT_WIDTH: f32 = 41.0;
const STYLE_FULL_WIDTH: f32 = 90.0;
const STYLE_COMPACT_WIDTH: f32 = 50.0;
const POSITION_FULL_WIDTH: f32 = 88.0;
const POSITION_COMPACT_WIDTH: f32 = 44.0;
const PROGRESS_MARGIN: f32 = 8.0;
const PROGRESS_FULL_WIDTH: f32 = 68.0;
const PROGRESS_COMPACT_WIDTH: f32 = 44.0;
const STATISTICS_FULL_WIDTH: f32 = 308.0;
const STATISTICS_COMPACT_WIDTH: f32 = 90.0;
const FORMAT_WIDTH: f32 = 64.0;
const MORE_WIDTH: f32 = 36.0;

// Status colors deliberately stay independent from document syntax colors. A status should
// communicate state consistently even when the active theme uses red for its first heading.
// Light values preserve the original hand-tuned palette; dark values follow the product
// dark palette (status-bg #111A24, success #36D399, warning, error).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StatusColors {
    clean: u32,
    clean_hover: u32,
    dirty: u32,
    dirty_hover: u32,
    working_text: u32,
    success_text: u32,
    error_text: u32,
    progress: u32,
    progress_border: u32,
    background: u32,
    border: u32,
    foreground: u32,
    /// Text on the mode/save chips against the status-bar background. Light
    /// uses the deep navy chip ink; dark must flip to a light tone because the
    /// chips sit on the dark bar, not on a colored dot.
    mode_text: u32,
    hover_background: u32,
    hover_foreground: u32,
    active_background: u32,
    active_foreground: u32,
    badge_background: u32,
    badge_foreground: u32,
    floating_background: u32,
}

static STATUS_LIGHT: StatusColors = StatusColors {
    clean: 0x079e70,
    clean_hover: 0xeaf3f8,
    dirty: 0xf5b718,
    dirty_hover: 0xeaf3f8,
    working_text: 0x627795,
    success_text: 0x079e70,
    error_text: 0xa14f5d,
    progress: 0x079e70,
    progress_border: 0xe3f1ed,
    background: 0xf7faff,
    border: 0xdce4ee,
    foreground: 0x60718c,
    mode_text: 0x34445b,
    hover_background: 0xeaf0f8,
    hover_foreground: 0x5278b5,
    active_background: 0xdfe9f8,
    active_foreground: 0x4977cf,
    badge_background: 0xe4eaff,
    badge_foreground: 0x4565cd,
    floating_background: 0xf7f9fbf5,
};

static STATUS_DARK: StatusColors = StatusColors {
    clean: 0x36d399,
    clean_hover: 0x253444,
    dirty: 0xf5b84b,
    dirty_hover: 0x253444,
    working_text: 0x8b98a7,
    success_text: 0x36d399,
    error_text: 0xf06a7a,
    progress: 0x36d399,
    progress_border: 0x253445,
    // One step lighter than the window bottom so the bar reads as its own
    // surface against the editor background.
    background: 0x141f2b,
    border: 0x2a3a4d,
    foreground: 0x9fadc0,
    mode_text: 0xe6edf3,
    hover_background: 0x253444,
    hover_foreground: 0x8b98a7,
    active_background: 0x1b3a5c,
    active_foreground: 0x66b3ff,
    badge_background: 0x1b3a5c,
    badge_foreground: 0x66b3ff,
    floating_background: 0x1e2b39f5,
};

fn status_colors() -> &'static StatusColors {
    if crate::theme::effective_theme_is_dark() {
        &STATUS_DARK
    } else {
        &STATUS_LIGHT
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModeColors {
    background: u32,
    hover: u32,
}

fn mode_colors(dirty: bool) -> ModeColors {
    let status = status_colors();
    if dirty {
        ModeColors {
            background: status.dirty,
            hover: status.dirty_hover,
        }
    } else {
        ModeColors {
            background: status.clean,
            hover: status.clean_hover,
        }
    }
}

fn status_tone_color(tone: StatusTone) -> u32 {
    let status = status_colors();
    match tone {
        StatusTone::Working => status.working_text,
        StatusTone::Success => status.success_text,
        StatusTone::Error => status.error_text,
    }
}

impl StatusLineSnapshot {
    fn layout_key(&self, width: f32, settings: StatusLineSettings) -> StatusLayoutKey {
        StatusLayoutKey {
            width_bits: width.to_bits(),
            settings,
            host: self.host,
            surface: self.surface,
            reading_style: self.reading_style.is_some(),
            language: self.language,
            outline: self.has_segment(StatusSegment::Outline),
            position: self.position.is_some(),
            progress: self.progress.is_some(),
            statistics: self.statistics.is_some(),
            document_statistics: self.document_statistics.is_some(),
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
            outline: if settings.outline && self.has_segment(StatusSegment::Outline) {
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
                Degrade::CompactStatistics,
                Degrade::HideFormat,
                Degrade::CompactOutline,
                Degrade::CompactProgress,
                Degrade::CompactPosition,
                Degrade::CompactMode,
                Degrade::CompactReadingStyle,
                Degrade::HideOutline,
                Degrade::HideProgress,
                Degrade::HideStatistics,
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
        match (self.language, self.host, self.surface) {
            (Language::Chinese, StatusHost::Dired, _) => "文件",
            (Language::Chinese, StatusHost::Agenda, _) => "日程",
            (Language::Chinese, _, crate::app::PaneSurface::Editor) => "编辑",
            (Language::Chinese, _, crate::app::PaneSurface::Reading) => "阅读",
            (_, StatusHost::Dired, _) => "Files",
            (_, StatusHost::Agenda, _) => "Agenda",
            (_, _, crate::app::PaneSurface::Editor) => "Edit",
            (_, _, crate::app::PaneSurface::Reading) => "Read",
        }
    }
}

#[derive(Clone, Copy)]
enum Degrade {
    CompactStatistics,
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
            Degrade::CompactStatistics if self.statistics == Variant::Full => {
                self.statistics = Variant::Compact
            }
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
        let mode = mode_slot_width(self.mode);
        let reading_style = if snapshot.reading_style.is_some() {
            style_slot_width(self.reading_style)
        } else {
            0.0
        };
        let outline = self.outline_reserve(snapshot);
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
            (Variant::Full, Some(_)) => progress_slot_width(false, measure) + PROGRESS_MARGIN,
            (Variant::Compact, Some(_)) => progress_slot_width(true, measure) + PROGRESS_MARGIN,
            _ => 0.0,
        };
        let statistics = statistics_slot_width(snapshot, self.statistics);
        let format = if self.format == Variant::Full && snapshot.format.is_some() {
            FORMAT_WIDTH
        } else {
            0.0
        };
        // More is mandatory. The flexible center keeps a small hit target even when empty.
        mode + reading_style
            + outline
            + position
            + progress
            + statistics
            + format
            + MORE_WIDTH
            + 16.0
    }

    fn outline_reserve(&self, snapshot: &StatusLineSnapshot) -> f32 {
        let reserve = match self.outline {
            Variant::Full => OUTLINE_FULL_RESERVE,
            Variant::Compact => OUTLINE_COMPACT_WIDTH,
            Variant::Hidden => return 0.0,
        };
        // Reading's extra control shares the navigation budget rather than pushing
        // otherwise identical right-hand controls into a different layout.
        let style = if snapshot.reading_style.is_some() {
            style_slot_width(self.reading_style)
        } else {
            0.0
        };
        (reserve - style).max(0.0)
    }

    fn resolved_outline_width(
        &self,
        snapshot: &StatusLineSnapshot,
        available: f32,
        measure: &impl Fn(&str) -> f32,
    ) -> f32 {
        if self.outline == Variant::Hidden {
            return 0.0;
        }
        let reserved = self.outline_reserve(snapshot);
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
    UnicodeWidthStr::width(text) as f32 * 6.5
}

fn measured_text_width(text: &str, window: &Window) -> f32 {
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font(".SystemUIFont"),
        color: Hsla::default(),
        ..Default::default()
    };
    f32::from(
        window
            .text_system()
            .shape_line(text, px(11.0), &[run], None)
            .width,
    )
}

pub(crate) fn reading_style_popover_left(
    snapshot: &StatusLineSnapshot,
    layout: &StatusLineLayout,
    window: &Window,
) -> f32 {
    let _ = (snapshot, window);
    mode_slot_width(layout.mode) + 4.0
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

fn mode_slot_width(variant: Variant) -> f32 {
    match variant {
        Variant::Full => MODE_FULL_WIDTH,
        Variant::Compact => MODE_COMPACT_WIDTH,
        Variant::Hidden => 0.0,
    }
}

fn style_slot_width(variant: Variant) -> f32 {
    match variant {
        Variant::Full => STYLE_FULL_WIDTH,
        Variant::Compact => STYLE_COMPACT_WIDTH,
        Variant::Hidden => 0.0,
    }
}

fn statistics_slot_width(snapshot: &StatusLineSnapshot, variant: Variant) -> f32 {
    match variant {
        Variant::Hidden => 0.0,
        Variant::Full if snapshot.document_statistics.is_some() => STATISTICS_FULL_WIDTH,
        _ => STATISTICS_COMPACT_WIDTH,
    }
}

fn position_slot_width(
    _position: StatusPosition,
    compact: bool,
    _language: Language,
    _measure: &impl Fn(&str) -> f32,
) -> f32 {
    if compact {
        POSITION_COMPACT_WIDTH
    } else {
        POSITION_FULL_WIDTH
    }
}

fn progress_slot_width(compact: bool, _measure: &impl Fn(&str) -> f32) -> f32 {
    if compact {
        PROGRESS_COMPACT_WIDTH
    } else {
        PROGRESS_FULL_WIDTH
    }
}

fn statistic_field(icon: StatusIcon, value: String, width: f32) -> gpui::Div {
    div()
        .w(px(width))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(5.0))
                .child(status_icon(icon))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(value),
                ),
        )
}

fn outline_is_truncated(
    path: &str,
    compact: bool,
    width: f32,
    measure: impl Fn(&str) -> f32,
) -> bool {
    let parts: Vec<_> = if compact {
        vec![leaf(path)]
    } else {
        path.split(" / ").collect()
    };
    // Button padding + border + icon + gap; each separator has two gaps.
    let needed = 37.0
        + parts.iter().map(|title| measure(title)).sum::<f32>()
        + parts.len().saturating_sub(1) as f32 * 22.0;
    needed > width
}

fn breadcrumb(path: &str, compact: bool) -> gpui::Div {
    let parts: Vec<_> = if compact {
        vec![leaf(path)]
    } else {
        path.split(" / ").collect()
    };
    let mut row = div()
        .min_w_0()
        .flex_1()
        .flex()
        .items_center()
        .gap(px(4.0))
        .overflow_hidden();
    for (index, title) in parts.iter().enumerate() {
        if index > 0 {
            row = row.child(
                gpui::svg()
                    .data(include_bytes!("assets/status-chevron-right.svg"))
                    .size(px(14.0))
                    .flex_none()
                    .text_color(rgb(status_colors().foreground)),
            );
        }
        row = row.child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child((*title).to_owned()),
        );
    }
    row
}

fn status_value(text: String, width: Option<f32>) -> gpui::Div {
    div()
        .min_w_0()
        .when_some(width, |value, width| value.w(px(width)).flex_none())
        .when(width.is_none(), |value| value.flex_1())
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text)
}

pub(crate) fn render_status_line(
    snapshot: &StatusLineSnapshot,
    layout: StatusLineLayout,
    entity: Entity<WorkspaceWindow>,
    window: &Window,
) -> gpui::AnyElement {
    render_status_line_content(snapshot, layout, entity, window, None, false, None)
}

#[cfg(test)]
pub(crate) fn render_floating_status_line(
    snapshot: &StatusLineSnapshot,
    layout: StatusLineLayout,
    entity: Entity<WorkspaceWindow>,
    window: &Window,
    echo: Option<gpui::AnyElement>,
) -> gpui::AnyElement {
    floating_status_container(FLOATING_STATUS_HEIGHT)
        .id(format!("pane-{}-floating-status", snapshot.pane.0))
        .child(render_status_line_content(
            snapshot, layout, entity, window, echo, true, None,
        ))
        .into_any_element()
}

/// Both status and search occupy the same floating shell and bottom anchor.
pub(crate) fn floating_status_container(height: f32) -> gpui::Div {
    div()
        .debug_selector(|| "floating-status-line".to_owned())
        .block_mouse_except_scroll()
        .absolute()
        .left(px(FLOATING_STATUS_INSET))
        .right(px(FLOATING_STATUS_INSET))
        .bottom(px(FLOATING_STATUS_BOTTOM))
        .h(px(height))
        .rounded(px(10.0))
        .border_1()
        .border_color(rgb(status_colors().border))
        .bg(gpui::rgba(status_colors().floating_background))
        .overflow_hidden()
        .shadow_lg()
}

pub(crate) fn buffer_status_width(count: usize) -> f32 {
    // Icon, gap, button padding and outer spacing; reserve two digits by default.
    count.to_string().len().max(2) as f32 * 7.0 + 42.0
}

pub(crate) fn render_buffer_status_trigger(
    count: usize,
    entity: Entity<WorkspaceWindow>,
    language: Language,
) -> gpui::AnyElement {
    let title: Arc<str> = match language {
        Language::Chinese => format!("已打开 {count} 个文档 · C-x b"),
        Language::English => format!("{count} open documents · C-x b"),
    }
    .into();
    div()
        .w(px(buffer_status_width(count)))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .id("buffer-status-trigger")
                .debug_selector(|| "buffer-status-trigger".to_owned())
                .h(px(28.0))
                .w(px(buffer_status_width(count) - 8.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(5.0))
                .rounded(px(5.0))
                .text_size(px(11.0))
                .text_color(rgb(status_colors().foreground))
                .cursor_pointer()
                .hover(|s| {
                    s.bg(rgb(status_colors().hover_background))
                        .text_color(rgb(status_colors().hover_foreground))
                })
                .active(|s| {
                    s.bg(rgb(status_colors().active_background))
                        .text_color(rgb(status_colors().active_foreground))
                })
                .tooltip(move |_, cx| cx.new(|_| popover::OutlineTooltip(title.clone())).into())
                .child(
                    gpui::svg()
                        .data(include_bytes!("assets/status-documents.svg"))
                        .text_color(rgb(status_colors().foreground))
                        .size(px(16.0))
                        .flex_none(),
                )
                .child(div().font_family("Menlo").child(count.to_string()))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |w, cx| {
                        w.open_buffer_picker(crate::app::buffers::PickerIntent::Switch, cx);
                    });
                }),
        )
        .into_any_element()
}

pub(crate) fn render_status_line_content(
    snapshot: &StatusLineSnapshot,
    layout: StatusLineLayout,
    entity: Entity<WorkspaceWindow>,
    window: &Window,
    echo: Option<gpui::AnyElement>,
    floating: bool,
    buffer_count: Option<usize>,
) -> gpui::AnyElement {
    let theme = current_theme();
    let pane_id = snapshot.pane.0;
    let mode_colors = mode_colors(snapshot.dirty);
    let mode = status_button(entity.clone(), pane_id, StatusSegment::Mode)
        .mx(px(8.0))
        .w(px(mode_slot_width(layout.mode) - 16.0))
        .flex_none()
        .my(px(5.0))
        .h(px(28.0))
        .px(px(if layout.mode == Variant::Compact {
            7.0
        } else {
            9.0
        }))
        .gap(px(6.0))
        .rounded_full()
        .border_1()
        .border_color(rgb(status_colors().border))
        .bg(rgb(status_colors().background))
        .text_color(rgb(status_colors().mode_text))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .hover(move |style| {
            style
                .bg(rgb(mode_colors.hover))
                .text_color(rgb(status_colors().mode_text))
        })
        .child(
            div()
                .relative()
                .size(px(9.0))
                .flex_none()
                .child(
                    div()
                        .absolute()
                        .top(px(-3.5))
                        .left(px(-3.5))
                        .size(px(16.0))
                        .rounded_full()
                        .bg(gpui::rgba((mode_colors.background << 8) | 0x14)),
                )
                .child(
                    div()
                        .relative()
                        .size_full()
                        .rounded_full()
                        .bg(rgb(mode_colors.background)),
                ),
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
                    .w(px(style_slot_width(layout.reading_style)))
                    .overflow_hidden()
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
        .when(
            layout.outline != Variant::Hidden && echo.is_none(),
            |left| {
                let outline = snapshot
                    .outline
                    .as_deref()
                    .unwrap_or(match snapshot.language {
                        Language::Chinese => "正文",
                        Language::English => "Body",
                    });
                left.child(
                    status_button(entity.clone(), pane_id, StatusSegment::Outline)
                        .w(px(layout.outline_max_width))
                        .when(
                            outline_is_truncated(
                                outline,
                                layout.outline == Variant::Compact,
                                layout.outline_max_width,
                                |text| measured_text_width(text, window),
                            ),
                            |button| {
                                let full_path: Arc<str> = outline.into();
                                button.tooltip(move |_, cx| {
                                    cx.new(|_| popover::OutlineTooltip(full_path.clone()))
                                        .into()
                                })
                            },
                        )
                        .overflow_hidden()
                        .child(status_icon(StatusIcon::Outline))
                        .child(breadcrumb(outline, layout.outline == Variant::Compact)),
                )
            },
        );

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
            center
                .gap(px(7.0))
                .child(match message.tone {
                    StatusTone::Success => "✓",
                    StatusTone::Working => "◌",
                    StatusTone::Error => "!",
                })
                .child(message.text.to_string())
        });

    let mut right = div().h_full().flex().items_center().flex_none();
    if layout.statistics != Variant::Hidden
        && let Some(statistics) = snapshot.statistics.as_deref()
    {
        let button = status_button(entity.clone(), pane_id, StatusSegment::Statistics)
            .w(px(statistics_slot_width(snapshot, layout.statistics)))
            .overflow_hidden()
            .gap(px(0.0));
        right = right.child(
            if layout.statistics == Variant::Full
                && let Some(stats) = snapshot.document_statistics
            {
                button
                    .child(statistic_field(
                        StatusIcon::Document,
                        statistics.to_owned(),
                        96.0,
                    ))
                    .child(status_separator())
                    .child(statistic_field(
                        StatusIcon::Lines,
                        match snapshot.language {
                            Language::Chinese => format!("{} 行", format_integer(stats.lines)),
                            Language::English => format!("{} lines", format_integer(stats.lines)),
                        },
                        96.0,
                    ))
                    .child(status_separator())
                    .child(statistic_field(
                        StatusIcon::Storage,
                        format_byte_count(stats.bytes),
                        76.0,
                    ))
            } else {
                button.child(status_value(statistics.to_owned(), None))
            },
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
                .justify_center()
                .overflow_hidden()
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .when(layout.position == Variant::Full, |group| {
                            group.child(status_icon(StatusIcon::Position))
                        })
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family("Menlo")
                                .child(position_text(position, compact, snapshot.language)),
                        ),
                ),
        );
    }
    if layout.progress != Variant::Hidden
        && let Some(progress) = snapshot.progress
    {
        let compact = layout.progress == Variant::Compact;
        let slot_width = progress_slot_width(compact, &|text| measured_text_width(text, window));
        right = right.child(
            status_button(entity.clone(), pane_id, StatusSegment::Progress)
                .mr(px(PROGRESS_MARGIN))
                .w(px(slot_width))
                .rounded_full()
                .border_1()
                .border_color(rgb(status_colors().progress_border))
                .text_color(rgb(status_colors().progress))
                .overflow_hidden()
                .when(layout.progress == Variant::Full, |button| {
                    button.child(progress_ring(progress))
                })
                .child(status_value(format!("{progress}%"), None).font_family("Menlo")),
        );
    }
    if layout.format != Variant::Hidden
        && let Some(format) = snapshot.format
    {
        right = right.child(
            status_button(entity.clone(), pane_id, StatusSegment::Format)
                .w(px(FORMAT_WIDTH))
                .child(status_icon(StatusIcon::Format))
                .child(format_label(format)),
        );
    }
    if let Some(count) = buffer_count {
        right = right.child(render_buffer_status_trigger(
            count,
            entity.clone(),
            snapshot.language,
        ));
    }
    let overflow = layout.overflow.clone();
    let more_entity = entity.clone();
    right = right.child(
        status_button(entity.clone(), pane_id, StatusSegment::More)
            .size(px(28.0))
            .mx(px((MORE_WIDTH - 28.0) / 2.0))
            .px(px(0.0))
            .justify_center()
            .relative()
            .rounded_full()
            .border_1()
            .child(
                gpui::svg()
                    .data(include_bytes!("assets/status-settings.svg"))
                    .text_color(rgb(status_colors().foreground))
                    .size(px(16.0))
                    .flex_none(),
            )
            .when(!layout.overflow.is_empty(), |button| {
                button.child(
                    div()
                        .absolute()
                        .top(px(-3.0))
                        .right(px(0.0))
                        .min_w(px(13.0))
                        .h(px(13.0))
                        .px(px(3.0))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgb(status_colors().badge_background))
                        .text_color(rgb(status_colors().badge_foreground))
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
        .h(px(if floating {
            FLOATING_STATUS_HEIGHT - 2.0
        } else {
            STATUS_LINE_HEIGHT
        }))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .overflow_hidden()
        .when(!floating, |bar| {
            bar.border_t_1().bg(rgb(status_colors().background))
        })
        .border_color(rgb(status_colors().border))
        .text_color(rgb(status_colors().foreground))
        .font_family(".SystemUIFont")
        .text_size(px(11.0))
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
        .child(echo.unwrap_or_else(|| center.into_any_element()))
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
        .h(px(26.0))
        .flex_none()
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .cursor_pointer()
        .border_l_1()
        .border_color(rgb(status_colors().border));
    let button = if segment == StatusSegment::Mode {
        button
    } else {
        button.hover(|style| {
            style
                .bg(rgb(status_colors().hover_background))
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
    let foreground = rgb(status_colors().progress);
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
            StatusSegment::Outline if self.content_route == ContentRoute::Document => {
                if self.status.popover.as_ref().is_some_and(|popover| {
                    popover.pane == pane
                        && matches!(
                            popover.content,
                            StatusPopoverContent::Outline { .. }
                                | StatusPopoverContent::Info(StatusSegment::Outline)
                        )
                }) {
                    self.status.popover = None;
                    cx.notify();
                    return;
                }
                let side = pane_side_for_status(pane);
                let content = self
                    .state
                    .ready()
                    .and_then(|ready| {
                        let session = ready.session.read(cx);
                        let entries = if let Some(editor) = ready.editors.get(side).as_ref() {
                            editor.read(cx).outline_entries(cx)
                        } else {
                            let panel = ready.readers.get(side).as_ref()?.read(cx);
                            let preview = panel.document();
                            if preview.document_id != session.id()
                                || preview.revision != session.revision()
                            {
                                return None;
                            }
                            preview.outline_entries()
                        };
                        Some(StatusPopoverContent::Outline {
                            document: session.id(),
                            entries: entries.into(),
                            collapsed: Default::default(),
                        })
                    })
                    .unwrap_or(StatusPopoverContent::Info(segment));
                self.status.popover = Some(StatusPopover { pane, content });
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

fn status_separator() -> impl IntoElement {
    div()
        .w(px(1.0))
        .h(px(14.0))
        .mx(px(5.0))
        .bg(rgb(status_colors().border))
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
            Language::Chinese => "当前为编辑界面。位置显示光标行列，进度显示滚动位置；点击可立即切换当前面板的编辑或阅读界面。".to_owned(),
            Language::English => {
                "Editing is active. Position uses the caret and progress uses the viewport. Click to switch this pane between editing and reading.".to_owned()
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

        assert_eq!(clean.background, status_colors().clean);
        assert_eq!(dirty.background, status_colors().dirty);
        assert_ne!(clean.background, dirty.background);
        assert_ne!(clean.background, current_theme().heading[0]);
        assert_ne!(dirty.background, current_theme().heading[0]);
        assert_ne!(status_colors().progress, current_theme().heading[0]);
        assert_eq!(
            status_tone_color(StatusTone::Error),
            status_colors().error_text
        );
    }

    #[test]
    fn responsive_layout_degrades_without_hiding_mandatory_segments() {
        let settings = StatusLineSettings::default();
        let wide = snapshot().layout(1_400.0, settings);
        assert_eq!(wide.mode, Variant::Full);
        assert_eq!(wide.statistics, Variant::Full);
        assert!(wide.overflow.is_empty());

        let split = snapshot().layout(430.0, settings);
        assert_eq!(split.statistics, Variant::Compact);
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
    fn outline_rows_wrap_and_fold_without_navigating(cx: &mut gpui::TestAppContext) {
        use crate::document::{DocumentSnapshot, HeadingIndex};
        use gpui::{AppContext, Context, IntoElement, Modifiers, Render};
        let text = format!(
            "* {}\n*** Child / title\n* Sibling\n",
            "很长的章节标题".repeat(30)
        );
        let document = DocumentSnapshot::from_utf8(text.into_bytes()).unwrap();
        let entries =
            HeadingIndex::parse(DocumentFormat::Org, &document).outline_entries(&document);
        let app = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        app.update(cx, |app, _| {
            app.status.popover = Some(StatusPopover {
                pane: DOCUMENT_PANE_ID,
                content: StatusPopoverContent::Outline {
                    document: document.document_id(),
                    entries: entries.into(),
                    collapsed: Default::default(),
                },
            })
        });
        struct OutlineHarness {
            app: Entity<WorkspaceWindow>,
            _subscription: gpui::Subscription,
        }
        impl Render for OutlineHarness {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let popover = self.app.read(cx).status.popover.clone();
                div()
                    .relative()
                    .w(px(500.0))
                    .h(px(600.0))
                    .children(popover.map(|popover| {
                        render_status_popover(
                            popover,
                            None,
                            StatusLineSettings::default(),
                            self.app.clone(),
                            Language::Chinese,
                            500.0,
                            0.0,
                        )
                    }))
            }
        }
        let (_, cx) = cx.add_window_view(|_, cx| OutlineHarness {
            app: app.clone(),
            _subscription: cx.observe(&app, |_, _, cx| cx.notify()),
        });
        let parent = cx.debug_bounds("outline-row-0").unwrap();
        assert!(
            f32::from(parent.size.height) <= 60.0,
            "long titles must stay within two lines"
        );
        assert!(cx.debug_bounds("outline-row-1").is_some());
        let fold = cx.debug_bounds("outline-fold-0").unwrap();
        cx.simulate_click(fold.center(), Modifiers::default());
        cx.update(|_, cx| {
            assert!(
                matches!(&app.read(cx).status.popover.as_ref().unwrap().content,
            StatusPopoverContent::Outline { collapsed, .. } if collapsed.contains(&0))
            )
        });
        assert!(cx.debug_bounds("outline-row-1").is_none());
        assert!(cx.debug_bounds("outline-row-2").is_some());
        let fold = cx.debug_bounds("outline-fold-0").unwrap();
        cx.simulate_click(fold.center(), Modifiers::default());
        assert!(cx.debug_bounds("outline-row-1").is_some());
    }

    #[gpui::test]
    fn clicking_the_rendered_more_segment_opens_its_pane_overflow(cx: &mut gpui::TestAppContext) {
        use gpui::{AppContext, Context, IntoElement, Modifiers, Render, point};

        let app = cx.update(|cx| cx.new(|_| WorkspaceWindow::with_split_layout(true)));
        let snapshot = snapshot();
        let pane = snapshot.pane;
        let underlying_clicks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        struct StatusHarness {
            app: Entity<WorkspaceWindow>,
            snapshot: StatusLineSnapshot,
            underlying_clicks: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl Render for StatusHarness {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let clicks = self.underlying_clicks.clone();
                let layout =
                    self.snapshot
                        .layout_in_window(210.0, StatusLineSettings::default(), window);
                div()
                    .relative()
                    .w(px(250.0))
                    .h(px(400.0))
                    .child(
                        div()
                            .debug_selector(|| "underlying-pane".to_owned())
                            .size_full()
                            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                                clicks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            }),
                    )
                    .child(render_floating_status_line(
                        &self.snapshot,
                        layout,
                        self.app.clone(),
                        window,
                        None,
                    ))
            }
        }
        let (_harness, cx) = cx.add_window_view(|_, _| StatusHarness {
            app: app.clone(),
            snapshot: snapshot.clone(),
            underlying_clicks: underlying_clicks.clone(),
        });
        let pane_bounds = cx.debug_bounds("underlying-pane").unwrap();
        let floating = cx.debug_bounds("floating-status-line").unwrap();
        assert_eq!(f32::from(pane_bounds.size.height), 400.0);
        assert_eq!(f32::from(floating.size.width), 210.0);
        assert_eq!(f32::from(floating.size.height), FLOATING_STATUS_HEIGHT);
        assert_eq!(f32::from(floating.origin.x), FLOATING_STATUS_INSET);
        assert_eq!(
            f32::from(floating.origin.y),
            400.0 - FLOATING_STATUS_BOTTOM - FLOATING_STATUS_HEIGHT
        );
        let bounds = cx
            .debug_bounds("status-7-segment-6")
            .expect("More segment should be painted");
        assert!(f32::from(bounds.origin.x + bounds.size.width) <= 230.0);
        let center = point(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        );
        cx.simulate_mouse_move(center, None, Modifiers::default());
        cx.simulate_click(center, Modifiers::default());

        assert_eq!(
            underlying_clicks.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "status button mouse-down must not start a selection underneath"
        );
        let blank = point(floating.origin.x + px(4.0), floating.origin.y + px(4.0));
        cx.simulate_click(blank, Modifiers::default());
        assert_eq!(
            underlying_clicks.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the floating bar's padding must also block mouse-down"
        );
        cx.simulate_click(point(px(5.0), px(350.0)), Modifiers::default());
        assert_eq!(
            underlying_clicks.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the editor outside the floating bar must remain clickable"
        );

        let popover = cx.update(|_, cx| app.read(cx).status.popover.clone().unwrap());
        assert_eq!(popover.pane, pane);
        assert!(matches!(
            popover.content,
            StatusPopoverContent::Overflow(ref segments) if !segments.is_empty()
        ));
    }

    #[gpui::test]
    fn clicking_a_status_surface_control_immediately_toggles_its_own_pane(
        cx: &mut gpui::TestAppContext,
    ) {
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
            assert_eq!(
                app.document_workspace.surface(crate::app::PaneSide::Right),
                crate::app::PaneSurface::Reading
            );
            assert!(app.status.popover.is_none());
        });
        cx.simulate_click(center, Modifiers::default());
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(
                app.document_workspace.surface(crate::app::PaneSide::Left),
                crate::app::PaneSurface::Editor
            );
            assert_eq!(
                app.document_workspace.surface(crate::app::PaneSide::Right),
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
    fn editor_outline_is_available_before_reading_and_tracks_edits(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        for (extension, source, expected) in [
            ("org", "* Parent\n** Child\nbody\n", "Parent / Child"),
            ("md", "# Parent\n## Child\nbody\n", "Parent / Child"),
        ] {
            let path = std::env::temp_dir().join(format!(
                "status-source-outline-{}.{}",
                std::process::id(),
                extension
            ));
            std::fs::write(&path, source).unwrap();
            let loaded = crate::preview::load_workspace_document(path.clone(), false).unwrap();
            std::fs::remove_file(path).unwrap();
            let app = cx.new(|_| WorkspaceWindow::with_split_layout(false));
            app.update(cx, |app, cx| {
                app.generation = 1;
                assert!(app.apply_load_result(1, Ok(loaded), cx));
                let ready = app.state.ready().unwrap();
                assert!(ready.readers.left.is_none());
                assert!(ready.readers.right.is_none());
                let editor = ready.editors.left.clone().unwrap();
                editor.update(cx, |editor, cx| editor.set_selection(crate::document::Selection::caret(crate::document::ByteOffset(source.len() as u64)), cx));
                assert_eq!(app.status_snapshot(cx).unwrap().outline.as_deref(), Some(expected));
                app.activate_status_segment(DOCUMENT_PANE_ID, StatusSegment::Outline, None, cx);
                assert!(matches!(&app.status.popover.as_ref().unwrap().content, StatusPopoverContent::Outline { entries, .. } if entries.len() == 2 && entries[1].title.as_ref() == "Child" && entries[1].level == 2));
                app.activate_status_segment(DOCUMENT_PANE_ID, StatusSegment::Outline, None, cx);
                assert!(app.status.popover.is_none(), "clicking the same outline entry closes it");
                app.activate_status_segment(DOCUMENT_PANE_ID, StatusSegment::Outline, None, cx);
                assert!(matches!(app.status.popover.as_ref().map(|popover| &popover.content), Some(StatusPopoverContent::Outline { .. })));
                let session = app.document_session().unwrap().clone();
                session.update(cx, |session, cx| {
                    let start = source.find("Child").unwrap() as u64;
                    session.apply_transient_edit(crate::document::EditTransaction::new(session.revision(), vec![crate::document::TextEdit::new(crate::document::ByteRange::new(start, start + 5), "Renamed")]), cx).unwrap();
                });
                assert_eq!(app.status_snapshot(cx).unwrap().outline.as_deref(), Some("Parent / Renamed"));
            });
        }
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

    #[gpui::test]
    fn save_only_surfaces_errors_in_the_status_line(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let path =
            std::env::temp_dir().join(format!("status-save-feedback-{}.org", std::process::id()));
        std::fs::write(&path, "hello").unwrap();
        let loaded = crate::preview::load_document(path.clone()).unwrap();
        std::fs::remove_file(path).unwrap();
        let app = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        app.update(cx, |app, cx| {
            app.language = Language::Chinese;
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(loaded), cx));
            // Neither finishing a save nor editing shows save text: the mode button's color
            // is the only save feedback.
            assert!(app.status_snapshot(cx).unwrap().transient.is_none());
            app.save.error = None;
            assert!(app.status_snapshot(cx).unwrap().transient.is_none());

            app.save.error = Some("failed".into());
            let snapshot = app.status_snapshot(cx).unwrap();
            assert_eq!(snapshot.transient_text(), Some("failed"));
            assert_eq!(
                snapshot.transient.as_ref().map(|message| message.tone),
                Some(StatusTone::Error)
            );
        });
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
    fn changing_values_keeps_every_layout_slot_stable() {
        let mut first = snapshot();
        first.host = StatusHost::Editor;
        first.position = Some(StatusPosition::EditorCaret { line: 1, column: 1 });
        first.outline = None;
        first.statistics = Some("9 字".into());
        let mut later = first.clone();
        later.position = Some(StatusPosition::EditorCaret {
            line: 102,
            column: 37,
        });
        later.outline = Some("a much longer heading / child".into());
        later.statistics = Some("999.9k 字".into());
        later.document_statistics = Some(DocumentStatistics {
            characters: 999_999,
            lines: 99_999,
            bytes: 9_999_999,
        });
        later.progress = Some(100);
        later.transient = Some(StatusMessage {
            text: "Saved".into(),
            tone: StatusTone::Success,
        });
        for width in [210.0, 430.0, 900.0, 1400.0] {
            let settings = StatusLineSettings::default();
            assert_eq!(
                first.layout_key(width, settings),
                later.layout_key(width, settings)
            );
            let before = first.layout(width, settings);
            let after = later.layout(width, settings);
            assert_eq!(before, after);
            assert_eq!(
                before.width(&first, &text_width),
                after.width(&later, &text_width)
            );
        }
    }

    #[gpui::test]
    fn rendered_slots_do_not_move_when_caret_statistics_or_heading_change(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{AppContext, Context, Render};
        struct Harness {
            app: Entity<WorkspaceWindow>,
            snapshot: StatusLineSnapshot,
            width: f32,
        }
        impl Render for Harness {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let layout = self.snapshot.layout_in_window(
                    self.width,
                    StatusLineSettings::default(),
                    window,
                );
                div().w(px(self.width)).child(render_status_line(
                    &self.snapshot,
                    layout,
                    self.app.clone(),
                    window,
                ))
            }
        }
        let app = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        let mut first = snapshot();
        first.host = StatusHost::Editor;
        first.position = Some(StatusPosition::EditorCaret { line: 1, column: 1 });
        first.outline = None;
        let mut later = first.clone();
        later.position = Some(StatusPosition::EditorCaret {
            line: 102,
            column: 37,
        });
        later.statistics = Some("999.9k chars".into());
        later.document_statistics = Some(DocumentStatistics {
            characters: 999_999,
            lines: 123_456,
            bytes: 12_345_678,
        });
        later.outline = Some("a long chapter / an even longer child heading".into());
        later.progress = Some(100);
        later.transient = Some(StatusMessage {
            text: "Saved successfully".into(),
            tone: StatusTone::Success,
        });
        let (harness, cx) = cx.add_window_view(|_, _| Harness {
            app,
            snapshot: first.clone(),
            width: 1400.0,
        });
        let selectors = [
            "status-7-segment-0",
            "status-7-segment-1",
            "status-7-segment-2",
            "status-7-segment-3",
            "status-7-segment-4",
            "status-7-segment-5",
            "status-7-segment-6",
            "status-7-segment-7",
        ];
        for width in [1400.0, 430.0, 210.0] {
            harness.update(cx, |view, cx| {
                view.width = width;
                view.snapshot = first.clone();
                cx.notify();
            });
            let before = selectors.map(|selector| cx.debug_bounds(selector));
            harness.update(cx, |view, cx| {
                view.snapshot = later.clone();
                cx.notify();
            });
            let after = selectors.map(|selector| cx.debug_bounds(selector));
            assert_eq!(before, after, "rendered slots moved at width {width}");
        }
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
    fn outline_tooltip_only_appears_when_visible_titles_are_truncated() {
        let measure = |text: &str| text.len() as f32 * 10.0;
        assert!(!outline_is_truncated(
            "Parent / Child",
            false,
            169.0,
            measure
        ));
        assert!(outline_is_truncated(
            "Parent / Child",
            false,
            168.0,
            measure
        ));
        assert!(!outline_is_truncated("Parent / Child", true, 87.0, measure));
        assert!(outline_is_truncated("Parent / Child", true, 86.0, measure));
    }

    #[test]
    fn reading_template_uses_navigation_space_without_changing_common_controls() {
        let reading = snapshot();
        let mut editing = reading.clone();
        editing.host = StatusHost::Editor;
        editing.surface = crate::app::PaneSurface::Editor;
        editing.reading_style = None;
        editing.position = Some(StatusPosition::EditorCaret {
            line: 259,
            column: 1,
        });
        for width in [700.0, 900.0, 1100.0, 1400.0] {
            let read = reading.layout(width, StatusLineSettings::default());
            let edit = editing.layout(width, StatusLineSettings::default());
            assert_eq!(
                (read.statistics, read.position, read.progress, read.format),
                (edit.statistics, edit.position, edit.progress, edit.format),
                "width {width}"
            );
            if read.outline == Variant::Full {
                assert_eq!(
                    read.outline_max_width + style_slot_width(read.reading_style),
                    edit.outline_max_width
                );
            }
        }
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
        let other_slots = layout.width(&snapshot, &text_width) - layout.outline_reserve(&snapshot);
        assert_eq!(layout.outline_max_width, 1_400.0 - 18.0 - other_slots);
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
