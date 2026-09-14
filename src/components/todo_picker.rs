//! Compact status quick bar. The host owns positioning and document edits.
use super::selection_style::{HOVER_BACKGROUND, SELECTED_BACKGROUND, SELECTED_HOVER_BACKGROUND};
use crate::{
    i18n::Language,
    org_semantic::{TodoState, TodoStateKind},
    theme::current_theme,
};
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, MouseButton, Render, Window,
    div, prelude::*, px, rgb,
};
use std::{collections::HashSet, sync::Arc};

pub(crate) const BAR_HEIGHT: f32 = 36.;
pub(crate) const MORE_HEIGHT: f32 = 78.;
gpui::actions!(todo_picker, [CancelTodo]);
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("escape", CancelTodo, Some("TodoPicker")),
        gpui::KeyBinding::new("ctrl-g", CancelTodo, Some("TodoPicker")),
    ]);
}
pub(crate) enum TodoPickerEvent {
    Selected(Option<Arc<str>>),
    Customize,
    Cancelled,
}
impl EventEmitter<TodoPickerEvent> for TodoPicker {}

pub(crate) struct TodoPicker {
    focus: FocusHandle,
    language: Language,
    states: Vec<TodoState>,
    current: Arc<str>,
    selected: usize,
    keyboard_navigation: bool,
    pub(crate) more_open: bool,
    more_selected: usize,
    pub(crate) width: f32,
    pub(crate) more_above: bool,
    scroll: gpui::ScrollHandle,
}
impl Focusable for TodoPicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl TodoPicker {
    pub(crate) fn new(
        states: Vec<TodoState>,
        current: Arc<str>,
        language: Language,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut seen = HashSet::new();
        let states: Vec<_> = states
            .into_iter()
            .filter(|s| seen.insert(s.keyword.clone()))
            .collect();
        let selected = states
            .iter()
            .position(|s| s.keyword == current)
            .unwrap_or(0);
        let scroll = gpui::ScrollHandle::new();
        scroll.scroll_to_item(selected);
        Self {
            focus: cx.focus_handle(),
            language,
            states,
            current,
            selected,
            keyboard_navigation: false,
            more_open: false,
            more_selected: 0,
            width: 500.,
            more_above: false,
            scroll,
        }
    }
    pub(crate) fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language;
            cx.notify();
        }
    }
    fn fast_key(&self, state: &TodoState) -> Option<char> {
        state.fast_key.filter(|key| {
            self.states
                .iter()
                .filter(|s| s.fast_key == Some(*key))
                .count()
                == 1
        })
    }
    fn button_width(&self, state: &TodoState, window: &Window) -> f32 {
        let text: gpui::SharedString = state.keyword.to_string().into();
        let mut font = gpui::font("Menlo");
        font.weight = gpui::FontWeight::MEDIUM;
        let run = gpui::TextRun {
            len: text.len(),
            font,
            ..Default::default()
        };
        let width = f32::from(
            window
                .text_system()
                .shape_line(text, px(12.), &[run], None)
                .width,
        );
        (width
            + 16.
            + if self.fast_key(state).is_some() {
                15.
            } else {
                0.
            }
            + if state.keyword == self.current {
                18.
            } else {
                0.
            })
        .min(200.)
    }
    pub(crate) fn preferred_width(&self, window: &Window) -> f32 {
        42. + self
            .states
            .iter()
            .map(|s| self.button_width(s, window) + 3.)
            .sum::<f32>()
    }
    fn choose(&self, index: usize, cx: &mut Context<Self>) {
        if let Some(state) = self.states.get(index) {
            cx.emit(TodoPickerEvent::Selected(Some(state.keyword.clone())));
        }
    }
    fn activate(&mut self, cx: &mut Context<Self>) {
        if self.more_open {
            cx.emit(if self.more_selected == 0 {
                TodoPickerEvent::Selected(None)
            } else {
                TodoPickerEvent::Customize
            });
        } else if self.selected == self.states.len() {
            self.more_open = true;
            self.more_selected = 0;
        } else {
            self.choose(self.selected, cx);
        }
    }
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.more_open {
            self.more_open = false;
            cx.notify();
        } else {
            cx.emit(TodoPickerEvent::Cancelled);
        }
    }
    pub(crate) fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = &event.keystroke.key;
        let modifiers = event.keystroke.modifiers;
        if key == "escape" || (modifiers.control && key == "g") {
            self.cancel(cx);
            return true;
        }
        if modifiers.platform || modifiers.control || modifiers.alt {
            return false;
        }
        match key.as_str() {
            "left" | "up" | "right" | "down" | "tab" => {
                let backwards =
                    matches!(key.as_str(), "left" | "up") || (key == "tab" && modifiers.shift);
                if self.more_open {
                    self.more_selected = 1 - self.more_selected;
                } else {
                    let count = self.states.len() + 1;
                    self.selected = (self.selected + if backwards { count - 1 } else { 1 }) % count;
                    if self.selected < self.states.len() {
                        self.scroll.scroll_to_item(self.selected);
                    }
                }
                self.keyboard_navigation = true;
            }
            "enter" | "space" | " " => self.activate(cx),
            _ => {
                let typed = event.keystroke.key_char.clone().unwrap_or_else(|| {
                    if modifiers.shift {
                        key.to_uppercase()
                    } else {
                        key.clone()
                    }
                });
                if let Some(index) = self
                    .states
                    .iter()
                    .position(|s| self.fast_key(s).is_some_and(|c| c.to_string() == typed))
                {
                    self.choose(index, cx);
                } else {
                    return false;
                }
            }
        }
        cx.notify();
        true
    }
    fn status_color(state: &TodoState) -> u32 {
        let theme = current_theme();
        match state.keyword.as_ref() {
            "STRT" | "DOING" | "NEXT" => theme.todo_active,
            "WAIT" | "WAITING" | "HOLD" => theme.waiting,
            "PROJ" => theme.todo_project,
            _ if state.kind == TodoStateKind::Done => theme.done,
            _ => theme.todo,
        }
    }
}
impl Render for TodoPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = current_theme();
        let mut options = div()
            .id("todo-options")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_x_scroll()
            .flex()
            .items_center()
            .gap(px(3.));
        for (index, state) in self.states.iter().enumerate() {
            let current = self.current == state.keyword;
            let keyword = state.keyword.to_string();
            let debug_keyword = keyword.clone();
            let current_keyword = keyword.clone();
            options = options.child(
                div()
                    .id(("todo-option", index))
                    .debug_selector(move || format!("todo-option-{debug_keyword}"))
                    .w(px(self.button_width(state, window)))
                    .h(px(26.))
                    .flex_none()
                    .px(px(8.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .when(current, |s| {
                        s.bg(rgb(SELECTED_BACKGROUND)).text_color(rgb(0xffffff))
                    })
                    .when(!current, |s| s.text_color(rgb(Self::status_color(state))))
                    .when(
                        !current
                            && self.keyboard_navigation
                            && index == self.selected
                            && !self.more_open,
                        |s| s.bg(rgb(HOVER_BACKGROUND)),
                    )
                    .hover(move |s| {
                        s.bg(rgb(if current {
                            SELECTED_HOVER_BACKGROUND
                        } else {
                            HOVER_BACKGROUND
                        }))
                    })
                    .when(current, |s| {
                        s.child(
                            div()
                                .flex_none()
                                .debug_selector(move || format!("todo-current-{current_keyword}"))
                                .child("✓"),
                        )
                    })
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .font_family("Menlo")
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(keyword),
                    )
                    .when_some(self.fast_key(state), |s, key| {
                        s.child(
                            div()
                                .flex_none()
                                .text_size(px(10.))
                                .opacity(0.65)
                                .child(key.to_string()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(index, cx))),
            );
        }
        let bar = div()
            .id("todo-bar")
            .debug_selector(|| "todo-bar".into())
            .h(px(BAR_HEIGHT))
            .w_full()
            .p(px(4.))
            .border_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.background))
            .rounded(px(10.))
            .shadow_md()
            .flex()
            .items_center()
            .gap(px(3.))
            .child(options)
            .child(
                div()
                    .id("todo-more")
                    .debug_selector(|| "todo-more".into())
                    .w(px(28.))
                    .h(px(26.))
                    .flex_none()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(
                        self.more_open
                            || (self.keyboard_navigation && self.selected == self.states.len()),
                        |s| s.bg(rgb(HOVER_BACKGROUND)),
                    )
                    .hover(|s| s.bg(rgb(HOVER_BACKGROUND)))
                    .text_color(rgb(theme.foreground_dim))
                    .child("···")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.more_open = !this.more_open;
                        this.more_selected = 0;
                        this.selected = this.states.len();
                        cx.notify();
                    })),
            );
        let menu = self.more_open.then(|| {
            let mut menu = div()
                .id("todo-more-menu")
                .debug_selector(|| "todo-more-menu".into())
                .w(px(self.width.min(190.)))
                .h(px(MORE_HEIGHT - 4.))
                .p(px(4.))
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(theme.border))
                .bg(rgb(theme.background))
                .shadow_md()
                .flex()
                .flex_col();
            for (index, (id, icon, key)) in [
                ("todo-remove", "−", "todo.remove"),
                ("todo-customize", "⚙", "todo.customize"),
            ]
            .into_iter()
            .enumerate()
            {
                menu = menu.child(
                    div()
                        .id(id)
                        .debug_selector(move || id.into())
                        .h(px(32.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .when(
                            self.keyboard_navigation && self.more_selected == index,
                            |s| s.bg(rgb(HOVER_BACKGROUND)),
                        )
                        .hover(|s| s.bg(rgb(HOVER_BACKGROUND)))
                        .child(icon)
                        .child(self.language.text(key))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(if index == 0 {
                                TodoPickerEvent::Selected(None)
                            } else {
                                TodoPickerEvent::Customize
                            })
                        })),
                );
            }
            menu
        });
        let mut content = div()
            .id("todo-picker")
            .debug_selector(|| "todo-picker".into())
            .key_context("TodoPicker")
            .track_focus(&self.focus)
            .w(px(self.width))
            .font_family(".SystemUIFont")
            .font_weight(gpui::FontWeight::NORMAL)
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(rgb(theme.foreground))
            .cursor_default()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(4.))
            .on_action(cx.listener(|this, _: &CancelTodo, _, cx| this.cancel(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event, _, cx| {
                if this.handle_key(event, cx) {
                    cx.stop_propagation();
                }
            }));
        if self.more_above {
            content = content.children(menu).child(bar);
        } else {
            content = content.child(bar).children(menu);
        }
        content
    }
}
