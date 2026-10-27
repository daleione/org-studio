//! Link activation for the editor: hit-testing, `org-open-at-point`
//! equivalents, and the open behaviors (web / mail / file / internal jump).

use super::*;
use crate::document::{ByteOffset, LineIndex};

mod edit;
pub(super) use edit::LinkEditor;

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
    pub(crate) fn hovered_link_target(&self) -> Option<Arc<str>> {
        let (line, range) = self.hovered_link.as_ref()?;
        self.link_hits
            .iter()
            .find(|hit| hit.line == *line && hit.display_range == *range)
            .map(|hit| hit.meta.raw.clone())
    }

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
            let Some((start, end)) = row.positions_for_display_range(hit.display_range.clone())
            else {
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
                cx.open_url(&target);
            }
            LinkKind::File => {
                if let Some((path, anchor)) =
                    crate::links::resolve_file_link(self.session.read(cx).path(), &link.meta.raw)
                {
                    if crate::preview::is_supported_document(&path)
                        || crate::preview::is_supported_image(&path)
                    {
                        cx.emit(EditorOpenFileEvent { path, anchor });
                    } else {
                        cx.open_with_system(&path);
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

#[cfg(test)]
mod link_tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    fn link_position(
        editor: &Entity<SemanticEditor>,
        line: u64,
        display_index: usize,
        cx: &mut VisualTestContext,
    ) -> Point<Pixels> {
        cx.read(|cx| {
            let row = editor
                .read(cx)
                .hit_rows
                .iter()
                .find(|row| row.line.0 == line)
                .unwrap();
            let position = row.position_for_display_index(display_index).unwrap();
            gpui::point(
                row.text_origin_x + position.x + px(2.),
                row.origin_y + position.y + px(6.),
            )
        })
    }

    fn double_click(position: Point<Pixels>, cx: &mut VisualTestContext) {
        cx.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        cx.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn file_links_place_the_caret_on_single_click_and_open_on_double_click(
        cx: &mut TestAppContext,
    ) {
        cx.update(init);
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("notes.org");
        std::fs::write(&destination, "* Destination\n").unwrap();
        for (name, source, anchor) in [
            (
                "main.org",
                "[[file:notes.org::*Destination][打开文件]]\n",
                "*Destination",
            ),
            (
                "main.md",
                "[打开文件](notes.org#destination)\n",
                "destination",
            ),
        ] {
            let session = cx.new(|_| {
                DocumentSession::from_utf8(directory.path().join(name), source.as_bytes().to_vec())
                    .unwrap()
            });
            let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            editor.update(view, |_, cx| {
                let events = events.clone();
                cx.subscribe_self(move |_, event: &EditorOpenFileEvent, _| {
                    events.lock().unwrap().push(event.clone())
                })
                .detach();
            });
            view.run_until_parked();
            let position = link_position(&editor, 0, "打".len(), view);
            view.simulate_mouse_move(position, None, Modifiers::default());
            view.executor().advance_clock(Duration::from_millis(400));
            view.run_until_parked();
            view.read(|cx| {
                assert!(editor.read(cx).hovered_link.is_some());
                assert!(editor.read(cx).inline_actions.popup.is_none());
            });
            let caret = view.read(|cx| editor.read(cx).hit_test(position));
            for modifiers in [
                Modifiers::default(),
                Modifiers {
                    platform: true,
                    ..Default::default()
                },
                Modifiers {
                    control: true,
                    ..Default::default()
                },
            ] {
                view.simulate_click(position, modifiers);
                view.read(|cx| assert_eq!(editor.read(cx).selection(), Selection::caret(caret)));
                assert!(
                    events.lock().unwrap().is_empty(),
                    "single clicks must not open links"
                );
            }
            double_click(position, view);
            let events = events.lock().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].path, destination);
            assert_eq!(events[0].anchor.as_deref(), Some(anchor));
            view.read(|cx| {
                let snapshot = editor.read(cx).snapshot(cx);
                assert_eq!(
                    snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                    source
                );
            });
        }
    }

    #[gpui::test]
    fn double_click_opens_urls_and_jumps_to_internal_targets(cx: &mut TestAppContext) {
        cx.update(init);
        let source = "[[#target][跳转]]\n[[https://example.com][网站]]\n* Target\n:PROPERTIES:\n:CUSTOM_ID: target\n:END:\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8("main.org".into(), source.as_bytes().to_vec()).unwrap()
        });
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.run_until_parked();
        let position = link_position(&editor, 1, 0, view);
        view.simulate_click(position, Modifiers::default());
        assert!(view.opened_url().is_none());
        double_click(position, view);
        assert_eq!(view.opened_url().as_deref(), Some("https://example.com"));
        let position = link_position(&editor, 0, 0, view);
        double_click(position, view);
        view.read(|cx| {
            let editor = editor.read(cx);
            assert_eq!(
                editor.selection().head(),
                internal_offset(&editor.snapshot(cx), "#target").unwrap()
            );
        });
    }

    #[gpui::test]
    fn wrapped_link_hover_hits_are_continuous_without_a_popup(cx: &mut TestAppContext) {
        cx.update(init);
        let caption = "中文链接 This long caption wraps across several visual rows";
        let source = format!("[[https://example.com][{caption}]]\n");
        let session = cx.new(|_| {
            DocumentSession::from_utf8("main.org".into(), source.as_bytes().to_vec()).unwrap()
        });
        let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));
        view.simulate_resize(gpui::size(px(340.), px(500.)));
        view.run_until_parked();
        for (index, _) in caption.char_indices() {
            let position = link_position(&editor, 0, index, view);
            view.simulate_mouse_move(position, None, Modifiers::default());
            view.read(|cx| {
                assert!(
                    editor.read(cx).link_at_position(position).is_some(),
                    "hole at {position:?}"
                )
            });
        }
        view.executor().advance_clock(Duration::from_millis(400));
        view.run_until_parked();
        view.read(|cx| assert!(editor.read(cx).inline_actions.popup.is_none()));
    }

    #[gpui::test]
    fn editor_link_hover_updates_status_without_changing_source(cx: &mut TestAppContext) {
        cx.update(init);
        let directory = tempfile::tempdir().unwrap();
        for (name, source, expected) in [
            (
                "guide.org",
                "[[file:reference.org::*Overview][Reference]]\n",
                "file:reference.org::*Overview",
            ),
            (
                "guide.md",
                "[Reference](reference.md#overview \"A title\")\n",
                "reference.md#overview",
            ),
        ] {
            let path = directory.path().join(name);
            std::fs::write(&path, source).unwrap();
            let loaded = crate::preview::load_workspace_document(path, false).unwrap();
            let (workspace, view) =
                cx.add_window_view(|_, _| crate::app::WorkspaceWindow::with_split_layout(false));
            workspace.update(view, |workspace, cx| {
                workspace.generation = 1;
                assert!(workspace.apply_load_result(1, Ok(loaded), cx));
                cx.notify();
            });
            view.update(|window, cx| window.simulate_next_frame(cx));
            view.run_until_parked();
            let editor = view.read(|cx| {
                workspace
                    .read(cx)
                    .editor(crate::app::PaneSide::Left)
                    .unwrap()
            });
            let selection = view.read(|cx| editor.read(cx).selection);
            let position = link_position(&editor, 0, 2, view);
            view.simulate_mouse_move(position, None, Modifiers::default());
            view.run_until_parked();
            view.read(|cx| {
                let status = workspace.read(cx).status_snapshot(cx).unwrap();
                assert_eq!(status.link_target.as_deref(), Some(expected), "{name}");
                assert_eq!(editor.read(cx).selection, selection);
                let snapshot = editor.read(cx).snapshot(cx);
                assert_eq!(
                    snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                    source
                );
            });
            assert!(view.debug_bounds("status-1-link-target").is_some());
            view.simulate_mouse_move(gpui::point(px(2.0), px(2.0)), None, Modifiers::default());
            view.run_until_parked();
            assert!(view.debug_bounds("status-1-link-target").is_none());
        }
    }

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
            cx.subscribe_self(move |_, event: &EditorOpenFileEvent, _| {
                events.lock().unwrap().push(event.clone());
            })
            .detach();
        });
        for (filename, destination, anchor, format) in [
            (
                "notes.org",
                "file:notes.org::*Heading",
                Some("*Heading"),
                crate::links::LinkFormat::Org,
            ),
            (
                "sample.rs",
                "file:sample.rs#main",
                Some("main"),
                crate::links::LinkFormat::Org,
            ),
            (
                "notes.txt",
                "file:notes.txt#section",
                Some("section"),
                crate::links::LinkFormat::Org,
            ),
            (
                "notes.md",
                "notes.md#section",
                Some("section"),
                crate::links::LinkFormat::Markdown,
            ),
            (
                "中文 notes.markdown",
                "%E4%B8%AD%E6%96%87%20notes.markdown#section",
                Some("section"),
                crate::links::LinkFormat::Markdown,
            ),
            (
                "figure.svg",
                "file:figure.svg",
                None,
                crate::links::LinkFormat::Org,
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
            assert_eq!(event.anchor.as_deref(), anchor);
            std::fs::remove_file(directory.join(filename)).unwrap();
        }
        std::fs::remove_dir(directory).unwrap();
    }
}
