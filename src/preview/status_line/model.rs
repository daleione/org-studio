use std::sync::Arc;

use crate::{i18n::Language, navigation::PaneId, settings::StatusLineSettings};

use super::super::DocumentFormat;
use super::super::PreviewStyleId;

pub(super) const DOCUMENT_PANE_ID: PaneId = PaneId(1);
pub(super) const DIRED_PANE_ID: PaneId = PaneId(2);
pub(super) const RIGHT_DOCUMENT_PANE_ID: PaneId = PaneId(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusHost {
    Preview,
    Editor,
    Dired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusPosition {
    PreviewSource { line: u64, total_lines: u64 },
    EditorCaret { line: u64, column: u64 },
    DiredSelection { selected: usize, total: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusTone {
    Working,
    Success,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StatusMessage {
    pub text: Arc<str>,
    pub tone: StatusTone,
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct StatusLineSnapshot {
    pub(in crate::preview) pane: PaneId,
    pub(super) language: Language,
    pub(super) host: StatusHost,
    pub(super) surface: crate::app::PaneSurface,
    pub(super) reading_style: Option<PreviewStyleId>,
    pub(super) outline: Option<Arc<str>>,
    pub(super) position: Option<StatusPosition>,
    pub(super) progress: Option<u8>,
    pub(super) statistics: Option<Arc<str>>,
    pub(super) document_statistics: Option<DocumentStatistics>,
    pub(super) format: Option<DocumentFormat>,
    pub(super) transient: Option<StatusMessage>,
}

impl StatusLineSnapshot {
    #[cfg(test)]
    pub(crate) fn uses_editor_viewport(&self) -> bool {
        self.host == StatusHost::Editor
    }

    #[cfg(test)]
    pub(crate) fn transient_text(&self) -> Option<&str> {
        self.transient.as_ref().map(|message| message.text.as_ref())
    }

    pub(super) fn has_segment(&self, segment: StatusSegment) -> bool {
        match segment {
            StatusSegment::Mode | StatusSegment::More => true,
            StatusSegment::ReadingStyle => self.reading_style.is_some(),
            StatusSegment::Outline => self.outline.is_some(),
            StatusSegment::Position => self.position.is_some(),
            StatusSegment::Progress => self.progress.is_some(),
            StatusSegment::Statistics => self.statistics.is_some(),
            StatusSegment::Format => self.format.is_some(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DocumentStatistics {
    pub characters: u64,
    pub lines: u64,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Variant {
    Full,
    Compact,
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::preview) struct StatusLineLayout {
    pub(super) mode: Variant,
    pub(super) reading_style: Variant,
    pub(super) outline: Variant,
    pub(super) position: Variant,
    pub(super) progress: Variant,
    pub(super) statistics: Variant,
    pub(super) format: Variant,
    pub(super) outline_max_width: f32,
    pub(super) overflow: Arc<[StatusSegment]>,
}

impl StatusLineLayout {
    pub(super) fn variant(&self, segment: StatusSegment) -> Variant {
        match segment {
            StatusSegment::Mode => self.mode,
            StatusSegment::ReadingStyle => self.reading_style,
            StatusSegment::Outline => self.outline,
            StatusSegment::Position => self.position,
            StatusSegment::Progress => self.progress,
            StatusSegment::Statistics => self.statistics,
            StatusSegment::Format => self.format,
            StatusSegment::More => Variant::Full,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusSegment {
    Mode,
    Outline,
    Position,
    Progress,
    Statistics,
    Format,
    More,
    ReadingStyle,
}

pub(super) const CONFIGURABLE_SEGMENTS: [StatusSegment; 5] = [
    StatusSegment::Outline,
    StatusSegment::Position,
    StatusSegment::Progress,
    StatusSegment::Statistics,
    StatusSegment::Format,
];

impl StatusSegment {
    pub(super) fn enabled(self, settings: StatusLineSettings) -> bool {
        match self {
            Self::Outline => settings.outline,
            Self::Position => settings.position,
            Self::Progress => settings.progress,
            Self::Statistics => settings.statistics,
            Self::Format => settings.format,
            Self::Mode | Self::ReadingStyle | Self::More => true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::preview) struct StatusPopover {
    pub(in crate::preview) pane: PaneId,
    pub(super) content: StatusPopoverContent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum StatusPopoverContent {
    ReadingStyle,
    Info(StatusSegment),
    Overflow(Arc<[StatusSegment]>),
    Customize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct StatusLayoutKey {
    pub width_bits: u32,
    pub settings: StatusLineSettings,
    pub host: StatusHost,
    pub surface: crate::app::PaneSurface,
    pub reading_style: Option<PreviewStyleId>,
    pub language: Language,
    pub outline: bool,
    pub position_reserve: Option<String>,
    pub progress: bool,
    pub statistics: Option<Arc<str>>,
    pub format: Option<DocumentFormat>,
}

#[derive(Clone, Debug)]
pub(in crate::preview) struct CachedStatusLayout {
    pub(super) key: StatusLayoutKey,
    pub(super) layout: StatusLineLayout,
}
