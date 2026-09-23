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
        if self.viewport.is_none_or(|v| !v.contains(&position)) {
            return None;
        }
        let row = self
            .hit_rows
            .iter()
            .find(|row| position.y >= row.origin_y && position.y < row.visible_bottom)?;
        if row.inline_image_preview
            || position.x < row.text_origin_x
            || position.x > row.text_origin_x + row.visual_width()
        {
            return None;
        }
        let local_y = position.y - row.origin_y;
        let visual_y =
            px((f32::from(local_y) / f32::from(row.line_height)).floor()
                * f32::from(row.line_height));
        self.link_hits.iter().position(|hit| {
            if hit.line != row.line {
                return false;
            }
            let Some(start) = row.position_for_display_index(hit.display_range.start) else {
                return false;
            };
            let Some(end) = row.position_for_display_index(hit.display_range.end) else {
                return false;
            };
            if visual_y < start.y || visual_y > end.y {
                return false;
            }
            let left = if visual_y == start.y { start.x } else { px(0.) };
            let right = if visual_y == end.y {
                end.x
            } else {
                row.visual_width()
            };
            // A link owns a continuous visual segment. Nearest-caret hit testing
            // leaves holes in the right half of wide glyphs and causes hover flicker.
            position.x >= row.text_origin_x + left && position.x < row.text_origin_x + right
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
                if let Some((path, anchor)) =
                    crate::links::resolve_file_link(self.session.read(cx).path(), &link.meta.raw)
                {
                    if crate::preview::is_supported_document(&path) {
                        cx.emit(EditorOpenDocumentEvent { path, anchor });
                    } else {
                        Self::open_with_system_handler(&path.to_string_lossy());
                    }
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

    fn internal_link_offset(&self, target: &str, cx: &Context<Self>) -> Option<ByteOffset> {
        internal_offset(&self.snapshot(cx), target)
    }
}

pub(super) fn internal_offset(snapshot: &DocumentSnapshot, target: &str) -> Option<ByteOffset> {
    let normalized = target.trim();
    let (mode, needle) = if let Some(custom) = normalized
        .strip_prefix('#')
        .or_else(|| normalized.strip_prefix("id:"))
    {
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
    if needle.is_empty() {
        return None;
    }
    for line in 0..snapshot.len_lines() {
        let Ok(range) = snapshot.line_content_range(LineIndex(line)) else {
            continue;
        };
        let text = snapshot.copy_range(range);
        let matched = match mode {
            InternalLinkMode::CustomId => {
                let trimmed = text.trim_start();
                (trimmed.starts_with(":CUSTOM_ID:")
                    && trimmed[":CUSTOM_ID:".len()..].trim().eq(needle))
                    || (trimmed.starts_with(":ID:") && trimmed[":ID:".len()..].trim().eq(needle))
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
            InternalLinkMode::Fuzzy => {
                text.contains(&format!("<<{needle}>>"))
                    || text
                        .trim()
                        .strip_prefix("#+NAME:")
                        .is_some_and(|name| name.trim() == needle)
                    || (!text.contains("[[") && text.contains(needle))
            }
        };
        if matched {
            return Some(crate::document::ByteOffset(range.start.0));
        }
    }
    None
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

/// A reading excerpt for an internal destination, without Org's property drawers.
pub(super) fn internal_preview(
    snapshot: &DocumentSnapshot,
    offset: ByteOffset,
) -> (Option<String>, Vec<String>) {
    let analysis =
        crate::org_semantic::analyze(snapshot, Arc::new(crate::org_syntax::parse(snapshot)));
    let heading = analysis
        .headings
        .iter()
        .filter(|h| h.content.start <= offset)
        .max_by_key(|h| h.content.start);
    let title = heading.map(|h| h.title.to_string());
    let start = heading.map_or(offset, |h| h.content.start);
    let limit = analysis
        .headings
        .iter()
        .filter(|h| h.content.start > start)
        .map(|h| h.content.start.0)
        .min()
        .unwrap_or(snapshot.len_bytes());
    let mut preview = Vec::new();
    let mut drawer = false;
    let Some(mut lines) =
        crate::document::LineCursor::within(snapshot, ByteRange::new(start.0, limit))
    else {
        return (title, preview);
    };
    let mut examined = 0;
    while let Some(line) = lines.next_line() {
        examined += 1;
        if examined > 128 {
            break;
        }
        let text = line.text.trim();
        if heading.is_some() && examined == 1 {
            continue;
        }
        if text.eq_ignore_ascii_case(":END:") {
            drawer = false;
            continue;
        }
        if text.len() > 2
            && text.starts_with(':')
            && text.ends_with(':')
            && text[1..text.len().saturating_sub(1)]
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_')
        {
            drawer = true;
            continue;
        }
        if drawer
            || text.is_empty()
            || text.starts_with("#+")
            || text.starts_with("# ")
            || ["SCHEDULED:", "DEADLINE:", "CLOSED:"]
                .iter()
                .any(|key| text.starts_with(key))
        {
            continue;
        }
        // Keep the excerpt near an explicit body target; property links start at their heading.
        if line.range.end < offset
            && !line.text.contains(":CUSTOM_ID:")
            && !line.text.contains(":ID:")
        {
            continue;
        }
        preview.push(text.chars().take(140).collect());
        if preview.len() == 2 {
            break;
        }
    }
    (title, preview)
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
    fn document_links_request_in_app_preview_with_their_anchor(cx: &mut gpui::TestAppContext) {
        let directory =
            std::env::temp_dir().join(format!("org-studio-link-events-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let session = cx.new(|_| {
            crate::document::DocumentSession::from_utf8(
                directory.join("main.org"),
                b"links\n".to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| SemanticEditor::new(session, cx));
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        editor.update(cx, |_, cx| {
            let events = events.clone();
            cx.subscribe_self(move |_, event: &EditorOpenDocumentEvent, _| {
                events.lock().unwrap().push(event.clone());
            })
            .detach();
        });
        for (filename, destination, anchor, format) in [
            (
                "notes.org",
                "file:notes.org::*Heading",
                "*Heading",
                crate::links::LinkFormat::Org,
            ),
            (
                "sample.rs",
                "file:sample.rs#main",
                "main",
                crate::links::LinkFormat::Org,
            ),
            (
                "notes.txt",
                "file:notes.txt#section",
                "section",
                crate::links::LinkFormat::Org,
            ),
            (
                "notes.md",
                "notes.md#section",
                "section",
                crate::links::LinkFormat::Markdown,
            ),
            (
                "中文 notes.markdown",
                "%E4%B8%AD%E6%96%87%20notes.markdown#section",
                "section",
                crate::links::LinkFormat::Markdown,
            ),
        ] {
            std::fs::write(directory.join(filename), "contents").unwrap();
            editor.update(cx, |editor, cx| {
                editor.open_link(
                    &LinkHit {
                        line: LineIndex(0),
                        display_range: 0..1,
                        meta: crate::links::classify(destination, format),
                    },
                    cx,
                )
            });
            let events = events.lock().unwrap();
            let event = events.last().unwrap();
            assert_eq!(event.path, directory.join(filename));
            assert_eq!(event.anchor.as_deref(), Some(anchor));
            std::fs::remove_file(directory.join(filename)).unwrap();
        }
        std::fs::remove_dir(directory).unwrap();
    }
}
