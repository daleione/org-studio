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
    div()
        .h(px(34.))
        .px_4()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .border_1()
        .border_color(rgb(if primary { 0x6f39c1 } else { 0xdfe1e4 }))
        .bg(rgb(if primary { 0x7540c4 } else { 0xffffff }))
        .text_color(rgb(if primary { 0xffffff } else { 0x454950 }))
        .text_size(px(11.))
        .cursor_pointer()
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
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(language.text("agenda.inbox")),
                )
                .child(
                    div()
                        .mt_1()
                        .text_size(px(10.))
                        .text_color(rgb(0x858990))
                        .child(
                            language
                                .text("agenda.items_pending")
                                .replace("{count}", &tasks.len().to_string()),
                        ),
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
                            .border_color(rgb(0xdfe1e4))
                            .bg(rgb(0xffffff))
                            .child(
                                div().text_size(px(10.)).text_color(rgb(0x858990)).child(
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
                                    .text_color(rgb(0x8a8e95))
                                    .child(language.text("agenda.add_note")),
                            )
                            .child(
                                div()
                                    .mt_8()
                                    .pt_4()
                                    .border_t_1()
                                    .border_color(rgb(0xececef))
                                    .text_size(px(10.))
                                    .text_color(rgb(0x8a8e95))
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
                    .border_color(rgb(0xdfe1e4))
                    .bg(rgb(0xfafafb))
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
                            .border_color(rgb(0xdfe1e4))
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
            .mt_5()
            .flex_1()
            .min_h_0()
            .id("inbox-list-scroll")
            .overflow_y_scroll()
            .child(
                div()
                    .h(px(34.))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(10.))
                    .text_color(rgb(0x858990))
                    .child(div().w(px(18.)))
                    .child(div().w(px(110.)).child(language.text("agenda.source")))
                    .child(div().w(px(88.)).child(language.text("agenda.captured")))
                    .child(div().w(px(80.)).child(language.text("agenda.scheduled")))
                    .child(div().w(px(72.)).child(language.text("agenda.status")))
                    .child(div().w(px(38.)).child(language.text("agenda.flag")))
                    .child(div().flex_1().child(language.text("agenda.task")))
                    .child(div().w(px(120.)).child(language.text("agenda.tags"))),
            )
            .child(
                div()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(rgb(0xe1e2e5))
                    .overflow_hidden()
                    .children(tasks.into_iter().map(|task| {
                        let open = workspace.clone();
                        let key = task.key;
                        div()
                            .min_h(px(62.))
                            .px_4()
                            .flex()
                            .items_center()
                            .gap_3()
                            .border_t_1()
                            .border_color(rgb(0xeeeeef))
                            .bg(rgb(0xffffff))
                            .cursor_pointer()
                            .child(
                                div()
                                    .size(px(9.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(0xb8bcc3)),
                            )
                            .child(
                                div()
                                    .w(px(110.))
                                    .text_size(px(11.))
                                    .text_color(rgb(0x585d65))
                                    .child(
                                        task.source
                                            .path
                                            .file_name()
                                            .and_then(|v| v.to_str())
                                            .unwrap_or("")
                                            .to_owned(),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(88.))
                                    .text_size(px(10.))
                                    .text_color(rgb(0x8a8e95))
                                    .child(language.text("agenda.just_now")),
                            )
                            .child(
                                div()
                                    .w(px(80.))
                                    .text_size(px(10.))
                                    .text_color(rgb(0x8a8e95))
                                    .child(language.text("agenda.not_scheduled")),
                            )
                            .child(
                                div()
                                    .w(px(72.))
                                    .h(px(25.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.))
                                    .bg(rgb(0xf0edf5))
                                    .text_color(rgb(0x754b82))
                                    .text_size(px(10.))
                                    .child(language.text("agenda.unprocessed")),
                            )
                            .child(
                                div()
                                    .w(px(38.))
                                    .text_size(px(16.))
                                    .text_color(rgb(0xa0a3a9))
                                    .child("☆"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(12.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(task.title.to_string()),
                            )
                            .child(div().w(px(120.)).flex().gap_1().children(
                                task.effective_tags.iter().take(2).map(|tag| {
                                    div()
                                        .px_2()
                                        .py_1()
                                        .rounded_full()
                                        .bg(rgb(0xf3edf7))
                                        .text_color(rgb(0x754b82))
                                        .text_size(px(9.))
                                        .child(tag.to_string())
                                }),
                            ))
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
                    .text_color(rgb(0x9699a0))
                    .child(language.text("agenda.inbox_hint")),
            )
            .into_any_element()
    };
    div()
        .size_full()
        .p_6()
        .pb_8()
        .flex()
        .flex_col()
        .bg(rgb(0xfdfdfe))
        .child(header)
        .child(body)
}

fn choice_group(label: &'static str, values: &[&'static str]) -> gpui::Div {
    div()
        .mb_5()
        .child(
            div()
                .mb_2()
                .text_size(px(10.))
                .text_color(rgb(0x777b82))
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
                        .border_color(rgb(if index == 1 { 0xcab0e9 } else { 0xdfe1e4 }))
                        .bg(rgb(if index == 1 { 0xf1eafa } else { 0xffffff }))
                        .text_color(rgb(if index == 1 { 0x6f39b8 } else { 0x555960 }))
                        .text_size(px(10.))
                        .child(*value)
                })),
        )
}
