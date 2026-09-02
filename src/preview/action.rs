use std::sync::Arc;

use crate::{
    document::{ByteOffset, ByteRange, DocumentId, Revision, RevisionRange},
    org_syntax::SyntaxId,
};

use super::{DocumentFormat, PreviewSnapshot, projection::VisualRow};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreviewActionTarget {
    pub(crate) document_id: DocumentId,
    pub(crate) base_revision: Revision,
    pub(crate) syntax_id: Option<SyntaxId>,
    pub(crate) source_range: ByteRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewActionIdentity {
    Syntax(SyntaxId),
    Source(ByteRange),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PreviewAction {
    ToggleCheckbox {
        target: PreviewActionTarget,
        expected: Arc<str>,
    },
    OpenLink {
        target: PreviewActionTarget,
        destination: Arc<str>,
    },
    CopyCode(PreviewActionTarget),
    OpenImage {
        target: PreviewActionTarget,
        path: Arc<str>,
    },
}

impl PreviewAction {
    pub(crate) fn target(&self) -> &PreviewActionTarget {
        match self {
            Self::ToggleCheckbox { target, .. } | Self::CopyCode(target) => target,
            Self::OpenLink { target, .. } | Self::OpenImage { target, .. } => target,
        }
    }
}

pub(in crate::preview) fn checkbox_action(
    document: &PreviewSnapshot,
    row: &VisualRow,
) -> Option<PreviewAction> {
    const CHECKBOX_PREFIX_BYTES: u64 = 256;
    let mut end = row
        .source
        .range
        .end
        .0
        .min(row.source.range.start.0 + CHECKBOX_PREFIX_BYTES);
    while end > row.source.range.start.0 && !text_boundary(document.text.as_ref(), ByteOffset(end))
    {
        end -= 1;
    }
    let line = document
        .text
        .copy_range(ByteRange::new(row.source.range.start.0, end));
    let (token, expected) = crate::org_syntax::command::checkbox_token(&line)?;
    let start = row.source.range.start.0 + token.start as u64;
    let source_range = ByteRange::new(start, start + token.len() as u64);
    let syntax_id = (document.format == DocumentFormat::Org)
        .then(|| {
            document
                .blocks
                .nodes()
                .get(row.block_id as usize)
                .map(|block| block.syntax_id)
        })
        .flatten();
    Some(PreviewAction::ToggleCheckbox {
        target: PreviewActionTarget {
            document_id: document.document_id,
            base_revision: row.source.revision,
            syntax_id,
            source_range,
        },
        expected: expected.into(),
    })
}

fn text_boundary(text: &dyn crate::document::TextSnapshot, offset: ByteOffset) -> bool {
    if offset.0 == text.len_bytes() {
        return true;
    }
    text.chunk_at(offset).is_some_and(|chunk| {
        usize::try_from(offset.0 - chunk.start.0)
            .ok()
            .is_some_and(|local| chunk.text.is_char_boundary(local))
    })
}

pub(in crate::preview) fn source_action_target(
    document: &PreviewSnapshot,
    row: &VisualRow,
) -> PreviewActionTarget {
    let syntax_id = (document.format == DocumentFormat::Org)
        .then(|| {
            document
                .blocks
                .nodes()
                .get(row.block_id as usize)
                .map(|block| block.syntax_id)
        })
        .flatten();
    PreviewActionTarget {
        document_id: document.document_id,
        base_revision: row.source.revision,
        syntax_id,
        source_range: row.source.range,
    }
}

pub(in crate::preview) fn code_action(
    document: &PreviewSnapshot,
    display_row: usize,
) -> Option<PreviewAction> {
    let row = document.projection.rows.get(display_row)?;
    let range = row.code_action_range?;
    let syntax_id = if document.format == DocumentFormat::Org {
        Some(
            document
                .blocks
                .nodes()
                .get(row.block_id as usize)?
                .syntax_id,
        )
    } else {
        None
    };
    Some(PreviewAction::CopyCode(PreviewActionTarget {
        document_id: document.document_id,
        base_revision: document.revision,
        syntax_id,
        source_range: range,
    }))
}

pub(in crate::preview) fn image_action(
    document: &PreviewSnapshot,
    row: &VisualRow,
    path: impl Into<Arc<str>>,
) -> PreviewAction {
    PreviewAction::OpenImage {
        target: source_action_target(document, row),
        path: path.into(),
    }
}

impl PreviewActionTarget {
    pub(crate) fn revision_range(&self) -> RevisionRange {
        RevisionRange {
            revision: self.base_revision,
            range: self.source_range,
        }
    }

    pub(crate) fn identity(&self) -> PreviewActionIdentity {
        self.syntax_id.map_or(
            PreviewActionIdentity::Source(self.source_range),
            PreviewActionIdentity::Syntax,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewActionVisualState {
    Pending,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CopyFeedbackState {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingPreviewAction {
    pub(crate) id: u64,
    pub(crate) identity: PreviewActionIdentity,
    pub(crate) committed_revision: Option<Revision>,
}
