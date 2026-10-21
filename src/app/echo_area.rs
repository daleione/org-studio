use crate::components::UiTextExt;
use crate::typography::{ui_family, ui_px};
use std::{sync::Arc, time::Duration};

use gpui::{
    Animation, AnimationExt, Context, Entity, MouseButton, Role, Task, div, prelude::*, px, rgb,
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EchoAction {
    StopMySqlQuery { request: u64, enabled: bool },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EchoMessage {
    pub(crate) text: Arc<str>,
    pub(crate) tone: EchoTone,
    pub(crate) action: Option<EchoAction>,
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
            action: None,
        }
    }

    pub(crate) fn with_action(mut self, action: EchoAction) -> Self {
        self.action = Some(action);
        self
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

    pub(crate) fn displayed_echo_message(&self, cx: &gpui::App) -> Option<EchoMessage> {
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
            .or_else(|| self.mysql_running_message(cx))
            .or_else(|| self.echo.message.clone())
    }
}

pub(crate) fn render_echo_area(
    message: Option<EchoMessage>,
    entity: Entity<WorkspaceWindow>,
    pending_keys: Option<&str>,
    language: crate::i18n::Language,
) -> gpui::AnyElement {
    render_echo_content(message, entity, pending_keys, language, false)
}

pub(crate) fn render_status_echo(
    message: EchoMessage,
    entity: Entity<WorkspaceWindow>,
    pending_keys: Option<&str>,
    language: crate::i18n::Language,
) -> gpui::AnyElement {
    render_echo_content(Some(message), entity, pending_keys, language, true)
}

fn render_echo_content(
    message: Option<EchoMessage>,
    entity: Entity<WorkspaceWindow>,
    pending_keys: Option<&str>,
    language: crate::i18n::Language,
    inline: bool,
) -> gpui::AnyElement {
    let theme = current_theme();
    let dismiss_entity = entity.clone();
    div()
        .id("workspace-echo-area")
        .when(!inline, |area| {
            area.h(ui_px(ECHO_AREA_HEIGHT))
                .w_full()
                .flex_none()
                .border_t_1()
                .border_color(rgb(theme.border))
                .bg(rgb(theme.echo_area))
        })
        .when(inline, |area| area.h_full().flex_1().min_w_0())
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .overflow_hidden()
        .font_family(ui_family(".SystemUIFont"))
        .text_size(ui_px(11.0))
        .when_some(message, |area, message| {
            let (icon, color) = match message.tone {
                EchoTone::Working => ("ⓘ", theme.info),
                EchoTone::Success => ("✓", theme.success),
                EchoTone::Warning => ("!", theme.warning),
                EchoTone::Error => ("×", theme.error),
            };
            let is_prefix = pending_keys == Some(message.text.as_ref());
            area.child(
                div()
                    .w(px(14.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(color))
                    .font_weight(gpui::FontWeight::BOLD)
                    .when(message.action.is_some(), |icon| {
                        icon.child(
                            gpui::svg()
                                .data(include_bytes!("assets/spinner.svg"))
                                .size(ui_px(14.))
                                .text_color(rgb(color))
                                .with_animation(
                                    "echo-working-spinner",
                                    Animation::new(Duration::from_millis(900))
                                        .repeat()
                                        .with_max_fps(24.),
                                    |icon, progress| {
                                        icon.with_transformation(gpui::Transformation::rotate(
                                            gpui::radians(progress * std::f32::consts::TAU),
                                        ))
                                    },
                                ),
                        )
                    })
                    .when(message.action.is_none(), |element| element.child(icon)),
            )
            .child(
                div()
                    .when(is_prefix, |label| {
                        label
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(6.0))
                            .bg(rgb(theme.hover))
                    })
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(rgb(theme.foreground))
                    .child_text(message.text.to_string()),
            )
            .when(message.action.is_none(), |area| {
                area.child(div().flex_1().text_color(rgb(theme.foreground_muted)).when(
                    is_prefix,
                    |hint| {
                        hint.child_text(match language {
                            crate::i18n::Language::Chinese => "（按下组合键…）",
                            crate::i18n::Language::English => "(waiting for next key…)",
                        })
                    },
                ))
            })
            .when_some(message.action, |area, action| {
                let EchoAction::StopMySqlQuery { request, enabled } = action;
                let stop_entity = entity.clone();
                let accessible_entity = entity.clone();
                let label = match language {
                    crate::i18n::Language::Chinese => "停止",
                    crate::i18n::Language::English => "Stop",
                };
                area.child(
                    div()
                        .id("echo-action")
                        .debug_selector(|| "echo-action".into())
                        .role(Role::Button)
                        .aria_label(label)
                        .flex_none()
                        .h(ui_px(24.))
                        .px(px(9.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .text_color(rgb(if enabled {
                            theme.accent
                        } else {
                            theme.foreground_disabled
                        }))
                        .when(enabled, |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(theme.hover)))
                        })
                        .child_text(label)
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if enabled {
                                stop_entity
                                    .update(cx, |this, cx| this.stop_mysql_query(request, cx));
                            }
                        })
                        .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                            if enabled {
                                accessible_entity
                                    .update(cx, |this, cx| this.stop_mysql_query(request, cx));
                            }
                        }),
                )
            })
            .when(message.action.is_some(), |area| area.child(div().flex_1()))
            .when(message.action.is_none(), |area| {
                area.child(
                    div()
                        .id("echo-dismiss")
                        .flex_none()
                        .px(px(7.0))
                        .rounded(px(5.0))
                        .cursor_pointer()
                        .text_color(rgb(theme.foreground_muted))
                        .hover(|style| style.bg(rgb(theme.hover)))
                        .child_text("×")
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            dismiss_entity.update(cx, |this, cx| {
                                this.dismiss_echo_message();
                                if is_prefix {
                                    this.cancel_prefix_input(window, cx);
                                }
                                cx.notify();
                            });
                        }),
                )
            })
        })
        .into_any_element()
}
