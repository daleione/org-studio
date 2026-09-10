use std::sync::Arc;

use gpui::{ListState, px};

use crate::app::export_ui::ExportRunState;
use crate::app::save::SaveStatus;
use crate::{
    document::{ByteOffset, DocumentSession},
    i18n::Language,
    navigation::PaneId,
    preview::{DocumentFormat, PreviewSnapshot, ReadingPreviewPanel},
};

use super::super::{DiredStatus, WorkspaceLoadState, WorkspaceWindow};
use super::{
    format_character_count,
    model::{
        DocumentStatistics, StatusHost, StatusLineSnapshot, StatusMessage, StatusPosition,
        StatusTone,
    },
};

fn preview_matches_session(preview: &ReadingPreviewPanel, session: &DocumentSession) -> bool {
    preview.document().document_id == session.id()
        && preview.document().revision <= session.revision()
        && preview.document().path == session.path()
}

impl WorkspaceWindow {
    pub(crate) fn document_status_snapshot(
        &self,
        pane: crate::app::PaneSide,
        cx: &gpui::App,
    ) -> Option<StatusLineSnapshot> {
        let document = match &self.state {
            WorkspaceLoadState::Ready { document }
            | WorkspaceLoadState::Failed {
                previous: Some(document),
                ..
            } => document,
            WorkspaceLoadState::Empty
            | WorkspaceLoadState::Loading { .. }
            | WorkspaceLoadState::Failed { previous: None, .. } => return None,
        };
        let status_pane = match pane {
            crate::app::PaneSide::Left => super::model::DOCUMENT_PANE_ID,
            crate::app::PaneSide::Right => super::model::RIGHT_DOCUMENT_PANE_ID,
        };
        if matches!(
            self.document_workspace.surface(pane),
            crate::app::PaneSurface::Editor
        ) {
            self.source_status_snapshot(status_pane, pane, document, cx)
        } else {
            let panel = document.readers.get(pane).as_ref();
            panel
                .filter(|panel| {
                    let preview = panel.read(cx);
                    let session = document.session.read(cx);
                    preview_matches_session(preview, session)
                })
                .map(|panel| self.preview_status_snapshot(status_pane, pane, panel, cx))
                .or_else(|| {
                    self.source_status_snapshot(status_pane, pane, document, cx)
                        .map(|mut snapshot| {
                            snapshot.surface = crate::app::PaneSurface::Reading;
                            snapshot.reading_style = Some(self.reading_style);
                            snapshot
                        })
                })
        }
    }

    fn source_status_snapshot(
        &self,
        pane: PaneId,
        pane_side: crate::app::PaneSide,
        ready: &super::super::ReadyDocument,
        cx: &gpui::App,
    ) -> Option<StatusLineSnapshot> {
        let status = ready.editors.get(pane_side).as_ref()?.read(cx).status(cx);
        let document_statistics = DocumentStatistics {
            characters: status.characters,
            lines: status.total_lines,
            bytes: status.bytes,
        };
        let outline = ready
            .editors
            .get(pane_side)
            .as_ref()?
            .read(cx)
            .current_outline(status.caret_offset, cx);
        Some(StatusLineSnapshot {
            pane,
            language: self.language,
            host: StatusHost::Editor,
            surface: crate::app::PaneSurface::Editor,
            dirty: ready.session.read(cx).is_dirty(),
            reading_style: None,
            outline,
            position: Some(StatusPosition::EditorCaret {
                line: status.caret_line,
                column: status.caret_column,
            }),
            progress: Some(reading_progress(
                status.visible_bottom_line,
                status.total_lines,
                status.reached_end,
            )),
            statistics: Some(
                format_character_count(document_statistics.characters, self.language).into(),
            ),
            document_statistics: Some(document_statistics),
            format: Some(DocumentFormat::from_path(ready.session.read(cx).path())),
            transient: self.document_transient_status(None, cx),
        })
    }

    fn preview_status_snapshot(
        &self,
        pane: PaneId,
        pane_side: crate::app::PaneSide,
        panel: &gpui::Entity<ReadingPreviewPanel>,
        cx: &gpui::App,
    ) -> StatusLineSnapshot {
        let panel = panel.read(cx);
        let document = panel.document();
        let visible_rows = panel.visible_rows();
        let list_state = panel.list_state();
        let item_count = visible_rows.len();
        let item_index = list_state
            .logical_scroll_top()
            .item_ix
            .min(item_count.saturating_sub(1));
        let source_index = visible_rows.get(item_index).copied().unwrap_or(0);
        let source_row = document.projection.source_row(source_index);
        let line = source_row
            .as_ref()
            .map(|row| document.text.line_of_byte(row.content.range.start) + 1)
            .unwrap_or(1);
        let visible_end = visible_item_end(list_state, item_count);
        let bottom_line = visible_source_bottom_line(document, visible_rows, visible_end);
        let progress = reading_progress(
            bottom_line,
            document.statistics.lines,
            list_reached_bottom(list_state, visible_end, item_count),
        );
        let document_statistics = DocumentStatistics {
            characters: document.statistics.characters,
            lines: document.statistics.lines,
            bytes: document.statistics.bytes,
        };
        let statistics: Arc<str> =
            format_character_count(document_statistics.characters, self.language).into();
        StatusLineSnapshot {
            pane,
            language: self.language,
            host: StatusHost::Reading,
            surface: crate::app::PaneSurface::Reading,
            dirty: self
                .document_session()
                .is_some_and(|session| session.read(cx).is_dirty()),
            reading_style: Some(self.reading_style),
            outline: current_outline(document, source_index),
            position: Some(StatusPosition::ReadingSource {
                line,
                total_lines: document.statistics.lines,
            }),
            progress: Some(progress),
            statistics: Some(statistics),
            document_statistics: Some(document_statistics),
            format: Some(DocumentFormat::from_path(&document.path)),
            transient: self.document_transient_status(Some(pane_side), cx),
        }
    }

    pub(crate) fn dired_status_snapshot(&self, pane: PaneId) -> Option<StatusLineSnapshot> {
        let session = self.file_manager.session()?;
        let total = session.entries().len();
        let selected = session
            .cursor()
            .and_then(|id| session.entries().iter().position(|entry| entry.id == id))
            .map_or(0, |index| index + 1);
        let directory = session
            .directory()
            .file_name()
            .unwrap_or_else(|| session.directory().as_os_str())
            .to_string_lossy()
            .into_owned();
        let marked = session.marked_count();
        Some(StatusLineSnapshot {
            pane,
            language: self.language,
            host: StatusHost::Dired,
            surface: crate::app::PaneSurface::Editor,
            dirty: false,
            reading_style: None,
            outline: Some(directory.into()),
            position: Some(StatusPosition::DiredSelection { selected, total }),
            progress: None,
            statistics: Some(
                match self.language {
                    Language::Chinese => format!("已标记 {marked}"),
                    Language::English => format!("{marked} marked"),
                }
                .into(),
            ),
            document_statistics: None,
            format: None,
            transient: self.file_manager.status().map(|status| StatusMessage {
                text: status.message(),
                tone: match status {
                    DiredStatus::Working(_) => StatusTone::Working,
                    DiredStatus::Success(_) => StatusTone::Success,
                    DiredStatus::Error(_) => StatusTone::Error,
                },
            }),
        })
    }

    fn document_transient_status(
        &self,
        reading_pane: Option<crate::app::PaneSide>,
        cx: &gpui::App,
    ) -> Option<StatusMessage> {
        let session = self.document_session().map(|session| session.read(cx));
        if let Some(session) = session {
            match session.sync_state() {
                crate::document::SyncState::Conflict { .. } => {
                    return Some(StatusMessage {
                        text: "Conflict: file changed on disk".into(),
                        tone: StatusTone::Error,
                    });
                }
                crate::document::SyncState::Missing { .. } => {
                    return Some(StatusMessage {
                        text: "File is missing on disk; Save will recreate it".into(),
                        tone: StatusTone::Error,
                    });
                }
                _ => {}
            }
            let pane_has_coherent_preview = reading_pane.is_some_and(|pane| {
                self.state.ready().is_some_and(|document| {
                    document.readers.get(pane).as_ref().is_some_and(|panel| {
                        let preview = panel.read(cx);
                        preview_matches_session(preview, session)
                    })
                })
            });
            if reading_pane.is_some()
                && !self.latest_preview_is_current(cx)
                && !pane_has_coherent_preview
            {
                return Some(StatusMessage {
                    text: "Updating Preview…".into(),
                    tone: StatusTone::Working,
                });
            }
        }
        if let Some(status) = self.export.status() {
            return Some(match status {
                ExportRunState::Working(message) => StatusMessage {
                    text: message.clone(),
                    tone: StatusTone::Working,
                },
                ExportRunState::Success { message, .. } => StatusMessage {
                    text: message.clone(),
                    tone: StatusTone::Success,
                },
                ExportRunState::Error(message) => StatusMessage {
                    text: message.clone(),
                    tone: StatusTone::Error,
                },
            });
        }
        match &self.save.status {
            Some(SaveStatus::Error(message)) => {
                return Some(StatusMessage {
                    text: message.clone(),
                    tone: StatusTone::Error,
                });
            }
            Some(SaveStatus::Saving) => {
                return Some(StatusMessage {
                    text: match self.language {
                        Language::Chinese => "正在保存…",
                        Language::English => "Saving…",
                    }
                    .into(),
                    tone: StatusTone::Working,
                });
            }
            Some(SaveStatus::Success {
                document, revision, ..
            }) if session.is_some_and(|session| {
                session.id() == *document && session.revision() == *revision && !session.is_dirty()
            }) =>
            {
                return Some(StatusMessage {
                    text: match self.language {
                        Language::Chinese => "保存成功",
                        Language::English => "Saved",
                    }
                    .into(),
                    tone: StatusTone::Success,
                });
            }
            _ => {}
        }
        None
    }
}

fn current_outline(document: &PreviewSnapshot, source_index: usize) -> Option<Arc<str>> {
    let block_id = document.projection.source_row(source_index)?.block_id as usize;
    document.outline_paths.get(block_id).cloned().flatten()
}

fn visible_item_end(list_state: &ListState, item_count: usize) -> usize {
    let viewport = list_state.viewport_bounds();
    if item_count == 0 || f32::from(viewport.size.height) <= 0.0 {
        return 0;
    }
    let start = list_state.logical_scroll_top().item_ix.min(item_count);
    let mut end = start;
    for index in start..item_count {
        match list_state.item_is_below_viewport(index) {
            Some(true) => break,
            Some(false) => end = index + 1,
            None => break,
        }
    }
    end.min(item_count)
}

fn visible_source_bottom_line(
    document: &PreviewSnapshot,
    visible_rows: &[usize],
    visible_end: usize,
) -> u64 {
    let Some(source_index) = visible_end
        .checked_sub(1)
        .and_then(|index| visible_rows.get(index))
        .copied()
    else {
        return 0;
    };
    let Some(row) = document.projection.source_row(source_index) else {
        return 0;
    };
    let byte = if row.content.range.end > row.content.range.start {
        row.content.range.end.0 - 1
    } else {
        row.content.range.start.0
    };
    document.text.line_of_byte(ByteOffset(byte)) + 1
}

fn list_reached_bottom(list_state: &ListState, visible_end: usize, item_count: usize) -> bool {
    if item_count == 0 || visible_end < item_count {
        return false;
    }
    let viewport = list_state.viewport_bounds();
    if f32::from(viewport.size.height) <= 0.0 {
        return false;
    }
    let Some(last_item) = list_state.bounds_for_item(item_count - 1) else {
        return false;
    };
    last_item.bottom() <= viewport.bottom() + px(0.5)
}

pub(crate) fn reading_progress(bottom_line: u64, total_lines: u64, reached_end: bool) -> u8 {
    if total_lines == 0 {
        return 0;
    }
    if reached_end {
        return 100;
    }
    (((bottom_line.min(total_lines) as f64 / total_lines as f64) * 100.0).round() as u8).min(99)
}

#[cfg(test)]
mod tests {
    use gpui::{
        Context, Element, IntoElement, ListAlignment, ListOffset, ListState, ParentElement, Render,
        Styled, Window, div, list, px,
    };

    use super::list_reached_bottom;

    #[test]
    fn an_unlaid_out_list_is_not_reported_as_the_document_bottom() {
        let list = ListState::new(100, ListAlignment::Top, px(80.0));
        assert!(!list_reached_bottom(&list, 0, 100));
    }

    #[gpui::test]
    fn bottom_requires_the_last_items_real_edge(cx: &mut gpui::TestAppContext) {
        let state = ListState::new(3, ListAlignment::Top, px(0.0)).measure_all();
        struct Rows(ListState);
        impl Render for Rows {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(100.0)).h(px(100.0)).child(
                    list(self.0.clone(), |_, _, _| {
                        div().h(px(60.0)).w_full().into_any()
                    })
                    .size_full(),
                )
            }
        }
        let (_rows, _cx) = cx.add_window_view(|_, _| Rows(state.clone()));

        // Even a transiently over-eager visible range cannot report 100% while the actual last
        // item still extends below the viewport.
        assert!(!list_reached_bottom(&state, 3, 3));
        state.scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(20.0),
        });
        assert!(list_reached_bottom(&state, 3, 3));
    }
}
