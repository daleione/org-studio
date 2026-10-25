//! Link fields and events only; the editor owns source ranges and transactions.

use gpui::{prelude::*, *};

use super::{
    UiTextExt,
    native_input::{InputConfig, InputEvent, NativeInput},
};
use crate::{
    i18n::Language,
    typography::{ui_family, ui_px},
};

use crate::org_syntax::inline::LinkValue;

pub(crate) enum LinkPickerEvent {
    Applied(String),
    Cancelled,
}

pub(crate) struct LinkPicker {
    target: Entity<NativeInput>,
    description: Entity<NativeInput>,
    language: Language,
    focus_target: Option<bool>,
    invalid: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LinkPickerEvent> for LinkPicker {}

impl LinkPicker {
    pub(crate) fn new(value: LinkValue, language: Language, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        let mut field = |id, text: &str, target: bool| {
            let input = cx.new(|cx| {
                let mut input = NativeInput::new(
                    InputConfig {
                        id,
                        outlined: true,
                        preserve_newlines: true,
                        font_size: 13.,
                        command: |key, modifiers, _| {
                            matches!(key, "enter" | "escape" | "tab")
                                || (modifiers.control && key == "g")
                        },
                        ..Default::default()
                    },
                    cx,
                );
                input.sync(text, cx);
                if target {
                    input.selection = 0..text.len();
                }
                input
            });
            subscriptions.push(cx.subscribe(&input, move |this, _, event, cx| match event {
                InputEvent::Command { key, .. } if key == "escape" || key == "g" => {
                    cx.emit(LinkPickerEvent::Cancelled)
                }
                InputEvent::Command { key, .. } if key == "enter" => this.apply(cx),
                InputEvent::Command { key, .. } if key == "tab" => {
                    this.focus_target = Some(!target);
                    cx.notify();
                }
                InputEvent::Changed(_) => {
                    this.invalid = false;
                    cx.notify();
                }
                _ => {}
            }));
            input
        };
        let target = field("link-target", &value.target, true);
        let description = field("link-description", &value.description, false);
        Self {
            target,
            description,
            language,
            focus_target: Some(true),
            invalid: false,
            _subscriptions: subscriptions,
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let value = LinkValue {
            target: self.target.read(cx).text.clone(),
            description: self.description.read(cx).text.clone(),
        };
        if let Some(source) = value.source() {
            cx.emit(LinkPickerEvent::Applied(source));
        } else {
            self.invalid = true;
            cx.notify();
        }
    }
}

impl Render for LinkPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(target) = self.focus_target.take() {
            let input = if target {
                &self.target
            } else {
                &self.description
            };
            let focus = input.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        let theme = crate::theme::current_theme();
        div()
            .id("link-picker")
            .debug_selector(|| "link-picker".into())
            .w(px(360.))
            .max_w(window.viewport_size().width - px(24.))
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .rounded(px(10.))
            .bg(rgb(theme.background))
            .border_1()
            .border_color(rgb(theme.border))
            .shadow_md()
            .font_family(ui_family(".SystemUIFont"))
            .text_size(ui_px(12.))
            .text_color(rgb(theme.foreground))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child_text(self.language.text("link.edit"))
            .child_text(self.language.text("link.target"))
            .child(self.target.clone())
            .child_text(self.language.text("link.description"))
            .child(self.description.clone())
            .when(self.invalid, |s| {
                s.child_text(self.language.text("link.invalid"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("link-cancel")
                            .debug_selector(|| "link-cancel".into())
                            .px(px(10.))
                            .py(px(5.))
                            .cursor_pointer()
                            .child_text(self.language.text("link.cancel"))
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(LinkPickerEvent::Cancelled)),
                            ),
                    )
                    .child(
                        div()
                            .id("link-apply")
                            .debug_selector(|| "link-apply".into())
                            .px(px(10.))
                            .py(px(5.))
                            .rounded(px(5.))
                            .bg(rgb(theme.link))
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .child_text(self.language.text("link.apply"))
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    ),
            )
    }
}
