//! Small, reusable controls; document ownership and positioning belong to the host.
use super::{
    native_input::{InputConfig, InputEvent, NativeInput},
    selection_style::*,
};
use crate::{i18n::Language, org_semantic::valid_tag, theme::current_theme};
use gpui::{prelude::*, *};

mod view;

#[derive(Clone)]
pub(crate) struct PriorityValue {
    pub current: String,
    pub choices: Vec<String>,
}
#[derive(Clone)]
pub(crate) struct TagsValue {
    pub local: Vec<String>,
    pub inherited: Vec<String>,
    pub available: Vec<String>,
}
#[derive(Clone)]
pub(crate) struct LinkValue {
    pub target: String,
    pub title: Option<String>,
    pub preview: Vec<String>,
    pub can_open: bool,
    pub can_edit: bool,
}
#[derive(Clone)]
pub(crate) enum InlineValue {
    Priority(PriorityValue),
    Tags(TagsValue),
    Link(LinkValue),
}
pub(crate) enum InlineEvent {
    Priority(Option<String>),
    Tags(Vec<String>),
    Link(String),
    Open,
    Copy,
    Cancel,
}
pub(crate) struct InlinePicker {
    pub value: InlineValue,
    pub language: Language,
    pub width: f32,
    pub height: f32,
    editing: bool,
    focus_on_render: bool,
    error: bool,
    input: Entity<NativeInput>,
    focus: FocusHandle,
    selected: usize,
    keyboard_navigation: bool,
    priority_scroll: ScrollHandle,
    _subscription: Subscription,
}
impl EventEmitter<InlineEvent> for InlinePicker {}
impl InlinePicker {
    pub(crate) fn is_pinned(&self) -> bool {
        matches!(self.value, InlineValue::Tags(_)) || self.editing
    }

    pub fn new(value: InlineValue, language: Language, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            NativeInput::new(
                InputConfig {
                    id: "inline-input",
                    outlined: true,
                    font_size: 13.,
                    ..Default::default()
                },
                cx,
            )
        });
        let subscription = cx.subscribe(&input, |this, _, event, cx| match event {
            InputEvent::Command { key, .. } if key == "escape" => cx.emit(InlineEvent::Cancel),
            InputEvent::Command { key, .. } if key == "enter" => {
                if matches!(this.value, InlineValue::Tags(_)) {
                    this.add_tag(cx);
                } else {
                    this.apply(cx);
                }
            }
            InputEvent::Changed(_) => {
                this.error = false;
                this.input.update(cx, |i, cx| {
                    i.invalid = false;
                    cx.notify();
                });
                cx.notify();
            }
            _ => cx.notify(),
        });
        let pinned = matches!(value, InlineValue::Tags(_));
        let selected = match &value {
            InlineValue::Priority(PriorityValue { current, choices }) => {
                choices.iter().position(|c| c == current).unwrap_or(0)
            }
            _ => 0,
        };
        let priority_scroll = ScrollHandle::new();
        priority_scroll.scroll_to_item(selected);
        Self {
            value,
            language,
            width: 320.,
            height: 300.,
            editing: false,
            focus_on_render: pinned,
            error: false,
            input,
            focus: cx.focus_handle(),
            selected,
            keyboard_navigation: false,
            priority_scroll,
            _subscription: subscription,
        }
    }
    fn priority_width(choice: &str, current: &str, window: &Window) -> f32 {
        (text_width(choice, 12., window) + 18. + if choice == current { 18. } else { 0. }).max(28.)
    }
    fn remove_width(&self, window: &Window) -> f32 {
        text_width(self.language.text("inline.remove"), 11., window) + 18.
    }
    pub fn preferred_size(&self, window: &Window) -> (f32, f32) {
        match &self.value {
            InlineValue::Priority(PriorityValue { current, choices }) => (
                10. + choices
                    .iter()
                    .map(|c| Self::priority_width(c, current, window) + 3.)
                    .sum::<f32>()
                    + 10.
                    + self.remove_width(window),
                QUICK_BAR_HEIGHT,
            ),
            InlineValue::Tags(TagsValue {
                available,
                inherited,
                local,
            }) => {
                let mut rows = usize::from(!inherited.is_empty());
                let mut used = 0.;
                let line_width = self.width.min(280.) - 22.;
                for tag in inherited {
                    let width = (text_width(tag, 11., window) + 14.).min(line_width);
                    if used > 0. && used + 4. + width > line_width {
                        rows += 1;
                        used = 0.;
                    }
                    used += width + if used > 0. { 4. } else { 0. };
                }
                let inherited_height = if rows == 0 {
                    0.
                } else {
                    32. + rows as f32 * 26.
                };
                (
                    280.,
                    121. + (available
                        .iter()
                        .filter(|tag| !inherited.contains(tag) || local.contains(tag))
                        .count()
                        .min(6) as f32
                        * 29.
                        - 3.)
                        .max(26.)
                        + inherited_height,
                )
            }
            InlineValue::Link(LinkValue {
                target,
                title,
                preview,
                ..
            }) => {
                if self.editing {
                    return (300., if self.error { 139. } else { 112. });
                }
                let label = title.as_deref().unwrap_or(target);
                let width = (text_width(label, 12., window) + 62.).clamp(240., 320.);
                let header = if title.is_some() { 50. } else { 42. };
                let body = if preview.is_empty() {
                    0.
                } else {
                    10. + preview.len() as f32 * 18.
                };
                (width, 2. + header + body + 35.)
            }
        }
    }
    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        let focus = self.input.read(cx).focus.clone();
        window.focus(&focus, cx);
    }
    fn add_tag(&mut self, cx: &mut Context<Self>) {
        let tag = self.input.read(cx).text.trim().to_owned();
        if !valid_tag(&tag) {
            return;
        }
        if let InlineValue::Tags(TagsValue {
            local, available, ..
        }) = &mut self.value
        {
            if !local.contains(&tag) {
                local.push(tag.clone());
            }
            if !available.contains(&tag) {
                available.push(tag);
                available.sort();
            }
        }
        self.input.update(cx, |i, cx| i.sync("", cx));
        cx.notify();
    }
    pub fn reject_input(&mut self, cx: &mut Context<Self>) {
        self.error = true;
        self.input.update(cx, |i, cx| {
            i.invalid = true;
            cx.notify();
        });
        cx.notify();
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        match &self.value {
            InlineValue::Tags(TagsValue { local, .. }) => cx.emit(InlineEvent::Tags(local.clone())),
            InlineValue::Link(_) if self.editing => {
                let target = self.input.read(cx).text.trim();
                if !target.is_empty() && !target.contains(['\n', '\r']) {
                    cx.emit(InlineEvent::Link(target.to_owned()));
                } else {
                    self.reject_input(cx);
                }
            }
            _ => {}
        }
    }
    pub fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = event.keystroke.key.as_str();
        if key == "escape" && self.input.read(cx).is_composing() {
            return false;
        }
        if key == "escape" {
            cx.emit(InlineEvent::Cancel);
            return true;
        }
        let InlineValue::Priority(PriorityValue { choices, .. }) = &self.value else {
            return false;
        };
        if event.keystroke.modifiers.platform
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
        {
            return false;
        }
        match key {
            "left" | "right" | "tab" => {
                let count = choices.len() + 1;
                self.selected = (self.selected
                    + if key == "left" || event.keystroke.modifiers.shift {
                        count - 1
                    } else {
                        1
                    })
                    % count;
                self.keyboard_navigation = true;
                self.priority_scroll
                    .scroll_to_item(self.selected + usize::from(self.selected == choices.len()));
                cx.notify();
            }
            "enter" | "space" | " " => {
                cx.emit(InlineEvent::Priority(choices.get(self.selected).cloned()))
            }
            "backspace" | "delete" => cx.emit(InlineEvent::Priority(None)),
            _ => {
                if let Some(value) = choices.iter().find(|value| value.eq_ignore_ascii_case(key)) {
                    cx.emit(InlineEvent::Priority(Some(value.clone())));
                } else {
                    return false;
                }
            }
        }
        true
    }
}
fn text_width(text: &str, size: f32, window: &Window) -> f32 {
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font(".SystemUIFont"),
        ..Default::default()
    };
    f32::from(
        window
            .text_system()
            .shape_line(text, px(size), &[run], None)
            .width,
    )
}
