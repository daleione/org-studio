use super::super::UiIntent;
use crate::{
    agenda::{InboxSession, TaskRecord},
    app::WorkspaceWindow,
};
use gpui::{Entity, MouseButton, div, prelude::*, px, rgb};

pub(super) fn button(
    workspace: Entity<WorkspaceWindow>,
    label: &'static str,
    intent: UiIntent,
    primary: bool,
) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .h(px(34.))
        .px_4()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .border_1()
        .border_color(rgb(if primary {
            theme.todo_active
        } else {
            theme.border
        }))
        .bg(rgb(if primary {
            theme.todo_active
        } else {
            theme.background
        }))
        .text_color(rgb(if primary {
            0xffffff
        } else {
            theme.foreground_dim
        }))
        .text_size(px(11.))
        .cursor_pointer()
        .hover(move |style| {
            if primary {
                style
                    .bg(rgb(theme.todo_active))
                    .border_color(rgb(theme.todo_active))
            } else {
                style
                    .bg(rgb(theme.hover))
                    .border_color(rgb(theme.border_hover))
            }
        })
        .child(label)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let intent = intent.clone();
            workspace.update(cx, |this, cx| this.dispatch_agenda_intent(intent, cx));
        })
}

pub(crate) fn inbox_view(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    tasks: Vec<TaskRecord>,
    session: Option<&InboxSession>,
) -> gpui::Div {
    let theme = crate::theme::current_theme();
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(theme.foreground_dim))
                .child(
                    language
                        .text("agenda.items_pending")
                        .replace("{count}", &tasks.len().to_string()),
                ),
        )
        .child(button(
            workspace.clone(),
            if session.is_some() {
                language.text("agenda.exit_triage")
            } else {
                language.text("agenda.start_triage")
            },
            if session.is_some() {
                UiIntent::ExitInboxOrganize
            } else {
                UiIntent::StartInboxOrganize
            },
            true,
        ));
    let body = if let Some(session) = session {
        let (done, total) = session.progress();
        let current = session
            .current()
            .and_then(|key| tasks.iter().find(|task| task.key == key));
        let title = current
            .map(|task| task.title.to_string())
            .unwrap_or_else(|| language.text("agenda.inbox_empty").into());
        let source = current
            .and_then(|task| task.source.path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("tasks.org")
            .to_owned();
        div()
            .mt_5()
            .flex_1()
            .min_h_0()
            .flex()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .min_h(px(360.))
                            .p_6()
                            .rounded(px(9.))
                            .border_1()
                            .border_color(rgb(theme.border))
                            .bg(rgb(theme.background))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(theme.foreground_dim))
                                    .child(
                                        language
                                            .text("agenda.item_position")
                                            .replace("{index}", &(done + 1).to_string())
                                            .replace("{total}", &total.to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .mt_5()
                                    .text_size(px(22.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .mt_4()
                                    .text_size(px(11.))
                                    .text_color(rgb(theme.foreground_dim))
                                    .child(language.text("agenda.add_note")),
                            )
                            .child(
                                div()
                                    .mt_8()
                                    .pt_4()
                                    .border_t_1()
                                    .border_color(rgb(theme.divider))
                                    .text_size(px(10.))
                                    .text_color(rgb(theme.foreground_dim))
                                    .child(format!("{source} · Capture template: task")),
                            ),
                    )
                    .child(div().mt_4().flex().justify_center().child(button(
                        workspace.clone(),
                        language.text("agenda.skip"),
                        UiIntent::InboxSkip,
                        false,
                    ))),
            )
            .child(
                div()
                    .w(px(390.))
                    .flex_none()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.surface))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .p_5()
                            .id("inbox-organize-scroll")
                            .overflow_y_scroll()
                            .child(choice_group(
                                language.text("agenda.what_action"),
                                &["TODO", "NEXT", "WAITING"],
                            ))
                            .child(choice_group(
                                language.text("agenda.move_project"),
                                &[
                                    "Agenda 首期设计",
                                    "Agenda 使用指南",
                                    language.text("agenda.other_location"),
                                ],
                            ))
                            .child(choice_group(
                                language.text("agenda.when"),
                                &[
                                    language.text("agenda.today"),
                                    language.text("agenda.tomorrow"),
                                    language.text("agenda.next_week"),
                                ],
                            ))
                            .child(choice_group(
                                language.text("agenda.priority"),
                                &[
                                    language.text("agenda.none"),
                                    language.text("agenda.high"),
                                    language.text("agenda.medium"),
                                    language.text("agenda.low"),
                                ],
                            ))
                            .child(choice_group(
                                language.text("agenda.tags"),
                                &["agenda", "产品", "文档", "沟通", "个人"],
                            )),
                    )
                    .child(
                        div()
                            .min_h(px(58.))
                            .px_4()
                            .flex()
                            .items_center()
                            .justify_end()
                            .border_t_1()
                            .border_color(rgb(theme.border))
                            .child(button(
                                workspace.clone(),
                                language.text("agenda.move_next"),
                                UiIntent::InboxFinish,
                                true,
                            )),
                    ),
            )
            .into_any_element()
    } else {
        div()
            .mt_4()
            .flex_1()
            .min_h_0()
            .id("inbox-list-scroll")
            .overflow_y_scroll()
            .child(
                div()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .overflow_hidden()
                    .children(tasks.into_iter().map(|task| {
                        let open = workspace.clone();
                        let key = task.key;
                        div()
                            .id(format!(
                                "agenda-inbox-task-{}-{}-{}",
                                key.file.0, key.local, key.shard_generation
                            ))
                            .min_h(px(72.))
                            .px_4()
                            .flex()
                            .items_center()
                            .gap_4()
                            .border_t_1()
                            .border_color(rgb(theme.divider))
                            .bg(rgb(theme.background))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(theme.hover)))
                            .child(
                                div()
                                    .size(px(24.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(rgb(theme.border_hover))
                                    .text_color(rgb(theme.foreground_muted))
                                    .text_size(px(12.))
                                    .child("✓"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(px(12.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(theme.foreground))
                                            .child(task.title.to_string()),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(px(10.))
                                            .text_color(rgb(theme.foreground_muted))
                                            .child(format!(
                                                "{} · {} · {}",
                                                task.source
                                                    .path
                                                    .file_name()
                                                    .and_then(|v| v.to_str())
                                                    .unwrap_or(""),
                                                language.text("agenda.unprocessed"),
                                                language.text("agenda.not_scheduled")
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(90.))
                                    .flex_none()
                                    .text_right()
                                    .text_size(px(10.))
                                    .text_color(rgb(theme.foreground_dim))
                                    .child(language.text("agenda.just_now")),
                            )
                            .child(
                                div()
                                    .w(px(18.))
                                    .flex_none()
                                    .text_color(rgb(theme.foreground_disabled))
                                    .text_size(px(16.))
                                    .child("›"),
                            )
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                open.update(cx, |this, cx| {
                                    this.dispatch_agenda_intent(UiIntent::OpenRefile(key), cx)
                                })
                            })
                    })),
            )
            .child(
                div()
                    .mt_3()
                    .px_4()
                    .text_size(px(9.))
                    .text_color(rgb(theme.foreground_muted))
                    .child(language.text("agenda.inbox_hint")),
            )
            .into_any_element()
    };
    div()
        .size_full()
        .px_6()
        .pt_6()
        .flex()
        .flex_col()
        .bg(rgb(theme.surface))
        .child(header)
        .child(body)
}

fn choice_group(label: &'static str, values: &[&'static str]) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .mb_5()
        .child(
            div()
                .mb_2()
                .text_size(px(10.))
                .text_color(rgb(theme.foreground_dim))
                .child(label),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(values.iter().enumerate().map(|(index, value)| {
                    div()
                        .min_h(px(32.))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded(px(7.))
                        .border_1()
                        .border_color(rgb(if index == 1 {
                            theme.accent_border
                        } else {
                            theme.border
                        }))
                        .bg(rgb(if index == 1 {
                            theme.accent_bg
                        } else {
                            theme.background
                        }))
                        .text_color(rgb(if index == 1 {
                            theme.todo_active
                        } else {
                            theme.foreground_dim
                        }))
                        .text_size(px(10.))
                        .child(*value)
                })),
        )
}
