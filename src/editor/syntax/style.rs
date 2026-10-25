//! Canonical editor line faces and layout metrics, shared by native text and minimaps.

use std::{path::Path, sync::Arc};

use super::{Language, TodoSpan, language};
use crate::{document::ByteRange, theme::Theme};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(in crate::editor) enum EditorStyleId {
    #[default]
    Plain,
    Heading(u8),
    List,
    Table,
    Quote,
    CodeBoundary,
    Code,
    Property,
    Meta,
    Comment,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::editor) enum EditorBlockKind {
    Source,
    Example,
    Quote,
    Verse,
    Center,
    Comment,
    Export,
    Special(Arc<str>),
    MarkdownFence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::editor) enum EditorBlockEdge {
    Open,
    Body,
    Close,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::editor) struct EditorBlockDecoration {
    pub(in crate::editor) kind: EditorBlockKind,
    pub(in crate::editor) edge: EditorBlockEdge,
    pub(in crate::editor) body_line: Option<u64>,
}

impl EditorBlockDecoration {
    pub(in crate::editor) fn is_first_source_body_line(&self) -> bool {
        self.kind == EditorBlockKind::Source
            && self.edge == EditorBlockEdge::Body
            && self.body_line == Some(1)
    }
}

impl EditorStyleId {
    /// Property and keyword prefixes have inline faces; their values inherit
    /// the body foreground in both the editor and its minimap.
    pub(in crate::editor) fn base_color(self, theme: &Theme) -> Option<u32> {
        Some(match self {
            Self::Heading(level) => {
                theme.heading[level.saturating_sub(1) as usize % theme.heading.len()]
            }
            Self::CodeBoundary => theme.meta,
            Self::Code => theme.code_foreground,
            Self::Quote => theme.quote,
            Self::Comment => theme.comment,
            Self::Table => theme.link,
            Self::Plain | Self::List | Self::Property | Self::Meta => return None,
        })
    }

    pub(in crate::editor) fn font_role(self) -> crate::typography::FontRole {
        if self == Self::Table {
            crate::typography::FontRole::Table
        } else {
            crate::typography::FontRole::Editor
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) struct BlockMetrics {
    pub(in crate::editor) font_scale: f32,
    pub(in crate::editor) font_weight: gpui::FontWeight,
    pub(in crate::editor) line_height: f32,
    pub(in crate::editor) before: f32,
    pub(in crate::editor) after: f32,
}

impl Default for BlockMetrics {
    fn default() -> Self {
        Self {
            font_scale: 1.0,
            font_weight: gpui::FontWeight::NORMAL,
            line_height: crate::editor::LINE_HEIGHT,
            before: 0.0,
            after: 0.0,
        }
    }
}

impl BlockMetrics {
    pub(in crate::editor) fn scaled(self, scale: f32) -> Self {
        Self {
            line_height: self.line_height * scale,
            before: self.before * scale,
            after: self.after * scale,
            ..self
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::editor) struct EditorLineStyle {
    pub(in crate::editor) source_range: ByteRange,
    pub(in crate::editor) id: EditorStyleId,
    pub(in crate::editor) code_language: Option<Arc<str>>,
    pub(in crate::editor) block: Option<EditorBlockDecoration>,
    pub(super) todo: Option<TodoSpan>,
    pub(in crate::editor) metrics: BlockMetrics,
}

impl EditorLineStyle {
    /// Heading prefixes, including a star-only draft, are block syntax. Keep
    /// them outside inline emphasis until a separator completes the heading.
    pub(in crate::editor) fn inline_start(&self, path: &Path, text: &str) -> usize {
        if language(path) == Language::Org && text.bytes().all(|byte| byte == b'*') {
            return text.len();
        }
        if !matches!(self.id, EditorStyleId::Heading(_)) {
            return 0;
        }
        let trimmed = text.trim_start();
        let marker = match language(path) {
            Language::Org => b'*',
            Language::Markdown => b'#',
        };
        text.len() - trimmed.len() + trimmed.bytes().take_while(|byte| *byte == marker).count()
    }

    pub(in crate::editor) fn pending_fallback(source_range: ByteRange) -> Self {
        Self {
            source_range,
            id: EditorStyleId::Plain,
            code_language: None,
            block: None,
            todo: None,
            metrics: BlockMetrics::default(),
        }
    }
}

pub(super) fn metrics_for(language: Language, style: EditorStyleId) -> BlockMetrics {
    let EditorStyleId::Heading(level) = style else {
        return BlockMetrics::default();
    };
    let index = usize::from(level.saturating_sub(1)).min(3);
    match language {
        Language::Org => {
            let font_scale = [1.3, 1.2, 1.1, 1.0][index];
            BlockMetrics {
                font_scale,
                font_weight: if level <= 2 {
                    gpui::FontWeight::BOLD
                } else {
                    gpui::FontWeight::NORMAL
                },
                line_height: crate::editor::LINE_HEIGHT * font_scale,
                ..Default::default()
            }
        }
        Language::Markdown => {
            let (font_scale, line_height, before, after) = [
                (1.50, 34.0, 12.0, 5.0),
                (1.34, 30.0, 10.0, 4.0),
                (1.20, 27.0, 8.0, 3.0),
                (1.08, 24.0, 5.0, 2.0),
            ][index];
            BlockMetrics {
                font_scale,
                font_weight: gpui::FontWeight::BOLD,
                line_height,
                before,
                after,
            }
        }
    }
}
