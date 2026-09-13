//! Link activation for the editor: hit-testing, `org-open-at-point`
//! equivalents, and the open behaviors (web / mail / file / internal jump).

use super::*;
use crate::document::{ByteOffset, LineIndex};

/// Which in-document construct an internal link targets (Org Manual 4.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InternalLinkMode {
    /// `[[#custom-id]]` → `:CUSTOM_ID:` / `:ID:` property.
    CustomId,
    /// `[[*headline]]` → headline by name.
    Headline,
    /// `[[<<target>>]]` / `[[target]]` → dedicated `<<target>>`.
    Dedicated,
    /// Any other bare text → fuzzy search.
    Fuzzy,
}

impl SemanticEditor {
    /// `C-c C-o` / `org-open-at-point` equivalent: opens the link under the caret.
    pub(crate) fn open_link_at_caret(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self
            .link_at_caret(cx)
            .and_then(|index| self.link_hits.get(index).cloned())
        else {
            return;
        };
        self.open_link(&link, cx);
    }

    /// Index into `self.link_hits` for the link under the pointer, if any.
    pub(super) fn link_at_position(&self, position: Point<Pixels>) -> Option<usize> {
        let row = self.hit_rows.iter().min_by(|left, right| {
            vertical_distance(position.y, left).total_cmp(&vertical_distance(position.y, right))
        })?;
        self.link_hits.iter().position(|hit| {
            if hit.line != row.line {
                return false;
            }
            let start = row
                .position_for_display_index(hit.display_range.start)
                .unwrap_or_default();
            let end = row
                .position_for_display_index(hit.display_range.end.max(hit.display_range.start + 1))
                .unwrap_or(start);
            let left = start.x.min(end.x) - px(4.0);
            let right = start.x.max(end.x) + px(4.0);
            let top = row.origin_y - px(4.0);
            let bottom = row.origin_y + row.line_height + px(4.0);
            position.x >= left && position.x <= right && position.y >= top && position.y <= bottom
        })
    }

    /// Index into `self.link_hits` for the link under the caret, if any.
    fn link_at_caret(&self, cx: &Context<Self>) -> Option<usize> {
        let snapshot = self.snapshot(cx);
        let head = self.selection.head();
        let line = snapshot.line_index_at(head).ok()?;
        let row = self.hit_rows.iter().find(|row| row.line == line)?;
        let source_local = head.0.checked_sub(row.range.start.0)?;
        let display = row.display.source_to_display(source_local as usize);
        self.link_hits.iter().position(|hit| {
            hit.line == line
                && hit.display_range.start <= display
                && display < hit.display_range.end
        })
    }

    pub(super) fn open_link(&mut self, link: &LinkHit, cx: &mut Context<Self>) {
        use crate::links::LinkKind;
        match &link.meta.kind {
            LinkKind::External | LinkKind::Mail => {
                let mut target = link.meta.raw.to_string();
                if target.starts_with("www.") {
                    target = format!("https://{target}");
                }
                Self::open_with_system_handler(&target);
            }
            LinkKind::File => {
                if let Some(path) = self.resolve_file_link(&link.meta.raw, cx) {
                    Self::open_with_system_handler(&path.to_string_lossy());
                }
            }
            LinkKind::Internal => {
                if let Some(offset) = self.internal_link_offset(&link.meta.raw, cx) {
                    self.set_selection(Selection::caret(offset), cx);
                }
            }
            LinkKind::Dangerous | LinkKind::Other => {}
        }
    }

    /// Opens `target` with the platform handler. This crate targets macOS, so
    /// `open` is the system mechanism for URLs, mail, and files alike.
    fn open_with_system_handler(target: &str) {
        if let Err(error) = std::process::Command::new("open").arg(target).spawn() {
            tracing::warn!(?error, target, "failed to open link");
        }
    }

    fn resolve_file_link(&self, raw: &str, cx: &Context<Self>) -> Option<std::path::PathBuf> {
        let target = raw
            .strip_prefix("file:")
            .or_else(|| raw.strip_prefix("attachment:"))?;
        let target = target.split("::").next().unwrap_or(target);
        let path = std::path::Path::new(target);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        if let Some(home_rest) = target.strip_prefix("~/")
            && let Ok(home) = std::env::var("HOME")
        {
            return Some(std::path::Path::new(&home).join(home_rest));
        }
        let base = self.session.read(cx).path().parent()?;
        Some(base.join(path))
    }

    fn internal_link_offset(&self, target: &str, cx: &Context<Self>) -> Option<ByteOffset> {
        let snapshot = self.snapshot(cx);
        let normalized = target.trim();
        let (mode, needle) = if let Some(custom) = normalized.strip_prefix('#') {
            (InternalLinkMode::CustomId, custom)
        } else if let Some(title) = normalized.strip_prefix('*') {
            (InternalLinkMode::Headline, title)
        } else if let Some(dedicated) = normalized
            .strip_prefix("<<")
            .and_then(|tail| tail.strip_suffix(">>"))
        {
            (InternalLinkMode::Dedicated, dedicated)
        } else {
            (InternalLinkMode::Fuzzy, normalized)
        };
        for line in 0..snapshot.len_lines() {
            let Ok(range) = snapshot.line_content_range(LineIndex(line)) else {
                continue;
            };
            let text = snapshot.copy_range(range);
            let matched = match mode {
                InternalLinkMode::CustomId => {
                    let trimmed = text.trim_start();
                    (trimmed.starts_with(":CUSTOM_ID:")
                        && trimmed[":CUSTOM_ID:".len()..]
                            .trim_start()
                            .starts_with(needle))
                        || (trimmed.starts_with(":ID:")
                            && trimmed[":ID:".len()..].trim_start().starts_with(needle))
                }
                InternalLinkMode::Headline => {
                    let trimmed = text.trim_start();
                    trimmed.starts_with('*')
                        && trimmed
                            .trim_start_matches('*')
                            .trim_start()
                            .contains(needle)
                }
                InternalLinkMode::Dedicated => text.contains(&format!("<<{needle}>>")),
                InternalLinkMode::Fuzzy => text.contains(needle),
            };
            if matched {
                return Some(crate::document::ByteOffset(range.start.0));
            }
        }
        None
    }
}

/// Distance from `y` to the nearest edge of `row`; zero when inside the row.
pub(crate) fn vertical_distance(y: Pixels, row: &super::HitRow) -> f32 {
    let top = row.visible_top;
    let bottom = row.visible_bottom;
    if y < top {
        f32::from(top - y)
    } else if y > bottom {
        f32::from(y - bottom)
    } else {
        0.0
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[gpui::test]
    fn internal_links_resolve_to_their_document_positions(cx: &mut gpui::TestAppContext) {
        let source = b"* Top\n:PROPERTIES:\n:CUSTOM_ID: plan\n:END:\n\n** Section <<target>>\n* Heading Two\n";
        let session = cx.new(|_| {
            crate::document::DocumentSession::from_utf8(
                std::path::Path::new("t.org").to_path_buf(),
                source.to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));
        editor.update(cx, |editor, cx| {
            let offset = |target: &str| editor.internal_link_offset(target, cx);
            assert_eq!(
                offset("#plan").unwrap().0,
                19,
                "#custom-id targets :CUSTOM_ID:"
            );
            assert_eq!(offset("<<target>>").unwrap().0, 43, "dedicated target");
            assert_eq!(offset("*Heading Two").unwrap().0, 65, "headline by name");
            assert!(offset("Section").is_some(), "fuzzy text search");
            assert!(offset("missing").is_none());
        });
    }

    #[gpui::test]
    fn file_links_resolve_relative_to_the_document_directory(cx: &mut gpui::TestAppContext) {
        let session = cx.new(|_| {
            crate::document::DocumentSession::from_utf8(
                std::path::Path::new("/tmp/org-studio-demo/main.org").to_path_buf(),
                b"[[file:notes.org]]".to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));
        editor.update(cx, |editor, cx| {
            let resolved = editor
                .resolve_file_link("file:notes.org::12", cx)
                .expect("resolves");
            assert_eq!(
                resolved,
                std::path::PathBuf::from("/tmp/org-studio-demo/notes.org")
            );
            let absolute = editor
                .resolve_file_link("file:/etc/hosts", cx)
                .expect("absolute path");
            assert_eq!(absolute, std::path::PathBuf::from("/etc/hosts"));
            assert!(editor.resolve_file_link("irc:chan", cx).is_none());
        });
    }
}
