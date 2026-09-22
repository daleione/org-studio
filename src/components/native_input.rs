use gpui::{
    Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, FocusHandle,
    KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, ShapedLine, SharedString, TextAlign,
    TextRun, UTF16Selection, Window, canvas, div, fill, point, prelude::*, px, rgb, rgba, size,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Single-line native input. Offsets in the model are UTF-8; platform IME offsets are UTF-16.
pub(crate) struct NativeInput {
    pub(crate) config: InputConfig,
    pub(crate) append_only: bool,
    pub(crate) invalid: bool,
    pub(crate) enabled: bool,
    pub(crate) content_width: f32,
    undo: Vec<String>,
    redo: Vec<String>,
    composition_just_committed: bool,
    composition_origin: Option<String>,
    pub(crate) focus: FocusHandle,
    pub(crate) text: String,
    pub(crate) selection: Range<usize>,
    selection_reversed: bool,
    pub(crate) marked: Option<Range<usize>>,
    layout: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    scroll: Pixels,
}

#[derive(Clone)]
pub(crate) struct InputConfig {
    pub(crate) id: &'static str,
    pub(crate) placeholder: SharedString,
    pub(crate) outlined: bool,
    pub(crate) preserve_newlines: bool,
    pub(crate) font_size: f32,
    pub(crate) command: fn(&str, gpui::Modifiers, bool) -> bool,
}
impl Default for InputConfig {
    fn default() -> Self {
        Self {
            id: "native-input",
            placeholder: "".into(),
            outlined: false,
            preserve_newlines: false,
            font_size: 12.,
            command: |key, _, _| matches!(key, "enter" | "escape"),
        }
    }
}
pub(crate) enum InputEvent {
    Changed(String),
    Command {
        key: String,
        modifiers: gpui::Modifiers,
    },
    MetricsChanged,
}
impl gpui::EventEmitter<InputEvent> for NativeInput {}

impl NativeInput {
    #[cfg(test)]
    pub(crate) fn painted_bounds(&self) -> Option<Bounds<Pixels>> {
        self.bounds
    }

    pub(crate) fn new(config: InputConfig, cx: &mut Context<Self>) -> Self {
        Self {
            config,
            append_only: false,
            invalid: false,
            enabled: true,
            content_width: 0.,
            undo: Vec::new(),
            redo: Vec::new(),
            composition_just_committed: false,
            composition_origin: None,
            focus: cx.focus_handle(),
            text: String::new(),
            selection: 0..0,
            selection_reversed: false,
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
            self.selection_reversed = false;
            self.marked = None;
            self.composition_origin = None;
            cx.notify();
        }
    }

    pub(crate) fn is_composing(&self) -> bool {
        self.marked.is_some() || self.composition_just_committed
    }
    pub(crate) fn display_text(&self) -> SharedString {
        if self.text.is_empty() {
            self.config.placeholder.clone()
        } else {
            self.text.replace('\n', "↵").replace('\r', "␍").into()
        }
    }
    fn publish(&self, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Changed(self.text.clone()));
    }
    fn display_offset(&self, offset: usize) -> usize {
        offset
            + self.text[..offset]
                .bytes()
                .filter(|b| matches!(b, b'\n' | b'\r'))
                .count()
                * 2
    }
    fn source_offset(&self, display: usize) -> usize {
        let mut at = 0;
        for (offset, ch) in self.text.char_indices() {
            let len = if matches!(ch, '\n' | '\r') {
                3
            } else {
                ch.len_utf8()
            };
            if at + len > display {
                return offset;
            }
            at += len;
        }
        self.text.len()
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
        let text = if !self.config.preserve_newlines {
            text.replace(['\r', '\n'], " ")
        } else {
            text.to_owned()
        };
        self.text.replace_range(range.clone(), &text);
        let inserted = range.start..range.start + text.len();
        self.selection = inserted.end..inserted.end;
        self.selection_reversed = false;
        self.marked = None;
        cx.notify();
        inserted
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        let key = event.keystroke.key.as_str();
        if self.composition_just_committed {
            cx.stop_propagation();
            return;
        }
        if self.marked.is_some() {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if (self.config.command)(key, modifiers, self.append_only) {
            cx.emit(InputEvent::Command {
                key: key.to_owned(),
                modifiers,
            });
            cx.stop_propagation();
            return;
        }
        if key == "tab" {
            if modifiers.shift {
                window.focus_prev(cx)
            } else {
                window.focus_next(cx)
            };
            cx.stop_propagation();
            return;
        }
        if self.append_only && matches!(key, "left" | "right" | "home" | "end" | "delete") {
            cx.stop_propagation();
            return;
        }
        if modifiers.platform && key == "z" {
            let value = if modifiers.shift {
                self.redo.pop()
            } else {
                self.undo.pop()
            };
            if let Some(value) = value {
                if modifiers.shift {
                    self.undo.push(self.text.clone());
                } else {
                    self.redo.push(self.text.clone());
                }
                self.sync(&value, cx);
                self.publish(cx);
            }
            cx.stop_propagation();
            return;
        }
        let previous = self.text.clone();
        if event.keystroke.modifiers.platform {
            match key {
                "a" if !self.append_only => self.selection = 0..self.text.len(),
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
                                .grapheme_indices(true)
                                .next_back()
                                .map_or(0, |(i, _)| i);
                        } else {
                            self.selection.end += self.text[self.selection.end..]
                                .graphemes(true)
                                .next()
                                .map_or(0, str::len);
                        }
                    }
                    self.replace(None, "", cx);
                }
                "left" | "right" | "home" | "end" => {
                    let head = if self.selection_reversed {
                        self.selection.start
                    } else {
                        self.selection.end
                    };
                    let anchor = if self.selection_reversed {
                        self.selection.end
                    } else {
                        self.selection.start
                    };
                    let extend = event.keystroke.modifiers.shift;
                    let offset = match key {
                        "home" => 0,
                        "end" => self.text.len(),
                        "left" => {
                            if self.selection.is_empty() || extend {
                                self.text[..head]
                                    .grapheme_indices(true)
                                    .next_back()
                                    .map_or(0, |(i, _)| i)
                            } else {
                                self.selection.start
                            }
                        }
                        _ => {
                            if self.selection.is_empty() || extend {
                                head + self.text[head..].graphemes(true).next().map_or(0, str::len)
                            } else {
                                self.selection.end
                            }
                        }
                    };
                    self.selection = if extend {
                        anchor.min(offset)..anchor.max(offset)
                    } else {
                        offset..offset
                    };
                    self.selection_reversed = extend && offset < anchor;
                }
                _ => return,
            }
        } else {
            return;
        }
        if previous != self.text {
            self.undo.push(previous);
            self.redo.clear();
            self.publish(cx);
        }
        // Only consume editing commands; ordinary text must reach the platform IME handler.
        cx.stop_propagation();
        cx.notify();
    }
}

impl EntityInputHandler for NativeInput {
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
            reversed: self.selection_reversed,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|r| self.utf16(r))
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            if let Some(origin) = self.composition_origin.take() {
                self.undo.push(origin);
            }
            self.composition_just_committed = true;
            cx.on_next_frame(window, |this, _, cx| {
                this.composition_just_committed = false;
                cx.notify();
            });
            self.publish(cx);
            cx.notify();
        }
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled {
            return;
        }
        if self.marked.is_some() {
            self.composition_just_committed = true;
            cx.on_next_frame(window, |this, _, cx| {
                this.composition_just_committed = false;
                cx.notify();
            });
        }
        self.undo.push(
            self.composition_origin
                .take()
                .unwrap_or_else(|| self.text.clone()),
        );
        self.redo.clear();
        self.replace(range, text, cx);
        self.publish(cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled {
            return;
        }
        if self.marked.is_none() {
            self.composition_origin = Some(self.text.clone());
        }
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
                bounds.left() + line.x_for_index(self.display_offset(range.start)) - self.scroll,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(self.display_offset(range.end)) - self.scroll,
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
            .closest_index_for_x(point.x - self.bounds?.left() + self.scroll);
        let index = self.source_offset(index);
        Some(self.utf16(index..index).start)
    }
}

impl Render for NativeInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::current_theme();
        let input = cx.entity();
        let id = self.config.id;
        input_frame(self.config.outlined, self.invalid)
            .id(id)
            .debug_selector(move || id.to_owned())
            .when(self.enabled, |input| input.track_focus(&self.focus))
            .focus(|style| {
                style.border_color(rgb(if self.invalid {
                    theme.error
                } else {
                    theme.accent
                }))
            })
            .cursor_text()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    if !this.enabled {
                        cx.stop_propagation();
                        return;
                    }
                    window.focus(&this.focus, cx);
                    let offset = this.layout.as_ref().zip(this.bounds).map_or(
                        this.text.len(),
                        |(line, bounds)| {
                            line.closest_index_for_x(event.position.x - bounds.left() + this.scroll)
                        },
                    );
                    let offset = this.source_offset(offset);
                    this.selection_reversed = false;
                    this.selection = if this.append_only {
                        this.text.len()..this.text.len()
                    } else if event.click_count >= 2 {
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
                            let focused = this.enabled && this.focus.is_focused(window);
                            let text = this.display_text();
                            let style = window.text_style();
                            let run = TextRun {
                                len: text.len(),
                                font: style.font(),
                                color: rgb(if this.config.outlined {
                                    if this.text.is_empty() {
                                        theme.foreground_muted
                                    } else {
                                        theme.foreground
                                    }
                                } else if this.text.is_empty() {
                                    crate::theme::current_theme().foreground_dim
                                } else {
                                    crate::theme::current_theme().foreground
                                })
                                .into(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let font_size = this.config.font_size;
                            let line =
                                window
                                    .text_system()
                                    .shape_line(text, px(font_size), &[run], None);
                            let content_width = f32::from(line.width);
                            if (this.content_width - content_width).abs() > 0.5 {
                                this.content_width = content_width;
                                cx.emit(InputEvent::MetricsChanged);
                            }
                            let caret = line.x_for_index(this.display_offset(this.selection.end));
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
                                            origin.x
                                                + line.x_for_index(
                                                    this.display_offset(this.selection.start),
                                                ),
                                            bounds.top(),
                                        ),
                                        point(origin.x + caret, bounds.bottom()),
                                    ),
                                    rgba((theme.accent << 8) | 0x30),
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
                                    rgb(theme.accent),
                                ));
                            }
                            if let Some(marked) = &this.marked {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(
                                            origin.x
                                                + line
                                                    .x_for_index(this.display_offset(marked.start)),
                                            bounds.bottom() - px(2.),
                                        ),
                                        size(
                                            line.x_for_index(this.display_offset(marked.end))
                                                - line
                                                    .x_for_index(this.display_offset(marked.start)),
                                            px(1.),
                                        ),
                                    ),
                                    rgb(theme.foreground),
                                ));
                            }
                            if this.enabled {
                                window.handle_input(
                                    &this.focus,
                                    ElementInputHandler::new(bounds, input.clone()),
                                    cx,
                                );
                            }
                            this.bounds = Some(bounds);
                            this.layout = Some(line);
                        });
                    },
                )
                .w_full()
                .h(px(22.)),
            )
    }
}

pub(crate) fn input_frame(outlined: bool, invalid: bool) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .w_full()
        .min_w_0()
        .h(px(if outlined { 30. } else { 22. }))
        .flex()
        .items_center()
        .overflow_hidden()
        .when(outlined, |input| {
            input
                .px(px(8.))
                .rounded(px(5.))
                .border_1()
                .border_color(rgb(if invalid { theme.error } else { theme.border }))
                .bg(rgb(theme.background))
        })
}
