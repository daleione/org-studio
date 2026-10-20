use std::sync::Arc;

use gpui::AppContext;

use super::command_window;

gpui::actions!(workspace, [LocateCursor]);

const CURSOR_CONTRACT: f32 = 0.2;
const CURSOR_HOLD: f32 = 0.5;
const CURSOR_FADE: f32 = 0.2;
const CURSOR_DURATION: f32 = CURSOR_CONTRACT + CURSOR_HOLD + CURSOR_FADE;

pub(super) struct CursorSpotlight {
    editor: gpui::Entity<crate::editor::SemanticEditor>,
    started: std::time::Instant,
    pub(super) document: gpui::Entity<SpotlightDocument>,
    _keystrokes: gpui::Subscription,
    _fade_timer: gpui::Task<()>,
}

// Cache the entire stage, including preview recording and input handlers. Caching
// individual editors inside DocumentStage would record incomplete overview frames.
pub(super) struct SpotlightDocument {
    workspace: gpui::WeakEntity<super::WorkspaceWindow>,
    _changes: gpui::Subscription,
}

impl gpui::Render for SpotlightDocument {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{IntoElement, Styled};
        let Some(workspace) = self.workspace.upgrade() else {
            return gpui::div().size_full().into_any_element();
        };
        let state = workspace.read(cx);
        state
            .workspace_body(
                workspace.clone(),
                f32::from(window.viewport_size().width),
                state.minimap_reveal_at(std::time::Instant::now()).0,
                window,
                cx,
                if window.is_fullscreen() {
                    0.
                } else {
                    super::TITLEBAR_LEADING_INSET
                },
            )
            .into_any_element()
    }
}

impl super::WorkspaceWindow {
    pub(super) fn locate_cursor(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        use gpui::Focusable;
        if self.content_route != super::ContentRoute::Document || self.file_overview_is_open() {
            return;
        }
        let Some(editor) = self.editor(self.document_workspace.active_pane) else {
            return;
        };
        if !editor.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        editor.update(cx, |editor, cx| editor.reveal_cursor(cx));
        let workspace = cx.entity();
        let document = cx.new(|cx| SpotlightDocument {
            workspace: workspace.downgrade(),
            _changes: cx.observe(&workspace, |_, _, cx| cx.notify()),
        });
        let workspace = workspace.downgrade();
        let target_window = window.window_handle();
        // Bound editor actions run before raw key listeners. Intercept here so
        // arrows, deletion and shortcuts dismiss the mask without eating the key.
        let keystrokes = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() == target_window {
                let _ = workspace.update(cx, |workspace, cx| {
                    if workspace.dismiss_cursor_spotlight(cx) && event.keystroke.key == "escape" {
                        cx.stop_propagation();
                    }
                });
            }
        });
        self.cursor_spotlight = Some(cx.new(|cx| {
            let started = std::time::Instant::now();
            let delay = cx
                .background_executor()
                .timer(std::time::Duration::from_secs_f32(
                    CURSOR_CONTRACT + CURSOR_HOLD,
                ));
            CursorSpotlight {
                editor,
                started,
                document,
                _keystrokes: keystrokes,
                _fade_timer: cx.spawn(async move |this, cx| {
                    delay.await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                }),
            }
        }));
        cx.notify();
    }

    pub(super) fn dismiss_cursor_spotlight(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let dismissed = self.cursor_spotlight.take().is_some();
        if dismissed {
            cx.notify();
        }
        dismissed
    }

    pub(super) fn cursor_spotlight_tick(&mut self, window: &gpui::Window, cx: &gpui::App) {
        use gpui::Focusable;
        if self.cursor_spotlight.as_ref().is_some_and(|spotlight| {
            let spotlight = spotlight.read(cx);
            spotlight.started.elapsed().as_secs_f32() >= CURSOR_DURATION
                || !spotlight
                    .editor
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
                || self.content_route != super::ContentRoute::Document
                || self.file_overview_is_open()
        }) {
            self.cursor_spotlight = None;
        }
    }
}

impl gpui::Render for CursorSpotlight {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{Styled, canvas, point, px, rgb};
        let elapsed = self.started.elapsed().as_secs_f32();
        // Hold a static frame; the timer wakes us when fade-out begins.
        if elapsed < CURSOR_CONTRACT || elapsed >= CURSOR_CONTRACT + CURSOR_HOLD {
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
        let editor = self.editor.clone();
        let started = self.started;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let Some(mut caret) = editor.read(cx).painted_caret() else {
                    return;
                };
                let center = caret.bounds.center();
                if !bounds.contains(&center) {
                    return;
                }
                let elapsed = started.elapsed().as_secs_f32();
                let smooth = |t: f32| {
                    let t = t.clamp(0., 1.);
                    t * t * (3. - 2. * t)
                };
                let contraction = 1. - smooth(elapsed / CURSOR_CONTRACT);
                let opacity =
                    smooth(elapsed / 0.06) * smooth((CURSOR_DURATION - elapsed) / CURSOR_FADE);
                let dx = (center.x - bounds.left()).max(bounds.right() - center.x);
                let dy = (center.y - bounds.top()).max(bounds.bottom() - center.y);
                let cover = f32::from(dx).hypot(f32::from(dy)).max(56.);
                let radius = px(56. + (cover - 56.) * contraction);
                let outer = px(cover + 2.);
                window.paint_quad(gpui::quad(
                    gpui::Bounds::new(
                        center - point(outer, outer),
                        gpui::size(outer * 2., outer * 2.),
                    ),
                    outer,
                    gpui::rgba(0),
                    outer - radius,
                    rgb(0x999999).opacity(0.7 * opacity),
                    gpui::BorderStyle::Solid,
                ));
                // Reuse the final editor quad, including image-edge caret geometry.
                caret.background = caret.background.opacity(opacity);
                window.paint_quad(caret);
            },
        )
        .absolute()
        .inset_0()
    }
}

pub(super) fn dired_help_window(
    items: Arc<Vec<(Arc<str>, Arc<str>)>>,
    available_width: f32,
) -> gpui::Div {
    use command_window::{CommandGroup, CommandWindow};

    let take = |title: &str, keys: &[&str], columns| CommandGroup {
        title: Arc::from(title),
        max_columns: columns,
        items: keys
            .iter()
            .filter_map(|key| {
                items
                    .iter()
                    .find(|(candidate, _)| candidate.as_ref() == *key)
                    .cloned()
            })
            .collect(),
    };
    CommandWindow {
        title: Arc::from("Dired Commands"),
        close: Some((Arc::from("C-g"), Arc::from("Close"))),
        groups: vec![
            take(
                "NAVIGATION",
                &["n / j", "p / k", "^ / h", "H", "L", "g", "q"],
                3,
            ),
            take("MARKS", &["m", "u", "U", "t", "d"], 3),
            take("FILES", &["RET / l", "x"], 2),
            take("GLOBAL", &["C-x C-b", "C-x d", "C-x C-d"], 3),
        ],
    }
    .render(available_width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceWindow;
    use crate::document::TextSnapshot;

    #[gpui::test]
    fn spotlight_reuses_document_and_preserves_dismissal_keystrokes(cx: &mut gpui::TestAppContext) {
        cx.update(crate::editor::init);
        cx.update(|cx| {
            cx.bind_keys([gpui::KeyBinding::new(
                "cmd-/",
                LocateCursor,
                Some("SemanticEditor"),
            )])
        });
        let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, cx| {
            workspace.create_buffer("test.org".into(), None, cx)
        });
        cx.run_until_parked();
        cx.simulate_input("abc");
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-/");
        cx.run_until_parked();
        // Let cursor reveal and the initial document handoff finish first.
        for _ in 0..3 {
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
            cx.run_until_parked();
        }
        let (spotlight, frame) = workspace.read_with(cx, |workspace, _| {
            (
                workspace
                    .cursor_spotlight
                    .clone()
                    .expect("shortcut starts spotlight"),
                workspace.file_overview.drawn_frame.latest().unwrap(),
            )
        });
        spotlight.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            assert!(
                Arc::ptr_eq(
                    &frame,
                    &workspace.file_overview.drawn_frame.latest().unwrap()
                ),
                "animation must not re-record the document"
            );
        });

        drop(spotlight);
        for (key, offset, text) in [
            ("left", 2, "abc"),
            ("backspace", 1, "ac"),
            ("escape", 1, "ac"),
        ] {
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
            workspace.read_with(cx, |workspace, cx| {
                assert!(
                    workspace.cursor_spotlight.is_none(),
                    "{key} dismisses spotlight"
                );
                let editor = workspace
                    .editor(workspace.document_workspace.active_pane)
                    .unwrap();
                assert_eq!(editor.read(cx).selection().head().0, offset);
                let snapshot = editor.read(cx).snapshot(cx);
                assert_eq!(
                    snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes())),
                    text
                );
            });
            cx.simulate_keystrokes("cmd-/");
            cx.run_until_parked();
            assert!(workspace.read_with(cx, |workspace, _| workspace.cursor_spotlight.is_some()));
        }
        cx.simulate_keystrokes("escape");
    }
}
