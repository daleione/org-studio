use crate::app::WorkspaceWindow;
use crate::{
    babel::mysql::{ConnectionTarget, MySqlRequest},
    components::UiTextExt,
    document::{DocumentId, DocumentSession, Revision},
    typography::{ui_font, ui_px},
};
use gpui::{
    Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement, MouseButton, Role, Window,
    div, prelude::*, px, rgb,
};

pub(crate) struct QueryConfirmation {
    document: DocumentId,
    revision: Revision,
    name: gpui::SharedString,
    target: ConnectionTarget,
    ask_each_time: bool,
    focuses: [FocusHandle; 2],
    answer: async_channel::Sender<bool>,
}

impl WorkspaceWindow {
    pub(crate) fn prompt_mysql_execution(
        &mut self,
        mysql: &MySqlRequest,
        session: &Entity<DocumentSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<bool> {
        let (answer, receiver) = async_channel::bounded(1);
        let Some(target) = mysql.target() else {
            return receiver;
        };
        let document = session.read(cx);
        let prompt = QueryConfirmation {
            document: document.id(),
            revision: document.revision(),
            name: document.display_name().into(),
            target,
            ask_each_time: mysql.ask_each_time,
            focuses: std::array::from_fn(|_| cx.focus_handle()),
            answer,
        };
        self.cancel_pending_document_focus(cx);
        window.focus(&prompt.focuses[1], cx);
        self.mysql.confirmation = Some(prompt);
        cx.notify();
        receiver
    }

    fn answer_mysql_confirmation(
        &mut self,
        approved: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.mysql.confirmation.take() else {
            return;
        };
        let _ = prompt.answer.try_send(approved);
        if self.content_route == crate::app::ContentRoute::Document
            && let Some(editor) = self.editor(self.document_workspace.active_pane)
        {
            window.focus(&editor.read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }

    pub(crate) fn mysql_confirmation_capture_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(prompt) = self.mysql.confirmation.as_ref() else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if key == "escape" || modifiers.control && key == "g" {
            self.answer_mysql_confirmation(false, window, cx);
        } else if key == "tab" {
            let index = usize::from(prompt.focuses[0].is_focused(window));
            window.focus(&prompt.focuses[index], cx);
        } else if matches!(key, "enter" | "space") {
            let approved = prompt.focuses[1].is_focused(window);
            self.answer_mysql_confirmation(approved, window, cx);
        }
        cx.stop_propagation();
        true
    }

    pub(crate) fn render_mysql_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let prompt = self.mysql.confirmation.as_ref()?;
        if self.content_route != crate::app::ContentRoute::Document
            || !self.document_session().is_some_and(|session| {
                let document = session.read(cx);
                document.id() == prompt.document && document.revision() == prompt.revision
            })
        {
            self.answer_mysql_confirmation(false, window, cx);
            return None;
        }
        let theme = crate::theme::current_theme();
        let (dialog_background, dialog_border, scrim_opacity) =
            if crate::theme::effective_theme_is_dark() {
                (theme.elevated, theme.border_hover, 0.5)
            } else {
                (theme.background, theme.border, 0.16)
            };
        let width = (window.viewport_size().width - px(32.))
            .max(px(1.))
            .min(px(600.));
        let compact = width < px(520.);
        let title = self.buffer_text("运行此查询？", "Run this query?");
        let note = if prompt.ask_each_time {
            self.buffer_text(
                "此文档要求每次运行前确认。",
                "This document asks before every run.",
            )
        } else {
            self.buffer_text(
                "本次会话内记住此文档与连接",
                "Remember this document and connection for this session.",
            )
        };
        let buttons = [
            (
                "mysql-confirm-cancel",
                self.buffer_text("取消", "Cancel"),
                false,
            ),
            (
                "mysql-confirm-run",
                self.buffer_text("运行查询", "Run Query"),
                true,
            ),
        ]
        .into_iter()
        .zip(&prompt.focuses)
        .map(|((id, label, approved), focus)| {
            let entity = cx.entity();
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .role(Role::Button)
                .aria_label(label)
                .track_focus(focus)
                .tab_index(0)
                .h(ui_px(36.))
                .px(px(16.))
                .min_w(px(80.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(if approved {
                    theme.accent
                } else {
                    dialog_border
                }))
                .bg(rgb(if approved {
                    theme.accent
                } else {
                    dialog_background
                }))
                .text_color(rgb(if approved { 0xffffff } else { theme.foreground }))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(move |style| {
                    style.bg(rgb(if approved {
                        theme.accent_bright
                    } else {
                        theme.hover
                    }))
                })
                .focus(|style| style.border_color(rgb(theme.accent_bright)))
                .child_text(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.answer_mysql_confirmation(approved, window, cx);
                }))
                .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.answer_mysql_confirmation(approved, window, cx)
                    });
                })
        });
        let database = if prompt.target.database.is_empty() {
            self.buffer_text("未指定数据库", "No default database")
                .to_owned()
        } else {
            prompt.target.database.clone()
        };
        Some(
            div()
                .id("mysql-query-confirmation")
                .debug_selector(|| "mysql-query-confirmation".into())
                .absolute()
                .inset_0()
                .occlude()
                .cursor_default()
                .bg(gpui::black().opacity(scrim_opacity))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("mysql-query-dialog")
                        .debug_selector(|| "mysql-query-dialog".into())
                        .role(Role::Dialog)
                        .aria_label(title)
                        .w(width)
                        .max_h((window.viewport_size().height - px(32.)).max(px(1.)))
                        .p(px(26.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(rgb(dialog_border))
                        .bg(rgb(dialog_background))
                        .shadow_lg()
                        .overflow_y_scroll()
                        .font(ui_font(".SystemUIFont"))
                        .text_size(ui_px(13.))
                        .line_height(ui_px(20.))
                        .text_color(rgb(theme.foreground))
                        .flex()
                        .flex_col()
                        .gap(px(22.))
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap(px(7.))
                                .child(
                                    div()
                                        .text_size(ui_px(11.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(theme.accent))
                                        .child_text("MySQL"),
                                )
                                .child(
                                    div()
                                        .text_size(ui_px(22.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child_text(title),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w_full()
                                .py(px(10.))
                                .flex()
                                .items_center()
                                .gap(px(if compact { 12. } else { 18. }))
                                .child(connection_endpoint(
                                    "mysql-confirm-document",
                                    self.buffer_text("文档", "Document"),
                                    prompt.name.clone(),
                                    include_bytes!("../../../assets/icons/agenda/file-text.svg"),
                                    compact,
                                ))
                                .child(
                                    gpui::svg()
                                        .data(include_bytes!("../assets/connection-arrow.svg"))
                                        .flex_none()
                                        .w(ui_px(if compact { 28. } else { 40. }))
                                        .h(ui_px(24.))
                                        .text_color(rgb(theme.accent)),
                                )
                                .child(connection_endpoint(
                                    "mysql-confirm-database",
                                    self.buffer_text("数据库", "Database"),
                                    database.into(),
                                    include_bytes!("../assets/database.svg"),
                                    compact,
                                )),
                        )
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_col()
                                .border_t_1()
                                .border_b_1()
                                .border_color(rgb(theme.border))
                                .child(
                                    connection_row(
                                        "mysql-confirm-host",
                                        self.buffer_text("主机", "Host"),
                                        prompt.target.host.clone(),
                                    )
                                    .py(px(14.)),
                                )
                                .child(
                                    div()
                                        .py(px(14.))
                                        .border_t_1()
                                        .border_color(rgb(theme.divider))
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(12.))
                                        .child(
                                            connection_row(
                                                "mysql-confirm-user",
                                                self.buffer_text("账号", "User"),
                                                prompt.target.user.clone(),
                                            )
                                            .flex_1()
                                            .min_w(ui_px(104.)),
                                        )
                                        .when(!compact, |row| {
                                            row.child(
                                                div()
                                                    .flex_none()
                                                    .w(px(1.))
                                                    .h(ui_px(16.))
                                                    .bg(rgb(theme.divider)),
                                            )
                                        })
                                        .child(
                                            connection_row(
                                                "mysql-confirm-port",
                                                self.buffer_text("端口", "Port"),
                                                prompt.target.port.to_string(),
                                            )
                                            .flex_1()
                                            .min_w(ui_px(96.)),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .ml_auto()
                                                .px(px(8.))
                                                .py(px(2.))
                                                .rounded(px(6.))
                                                .text_size(ui_px(11.))
                                                .text_color(rgb(theme.success))
                                                .bg(rgb(theme.success).opacity(0.1))
                                                .child_text(self.buffer_text("只读", "Read only")),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(16.))
                                .when(compact, |footer| footer.flex_col().items_stretch())
                                .child(
                                    div()
                                        .min_w_0()
                                        .when(!compact, |note| note.flex_1())
                                        .text_size(ui_px(11.))
                                        .text_color(rgb(theme.foreground).opacity(0.7))
                                        .child_text(note),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .flex()
                                        .justify_end()
                                        .gap(px(8.))
                                        .children(buttons),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

fn connection_endpoint(
    id: &'static str,
    label: &'static str,
    value: gpui::SharedString,
    icon: &'static [u8],
    compact: bool,
) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(12.))
        .when(compact, |node| node.flex_col().gap(px(8.)))
        .child(
            div()
                .flex_none()
                .size(ui_px(if compact { 44. } else { 52. }))
                .rounded_full()
                .bg(rgb(theme.accent).opacity(0.08))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    gpui::svg()
                        .data(icon)
                        .size(ui_px(26.))
                        .text_color(rgb(theme.accent)),
                ),
        )
        .child(
            div()
                .min_w_0()
                .when(compact, |text| text.w_full().text_center())
                .when(!compact, |text| text.flex_1())
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(ui_px(11.))
                        .text_color(rgb(theme.foreground).opacity(0.7))
                        .child_text(label),
                )
                .child(
                    connection_value(id, value)
                        .text_size(ui_px(14.))
                        .font_weight(FontWeight::SEMIBOLD),
                ),
        )
}

fn connection_row(id: &'static str, label: &'static str, value: String) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .text_size(ui_px(12.))
        .child(
            div()
                .w(ui_px(36.))
                .flex_none()
                .text_color(rgb(theme.foreground).opacity(0.7))
                .child_text(label),
        )
        .child(connection_value(id, value).flex_1().font(ui_font("Menlo")))
}

fn connection_value(
    id: &'static str,
    value: impl Into<gpui::SharedString>,
) -> gpui::Stateful<gpui::Div> {
    let value = value.into();
    let hint = value.clone();
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .min_w_0()
        .truncate()
        .tooltip(move |_, cx| cx.new(|_| ConnectionHint(hint.clone())).into())
        .child_text(value)
}

struct ConnectionHint(gpui::SharedString);
impl gpui::Render for ConnectionHint {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::current_theme();
        div()
            .p(px(8.))
            .max_w(px(640.))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.elevated))
            .text_color(rgb(theme.foreground))
            .font(ui_font(".SystemUIFont"))
            .text_size(ui_px(12.))
            .shadow_md()
            .child_text(self.0.clone())
    }
}
