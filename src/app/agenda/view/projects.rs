use super::super::UiIntent;
use crate::{agenda::ProjectSummary, app::WorkspaceWindow};
use gpui::{Entity, MouseButton, div, prelude::*, px, rgb};

pub(crate) fn projects_view(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    projects: Vec<ProjectSummary>,
    selected: usize,
) -> gpui::Div {
    let active = projects.get(selected).or_else(|| projects.first());
    let stuck = projects.iter().filter(|project| project.stuck).count();
    let summary = div()
        .flex()
        .gap_2()
        .child(metric(
            projects.len(),
            language.text("agenda.active_projects"),
        ))
        .child(metric(stuck, language.text("agenda.needs_next")))
        .child(metric(
            projects.iter().map(|p| p.next_actions.len()).sum(),
            "NEXT actions",
        ));
    let list = projects.iter().enumerate().fold(
        div()
            .w(px(292.))
            .flex_none()
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .border_r_1()
            .border_color(rgb(0xe2e3e6))
            .bg(rgb(0xf7f7f8)),
        |list, (index, project)| {
            let target = workspace.clone();
            list.child(
                div()
                    .min_h(px(76.))
                    .p_3()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(index == selected, |row| row.bg(rgb(0xffffff)).shadow_sm())
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(project.project.title.to_string()),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_size(px(9.))
                            .text_color(rgb(if project.stuck { 0xa56617 } else { 0x858990 }))
                            .child(if project.stuck {
                                project
                                    .blocked_reason
                                    .as_deref()
                                    .unwrap_or(language.text("agenda.missing_next"))
                                    .to_owned()
                            } else {
                                language
                                    .text("agenda.project_completed")
                                    .replace("{done}", &project.done.to_string())
                                    .replace("{total}", &project.total.to_string())
                            }),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        target.update(cx, |this, cx| {
                            this.dispatch_agenda_intent(UiIntent::SelectProject(index), cx)
                        })
                    }),
            )
        },
    );
    let detail = if let Some(project) = active {
        let source = workspace.clone();
        let rows = project
            .children
            .iter()
            .fold(div().mt_4().flex().flex_col(), |rows, task| {
                let target = workspace.clone();
                let key = task.key;
                rows.child(
                    div()
                        .min_h(px(43.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_3()
                        .border_t_1()
                        .border_color(rgb(0xeeeeef))
                        .cursor_pointer()
                        .child(
                            div()
                                .w(px(70.))
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .bg(rgb(
                                    if matches!(
                                        task.todo_kind,
                                        crate::org_semantic::TodoStateKind::Done
                                    ) {
                                        0xececef
                                    } else if task.todo.eq_ignore_ascii_case("WAITING") {
                                        0xeaf2fb
                                    } else {
                                        0xe8f4eb
                                    },
                                ))
                                .text_size(px(9.))
                                .child(task.todo.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(11.))
                                .child(task.title.to_string()),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            target.update(cx, |this, cx| {
                                this.dispatch_agenda_intent(UiIntent::OpenSource(key), cx)
                            })
                        }),
                )
            });
        div()
            .flex_1()
            .min_w_0()
            .p_5()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(
                        div()
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(0x7b4e85))
                                    .child(language.text("agenda.project_details")),
                            )
                            .child(
                                div()
                                    .mt_2()
                                    .text_size(px(19.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(project.project.title.to_string()),
                            ),
                    )
                    .child(super::inbox::button(
                        source,
                        language.text("agenda.project_source"),
                        UiIntent::OpenProjectSource,
                        false,
                    )),
            )
            .when(project.stuck, |detail| {
                detail.child(
                    div()
                        .mt_4()
                        .p_3()
                        .rounded(px(8.))
                        .bg(rgb(0xfff5e9))
                        .text_color(rgb(0x8e5a16))
                        .text_size(px(10.))
                        .child(
                            project
                                .blocked_reason
                                .as_deref()
                                .unwrap_or(language.text("agenda.project_hint"))
                                .to_owned(),
                        ),
                )
            })
            .child(
                div()
                    .mt_4()
                    .text_size(px(10.))
                    .text_color(rgb(0x858990))
                    .child(
                        language
                            .text("agenda.project_progress")
                            .replace("{done}", &project.done.to_string())
                            .replace("{total}", &project.total.to_string())
                            .replace("{next}", &project.next_actions.len().to_string()),
                    ),
            )
            .child(rows)
    } else {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(0x858990))
            .child(language.text("agenda.no_projects"))
    };
    div()
        .size_full()
        .p_6()
        .pb_8()
        .flex()
        .flex_col()
        .bg(rgb(0xfdfdfe))
        .child(
            div()
                .text_size(px(20.))
                .font_weight(gpui::FontWeight::BOLD)
                .child(language.text("agenda.projects")),
        )
        .child(div().mt_4().child(summary))
        .child(
            div()
                .mt_4()
                .flex_1()
                .min_h_0()
                .flex()
                .overflow_hidden()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(0xdfe1e4))
                .child(list)
                .child(detail),
        )
}

fn metric(value: usize, label: &'static str) -> gpui::Div {
    div()
        .flex_1()
        .p_3()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(0xe4e5e8))
        .bg(rgb(0xfafafb))
        .child(
            div()
                .text_size(px(18.))
                .font_weight(gpui::FontWeight::BOLD)
                .child(value.to_string()),
        )
        .child(
            div()
                .mt_1()
                .text_size(px(10.))
                .text_color(rgb(0x858990))
                .child(label),
        )
}
