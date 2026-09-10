use std::sync::Arc;

use crate::{i18n::Language, navigation::PaneId, settings::StatusLineSettings};

use crate::preview::{DocumentFormat, PreviewStyleId};

pub(crate) const DOCUMENT_PANE_ID: PaneId = PaneId(1);
pub(crate) const DIRED_PANE_ID: PaneId = PaneId(2);
pub(crate) const RIGHT_DOCUMENT_PANE_ID: PaneId = PaneId(3);
pub(crate) const AGENDA_PANE_ID: PaneId = PaneId(4);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusHost {
    Reading,
    Editor,
    Dired,
    Agenda,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusPosition {
    ReadingSource { line: u64, total_lines: u64 },
    EditorCaret { line: u64, column: u64 },
    DiredSelection { selected: usize, total: usize },
    AgendaSelection { selected: usize, total: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusTone {
    Working,
    Success,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StatusMessage {
    pub text: Arc<str>,
    pub tone: StatusTone,
}

#[derive(Clone, Debug)]
pub(crate) struct StatusLineSnapshot {
    pub(crate) pane: PaneId,
    pub(crate) language: Language,
    pub(crate) host: StatusHost,
    pub(crate) surface: crate::app::PaneSurface,
    pub(crate) dirty: bool,
    pub(crate) reading_style: Option<PreviewStyleId>,
    pub(crate) outline: Option<Arc<str>>,
    pub(crate) position: Option<StatusPosition>,
    pub(crate) progress: Option<u8>,
    pub(crate) statistics: Option<Arc<str>>,
    pub(crate) document_statistics: Option<DocumentStatistics>,
    pub(crate) format: Option<DocumentFormat>,
    pub(crate) transient: Option<StatusMessage>,
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

    pub(crate) fn has_segment(&self, segment: StatusSegment) -> bool {
        match segment {
            StatusSegment::Mode | StatusSegment::More => true,
            StatusSegment::ReadingStyle => self.reading_style.is_some(),
            StatusSegment::Outline => {
                self.outline.is_some()
                    || matches!(self.host, StatusHost::Editor | StatusHost::Reading)
            }
            StatusSegment::Position => self.position.is_some(),
            StatusSegment::Progress => self.progress.is_some(),
            StatusSegment::Statistics => self.statistics.is_some(),
            StatusSegment::Format => self.format.is_some(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentStatistics {
    pub characters: u64,
    pub lines: u64,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Variant {
    Full,
    Compact,
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StatusLineLayout {
    pub(crate) mode: Variant,
    pub(crate) reading_style: Variant,
    pub(crate) outline: Variant,
    pub(crate) position: Variant,
    pub(crate) progress: Variant,
    pub(crate) statistics: Variant,
    pub(crate) format: Variant,
    pub(crate) outline_max_width: f32,
    pub(crate) overflow: Arc<[StatusSegment]>,
}

impl StatusLineLayout {
    pub(crate) fn variant(&self, segment: StatusSegment) -> Variant {
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
pub(crate) enum StatusSegment {
    Mode,
    Outline,
    Position,
    Progress,
    Statistics,
    Format,
    More,
    ReadingStyle,
}

pub(crate) const CONFIGURABLE_SEGMENTS: [StatusSegment; 5] = [
    StatusSegment::Outline,
    StatusSegment::Position,
    StatusSegment::Progress,
    StatusSegment::Statistics,
    StatusSegment::Format,
];

impl StatusSegment {
    pub(crate) fn enabled(self, settings: StatusLineSettings) -> bool {
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
pub(crate) struct StatusPopover {
    pub(crate) pane: PaneId,
    pub(crate) content: StatusPopoverContent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StatusPopoverContent {
    Outline {
        document: crate::document::DocumentId,
        entries: Arc<[(Arc<str>, crate::document::RevisionRange)]>,
    },
    ReadingStyle,
    Info(StatusSegment),
    Overflow(Arc<[StatusSegment]>),
    Customize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StatusLayoutKey {
    pub width_bits: u32,
    pub settings: StatusLineSettings,
    pub host: StatusHost,
    pub surface: crate::app::PaneSurface,
    pub reading_style: bool,
    pub language: Language,
    pub outline: bool,
    pub position: bool,
    pub progress: bool,
    pub statistics: bool,
    pub document_statistics: bool,
    pub format: Option<DocumentFormat>,
}

#[derive(Clone, Debug)]
pub(crate) struct CachedStatusLayout {
    pub(crate) key: StatusLayoutKey,
    pub(crate) layout: StatusLineLayout,
}
