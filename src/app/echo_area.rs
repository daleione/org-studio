use std::{sync::Arc, time::Duration};

use gpui::{Context, Entity, MouseButton, Task, div, prelude::*, px, rgb};

use super::{WorkspaceLoadState, WorkspaceWindow};
use crate::theme::current_theme;

pub(crate) const ECHO_AREA_HEIGHT: f32 = 36.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EchoTone {
    Working,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EchoMessage {
    pub(crate) text: Arc<str>,
    pub(crate) tone: EchoTone,
}

impl EchoMessage {
    pub(crate) fn working(text: impl Into<Arc<str>>) -> Self {
        Self::new(text, EchoTone::Working)
    }

    pub(crate) fn success(text: impl Into<Arc<str>>) -> Self {
        Self::new(text, EchoTone::Success)
    }

    pub(crate) fn warning(text: impl Into<Arc<str>>) -> Self {
        Self::new(text, EchoTone::Warning)
    }

    pub(crate) fn error(text: impl Into<Arc<str>>) -> Self {
        Self::new(text, EchoTone::Error)
    }

    fn new(text: impl Into<Arc<str>>, tone: EchoTone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }

    fn dismiss_after(&self) -> Option<Duration> {
        match self.tone {
            EchoTone::Working => None,
            EchoTone::Success => Some(Duration::from_secs(3)),
            EchoTone::Warning => Some(Duration::from_secs(5)),
            EchoTone::Error => Some(Duration::from_secs(8)),
        }
    }
}

#[derive(Default)]
pub(crate) struct EchoAreaHost {
    message: Option<EchoMessage>,
    generation: u64,
    task: Option<Task<()>>,
}

impl WorkspaceWindow {
    pub(crate) fn set_echo_message(&mut self, message: Option<EchoMessage>) {
        self.echo.generation = self.echo.generation.wrapping_add(1);
        self.echo.task = None;
        self.echo.message = message;
    }

    pub(crate) fn dismiss_echo_message(&mut self) -> bool {
        if self.echo.message.is_none() {
            return false;
        }
        self.set_echo_message(None);
        true
    }

    pub(crate) fn show_echo_message(&mut self, message: EchoMessage, cx: &mut Context<Self>) {
        let dismiss_after = message.dismiss_after();
        self.set_echo_message(Some(message));
        let Some(duration) = dismiss_after else {
            cx.notify();
            return;
        };
        let generation = self.echo.generation;
        let delay = cx.background_executor().timer(duration);
        self.echo.task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |this, cx| {
                if this.echo.generation != generation {
                    return;
                }
                this.echo.task = None;
                this.echo.message = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(crate) fn displayed_echo_message(&self) -> Option<EchoMessage> {
        if let WorkspaceLoadState::Failed {
            path,
            message,
            previous: Some(_),
        } = &self.state
        {
            return Some(EchoMessage::error(format!(
                "Could not open {}: {message}. Showing the previous file.",
                path.display()
            )));
        }
        if self
            .echo
            .message
            .as_ref()
            .is_some_and(|message| matches!(message.tone, EchoTone::Error | EchoTone::Warning))
        {
            return self.echo.message.clone();
        }
        self.keyboard
            .status()
            .map(EchoMessage::working)
            .or_else(|| self.echo.message.clone())
    }
}

pub(crate) fn render_echo_area(
    message: Option<EchoMessage>,
    entity: Entity<WorkspaceWindow>,
    pending_keys: Option<&str>,
    language: crate::i18n::Language,
) -> gpui::AnyElement {
    let theme = current_theme();
    let dismiss_entity = entity;
    div()
        .id("workspace-echo-area")
        .h(px(ECHO_AREA_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .overflow_hidden()
        .border_t_1()
        .border_color(rgb(theme.border))
        .bg(rgb(0xf7faff))
        .font_family(".SystemUIFont")
        .text_size(px(11.0))
        .when_some(message, |area, message| {
            let (icon, color) = match message.tone {
                EchoTone::Working => ("ⓘ", 0x3477bb),
                EchoTone::Success => ("✓", theme.heading[2]),
                EchoTone::Warning => ("!", theme.heading[1]),
                EchoTone::Error => ("×", 0xb23a63),
            };
            let is_prefix = pending_keys == Some(message.text.as_ref());
            area.child(
                div()
                    .w(px(14.0))
                    .flex_none()
                    .text_color(rgb(color))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(icon),
            )
            .child(
                div()
                    .when(is_prefix, |label| {
                        label
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(6.0))
                            .bg(rgb(0xe9eef5))
                    })
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(rgb(theme.foreground))
                    .child(message.text.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .text_color(rgb(0x8391a6))
                    .when(is_prefix, |hint| {
                        hint.child(match language {
                            crate::i18n::Language::Chinese => "（按下组合键…）",
                            crate::i18n::Language::English => "(waiting for next key…)",
                        })
                    }),
            )
            .child(
                div()
                    .id("echo-dismiss")
                    .px(px(7.0))
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .text_color(rgb(0x8391a6))
                    .hover(|style| style.bg(rgb(0xe9eef5)))
                    .child("×")
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        dismiss_entity.update(cx, |this, cx| {
                            this.dismiss_echo_message();
                            if is_prefix {
                                this.keyboard.cancel();
                                this.keyboard.dismiss_status();
                                this.cancel_key_feedback();
                                this.cancel_which_key(cx);
                            }
                            cx.notify();
                        });
                    }),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_tones_have_deliberate_lifetimes() {
        assert_eq!(
            EchoMessage::success("done").dismiss_after(),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            EchoMessage::warning("careful").dismiss_after(),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            EchoMessage::error("failed").dismiss_after(),
            Some(Duration::from_secs(8))
        );
        assert_eq!(EchoMessage::working("busy").dismiss_after(), None);
    }
}
