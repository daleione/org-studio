use gpui::{ClipboardItem, Context, Window};

use super::WorkspaceWindow;
use crate::document::{
    ByteOffset, ByteRange, DocumentCommand, DocumentSession, EditOrigin, EditTransaction,
    Selection, TextSnapshot,
};
use crate::preview::{CopyFeedbackState, PreviewAction, PreviewActionTarget, ReadingPreviewPanel};

impl WorkspaceWindow {
    pub(in crate::preview) fn dispatch_preview_action(
        &mut self,
        action: PreviewAction,
        panel: gpui::Entity<ReadingPreviewPanel>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !target_belongs_to_panel(panel.read(cx).document(), action.target()) {
            self.set_document_notice(Some("Preview action is no longer valid".into()));
            cx.notify();
            return;
        }
        match action {
            PreviewAction::ToggleCheckbox { target, expected } => {
                let Some(action_id) =
                    panel.update(cx, |panel, _| panel.begin_action(target.identity()))
                else {
                    return;
                };
                let result = self.toggle_reading_checkbox(&target, &expected, cx);
                panel.update(cx, |panel, _| match result {
                    Ok(revision) => panel.commit_action(action_id, revision),
                    Err(_) => panel.fail_action(action_id),
                });
                if let Err(message) = result {
                    self.set_document_notice(Some(message.into()));
                }
                cx.notify();
            }
            PreviewAction::OpenLink {
                target,
                destination,
            } => {
                if let Err(message) =
                    self.open_reading_link(&target, &destination, panel, _window, cx)
                {
                    self.set_document_notice(Some(message.into()));
                    cx.notify();
                }
            }
            PreviewAction::CopyCode(target) => {
                let result = self.copy_reading_code(&target, cx);
                panel.update(cx, |panel, cx| {
                    panel.show_copy_feedback(
                        target.source_range,
                        if result.is_ok() {
                            CopyFeedbackState::Succeeded
                        } else {
                            CopyFeedbackState::Failed
                        },
                        cx,
                    );
                });
                if let Err(message) = result {
                    self.set_document_notice(Some(message.into()));
                }
            }
            PreviewAction::OpenImage { target, path } => {
                if let Err(message) = self.open_reading_image(&target, &path, cx) {
                    self.set_document_notice(Some(message.into()));
                    cx.notify();
                }
            }
        }
    }

    fn toggle_reading_checkbox(
        &mut self,
        target: &PreviewActionTarget,
        expected: &str,
        cx: &mut Context<Self>,
    ) -> Result<crate::document::Revision, &'static str> {
        let Some(session) = self.document_session().cloned() else {
            return Err("Document is no longer open");
        };
        session.update(cx, |session, cx| {
            let (range, source) = validate_target(session, target)?;
            validate_checkbox_source(&source, expected)?;
            let revision = session.revision();
            let snapshot = session.snapshot();
            let Some(edits) = crate::org_syntax::command::checkbox_transaction(&snapshot, range)
            else {
                return Err("Checkbox is no longer present");
            };
            let before = Selection::caret(range.start);
            let after = Selection::caret(ByteOffset(range.end.0));
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(revision, edits),
                        before,
                        after,
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .map(|delta| delta.after)
                .map_err(|_| "Could not update the checkbox")
        })
    }

    fn open_reading_link(
        &mut self,
        target: &PreviewActionTarget,
        destination: &str,
        panel: gpui::Entity<ReadingPreviewPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let Some(session) = self.document_session().cloned() else {
            return Err("Document is no longer open");
        };
        let document_path = {
            let session = session.read(cx);
            validate_target(session, target)?;
            session.path().to_path_buf()
        };
        if destination.starts_with("http://")
            || destination.starts_with("https://")
            || destination.starts_with("mailto:")
        {
            cx.open_url(destination);
            return Ok(());
        }

        let destination = destination.strip_prefix("file:").unwrap_or(destination);
        let (path_or_anchor, search) = split_link_target(destination);
        if path_or_anchor.is_empty()
            || path_or_anchor.starts_with('#')
            || path_or_anchor.starts_with('*')
        {
            let anchor = search.unwrap_or(path_or_anchor);
            return panel
                .update(cx, |panel, _| panel.jump_to_destination(anchor))
                .then_some(())
                .ok_or("Link target was not found");
        }
        if search.is_none()
            && panel.update(cx, |panel, _| panel.jump_to_destination(path_or_anchor))
        {
            return Ok(());
        }

        let path = std::path::Path::new(path_or_anchor);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            document_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(path)
        };
        if !path.exists() {
            return Err("Linked file does not exist");
        }
        let opens_as_document = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("org")
                    || extension.eq_ignore_ascii_case("md")
                    || extension.eq_ignore_ascii_case("markdown")
            });
        if opens_as_document {
            if let Some(anchor) = search {
                self.request_open_at(path, anchor.into(), window, cx);
            } else {
                self.request_open(path, window, cx);
            }
        } else {
            cx.open_with_system(&path);
        }
        Ok(())
    }

    fn copy_reading_code(
        &self,
        target: &PreviewActionTarget,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let Some(session) = self.document_session() else {
            return Err("Document is no longer open");
        };
        let (_, source) = validate_target(session.read(cx), target)?;
        cx.write_to_clipboard(ClipboardItem::new_string(source));
        Ok(())
    }

    fn open_reading_image(
        &self,
        target: &PreviewActionTarget,
        image_path: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let Some(session) = self.document_session() else {
            return Err("Document is no longer open");
        };
        let document_path = {
            let session = session.read(cx);
            validate_target(session, target)?;
            session.path().to_path_buf()
        };
        if image_path.starts_with("http://") || image_path.starts_with("https://") {
            cx.open_url(image_path);
            return Ok(());
        }
        let image_path = image_path.strip_prefix("file:").unwrap_or(image_path);
        let path = std::path::Path::new(image_path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            document_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(path)
        };
        if !path.exists() {
            return Err("Image file does not exist");
        }
        cx.open_with_system(&path);
        Ok(())
    }
}

fn split_link_target(destination: &str) -> (&str, Option<&str>) {
    if let Some((path, search)) = destination.split_once("::") {
        return (path, Some(search));
    }
    if destination.starts_with('#') {
        return (destination, None);
    }
    destination
        .split_once('#')
        .map_or((destination, None), |(path, anchor)| (path, Some(anchor)))
}

fn target_belongs_to_panel(
    document: &crate::preview::PreviewSnapshot,
    target: &PreviewActionTarget,
) -> bool {
    if document.document_id != target.document_id || document.revision != target.base_revision {
        return false;
    }
    let Some(syntax_id) = target.syntax_id else {
        return true;
    };
    document
        .blocks
        .block_for_syntax_id(syntax_id)
        .and_then(|block_id| document.blocks.nodes().get(block_id as usize))
        .is_some_and(|block| {
            block.source.start <= target.source_range.start
                && block.source.end >= target.source_range.end
        })
}

fn validate_target(
    session: &DocumentSession,
    target: &PreviewActionTarget,
) -> Result<(ByteRange, String), &'static str> {
    if session.id() != target.document_id {
        return Err("The action belongs to another document");
    }
    let mapped = session
        .map_range_to_current(target.revision_range())
        .map_err(|_| "The source changed; action cancelled")?;
    let snapshot = session.snapshot();
    if mapped.range.end.0 > snapshot.len_bytes() {
        return Err("The action target is no longer valid");
    }
    let source = snapshot.copy_range(mapped.range);
    Ok((mapped.range, source))
}

fn validate_checkbox_source(source: &str, expected: &str) -> Result<(), &'static str> {
    if source != expected {
        return Err("Checkbox changed before the action completed");
    }
    if !crate::org_syntax::command::is_checkbox_state_token(source) {
        return Err("Checkbox target is ambiguous");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{split_link_target, validate_checkbox_source};

    #[test]
    fn link_target_preserves_cross_file_anchors() {
        assert_eq!(
            split_link_target("notes.org::*Heading"),
            ("notes.org", Some("*Heading"))
        );
        assert_eq!(
            split_link_target("notes.md#section"),
            ("notes.md", Some("section"))
        );
        assert_eq!(split_link_target("#section"), ("#section", None));
    }

    #[test]
    fn exact_checkbox_tokens_are_valid_action_targets() {
        for token in ["[ ]", "[-]", "[X]", "[x]"] {
            assert_eq!(validate_checkbox_source(token, token), Ok(()));
        }
        assert_eq!(
            validate_checkbox_source("literal [ ]", "literal [ ]"),
            Err("Checkbox target is ambiguous")
        );
        assert_eq!(
            validate_checkbox_source("[X]", "[ ]"),
            Err("Checkbox changed before the action completed")
        );
    }
}
