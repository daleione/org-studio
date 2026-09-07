use crate::app::WorkspaceWindow;
use gpui::{
    Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, FocusHandle,
    KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, ShapedLine, SharedString, TextAlign,
    TextRun, UTF16Selection, WeakEntity, Window, canvas, div, fill, point, prelude::*, px, rgb,
    rgba, size,
};
use std::ops::Range;

/// Single-line native input. Offsets in the model are UTF-8; platform IME offsets are UTF-16.
pub(crate) struct AgendaSearch {
    pub(crate) focus: FocusHandle,
    workspace: WeakEntity<WorkspaceWindow>,
    text: String,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    layout: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    scroll: Pixels,
}

impl AgendaSearch {
    pub(crate) fn new(workspace: WeakEntity<WorkspaceWindow>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            workspace,
            text: String::new(),
            selection: 0..0,
            marked: None,
            layout: None,
            bounds: None,
            scroll: px(0.),
        }
    }

    pub(crate) fn sync(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.text != text {
            self.text = text.to_owned();
            self.selection = text.len()..text.len();
            self.marked = None;
            cx.notify();
        }
    }

    fn utf8(text: &str, offset: usize) -> usize {
        let mut units = 0;
        for (byte, ch) in text.char_indices() {
            if units + ch.len_utf16() > offset {
                return byte;
            }
            units += ch.len_utf16();
        }
        text.len()
    }
    fn bytes(&self, range: Range<usize>) -> Range<usize> {
        Self::utf8(&self.text, range.start)..Self::utf8(&self.text, range.end)
    }
    fn utf16(&self, range: Range<usize>) -> Range<usize> {
        self.text[..range.start].encode_utf16().count()
            ..self.text[..range.end].encode_utf16().count()
    }
    fn replace(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let range = range
            .map(|r| self.bytes(r))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        let text = text.replace(['\r', '\n'], " ");
        self.text.replace_range(range.clone(), &text);
        let inserted = range.start..range.start + text.len();
        self.selection = inserted.end..inserted.end;
        self.marked = None;
        let workspace = self.workspace.clone();
        let value = self.text.clone();
        cx.defer(move |cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.dispatch_agenda_intent(super::UiIntent::SetSearch(value), cx);
            });
        });
        cx.notify();
        inserted
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if event.keystroke.modifiers.platform {
            match key {
                "a" => self.selection = 0..self.text.len(),
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.replace(None, &text, cx);
                    }
                }
                "c" | "x" => {
                    if !self.selection.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            self.text[self.selection.clone()].to_owned(),
                        ));
                        if key == "x" {
                            self.replace(None, "", cx);
                        }
                    }
                }
                _ => return,
            }
        } else if self.marked.is_none() {
            match key {
                "backspace" | "delete" => {
                    if self.selection.is_empty() {
                        if key == "backspace" {
                            self.selection.start = self.text[..self.selection.start]
                                .char_indices()
                                .next_back()
                                .map_or(0, |(i, _)| i);
                        } else {
                            self.selection.end += self.text[self.selection.end..]
                                .chars()
                                .next()
                                .map_or(0, char::len_utf8);
                        }
                    }
                    self.replace(None, "", cx);
                }
                "left" | "right" | "home" | "end" => {
                    let offset = match key {
                        "home" => 0,
                        "end" => self.text.len(),
                        "left" => {
                            if self.selection.is_empty() {
                                self.text[..self.selection.start]
                                    .char_indices()
                                    .next_back()
                                    .map_or(0, |(i, _)| i)
                            } else {
                                self.selection.start
                            }
                        }
                        _ => {
                            if self.selection.is_empty() {
                                self.selection.end
                                    + self.text[self.selection.end..]
                                        .chars()
                                        .next()
                                        .map_or(0, char::len_utf8)
                            } else {
                                self.selection.end
                            }
                        }
                    };
                    self.selection = offset..offset;
                }
                "escape" | "enter" => {
                    let _ = self.workspace.update(cx, |workspace, cx| {
                        workspace.focus_workspace_on_render = true;
                        cx.notify();
                    });
                    window.blur();
                }
                _ => return,
            }
        } else {
            return;
        }
        // Only consume editing commands; ordinary text must reach the platform IME handler.
        cx.stop_propagation();
        cx.notify();
    }
}

impl EntityInputHandler for AgendaSearch {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.bytes(range);
        *adjusted = Some(self.utf16(range.clone()));
        Some(self.text[range].into())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf16(self.selection.clone()),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|r| self.utf16(r))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(range, text, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let inserted = self.replace(range, text, cx);
        if let Some(selected) = selected {
            let text = &self.text[inserted.clone()];
            self.selection = inserted.start + Self::utf8(text, selected.start)
                ..inserted.start + Self::utf8(text, selected.end);
        }
        self.marked = (!inserted.is_empty()).then_some(inserted);
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let bounds = self.bounds?;
        let line = self.layout.as_ref()?;
        let range = self.bytes(range);
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(range.start) - self.scroll,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(range.end) - self.scroll,
                bounds.bottom(),
            ),
        ))
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let index = self
            .layout
            .as_ref()?
            .closest_index_for_x(point.x - self.bounds?.left() + self.scroll)
            .min(self.text.len());
        Some(self.utf16(index..index).start)
    }
}

impl Render for AgendaSearch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = cx.entity();
        div()
            .id("agenda-search-input")
            .w_full()
            .h(px(22.))
            .overflow_hidden()
            .track_focus(&self.focus)
            .cursor_text()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    let offset = this.layout.as_ref().zip(this.bounds).map_or(
                        this.text.len(),
                        |(line, bounds)| {
                            line.closest_index_for_x(event.position.x - bounds.left() + this.scroll)
                                .min(this.text.len())
                        },
                    );
                    this.selection = if event.click_count >= 2 {
                        0..this.text.len()
                    } else {
                        offset..offset
                    };
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        input.update(cx, |this, cx| {
                            let focused = this.focus.is_focused(window);
                            let text: SharedString = if this.text.is_empty() {
                                this.workspace
                                    .upgrade()
                                    .map(|workspace| workspace.read(cx).language)
                                    .unwrap_or_else(crate::i18n::Language::system)
                                    .text("agenda.search")
                                    .into()
                            } else {
                                this.text.clone().into()
                            };
                            let style = window.text_style();
                            let run = TextRun {
                                len: text.len(),
                                font: style.font(),
                                color: rgb(if this.text.is_empty() {
                                    0x858990
                                } else {
                                    0x34373d
                                })
                                .into(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let line = window.text_system().shape_line(text, px(12.), &[run], None);
                            let caret = line.x_for_index(this.selection.end);
                            this.scroll = if focused {
                                (caret - bounds.size.width + px(2.)).max(px(0.))
                            } else {
                                px(0.)
                            };
                            let origin = point(bounds.left() - this.scroll, bounds.top());
                            if focused && !this.selection.is_empty() {
                                window.paint_quad(fill(
                                    Bounds::from_corners(
                                        point(
                                            origin.x + line.x_for_index(this.selection.start),
                                            bounds.top(),
                                        ),
                                        point(origin.x + caret, bounds.bottom()),
                                    ),
                                    rgba(0x1688ff30),
                                ));
                            }
                            let _ = line.paint(
                                origin,
                                bounds.size.height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                            if focused {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(origin.x + caret, bounds.top() + px(3.)),
                                        size(px(1.), px(16.)),
                                    ),
                                    rgb(0x1688ff),
                                ));
                            }
                            if let Some(marked) = &this.marked {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(
                                            origin.x + line.x_for_index(marked.start),
                                            bounds.bottom() - px(2.),
                                        ),
                                        size(
                                            line.x_for_index(marked.end)
                                                - line.x_for_index(marked.start),
                                            px(1.),
                                        ),
                                    ),
                                    rgb(0x34373d),
                                ));
                            }
                            window.handle_input(
                                &this.focus,
                                ElementInputHandler::new(bounds, input.clone()),
                                cx,
                            );
                            this.bounds = Some(bounds);
                            this.layout = Some(line);
                        });
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn agenda_search_native_typing_ime_and_clear_filter_results(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, _| {
            let snapshot = crate::document::DocumentSnapshot::from_utf8(
                "* TODO 中文任务\n* TODO qckj\n".as_bytes().to_vec(),
            )
            .unwrap();
            let analysis = crate::org_semantic::analyze(
                &snapshot,
                std::sync::Arc::new(crate::org_syntax::parse(&snapshot)),
            );
            workspace
                .agenda
                .index
                .replace(crate::agenda::shard_from_live(
                    crate::agenda::FileId(99),
                    1,
                    std::sync::Arc::new("/tmp/search-test.org".into()),
                    &analysis,
                ));
            workspace.agenda.state.builtin = crate::agenda::BuiltinQuery::Unscheduled;
            workspace.agenda.requery();
        });
        let (input, cx) = cx.add_window_view(|_, cx| AgendaSearch::new(workspace.downgrade(), cx));
        cx.update(|window, app| {
            let focus = input.read(app).focus.clone();
            window.focus(&focus, app);
        });
        cx.simulate_keystrokes("q c k j");
        cx.run_until_parked();
        input.update(cx, |input, _| assert_eq!(input.text, "qckj"));
        workspace.update(cx, |workspace, _| {
            assert_eq!(workspace.agenda.result.as_ref().unwrap().rows.len(), 1)
        });
        cx.simulate_keystrokes("cmd-a backspace");
        cx.run_until_parked();
        workspace.update(cx, |workspace, _| {
            assert_eq!(workspace.agenda.result.as_ref().unwrap().rows.len(), 2)
        });
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.replace_and_mark_text_in_range(None, "中", Some(1..1), window, cx);
                input.replace_text_in_range(None, "中文", window, cx);
                assert_eq!(input.selection, 6..6);
                assert!(input.marked.is_none());
            })
        });
        cx.run_until_parked();
        workspace.update(cx, |workspace, _| {
            assert_eq!(&*workspace.agenda.state.search, "中文");
            assert_eq!(workspace.agenda.result.as_ref().unwrap().rows.len(), 1);
        });
    }
}
