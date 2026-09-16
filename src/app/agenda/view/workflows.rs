use super::super::{UiIntent, state::AgendaOverlay};
use crate::{
    agenda::{CaptureDraft, CaptureTemplate, RefileTarget},
    app::WorkspaceWindow,
};
use gpui::{Entity, MouseButton, div, prelude::*, px, rgb};

fn action(
    workspace: Entity<WorkspaceWindow>,
    label: &'static str,
    intent: UiIntent,
    primary: bool,
) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .debug_selector(move || format!("agenda-workflow-{label}"))
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
            cx.stop_propagation();
            let intent = intent.clone();
            workspace.update(cx, |this, cx| this.dispatch_agenda_intent(intent, cx));
        })
}

fn template(
    workspace: Entity<WorkspaceWindow>,
    label: &'static str,
    value: CaptureTemplate,
    selected: bool,
) -> gpui::Div {
    action(
        workspace,
        label,
        UiIntent::SetCaptureTemplate(value),
        selected,
    )
}

pub(crate) fn capture_overlay(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    draft: &CaptureDraft,
    message: Option<&str>,
) -> gpui::Div {
    let theme = crate::theme::current_theme();
    let close = workspace.clone();
    div()
        .absolute()
        .inset_0()
        .bg(gpui::rgba(0x00000052))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(560.))
                .p_5()
                .rounded(px(12.))
                .bg(rgb(theme.background))
                .shadow_lg()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .child(
                            div()
                                .child(
                                    div()
                                        .text_size(px(19.))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child(language.text("agenda.capture")),
                                )
                                .child(
                                    div()
                                        .mt_1()
                                        .text_size(px(10.))
                                        .text_color(rgb(theme.foreground_dim))
                                        .child(language.text("agenda.capture_hint")),
                                ),
                        )
                        .child(action(
                            close.clone(),
                            language.text("agenda.close"),
                            UiIntent::CloseOverlay,
                            false,
                        )),
                )
                .child(
                    div()
                        .mt_5()
                        .flex()
                        .gap_2()
                        .child(template(
                            workspace.clone(),
                            language.text("agenda.task"),
                            CaptureTemplate::Task,
                            draft.template == CaptureTemplate::Task,
                        ))
                        .child(template(
                            workspace.clone(),
                            language.text("agenda.note"),
                            CaptureTemplate::Note,
                            draft.template == CaptureTemplate::Note,
                        ))
                        .child(template(
                            workspace.clone(),
                            language.text("agenda.meeting"),
                            CaptureTemplate::Meeting,
                            draft.template == CaptureTemplate::Meeting,
                        )),
                )
                .child(
                    div()
                        .mt_4()
                        .min_h(px(46.))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(theme.border))
                        .text_size(px(13.))
                        .child(if draft.title.is_empty() {
                            language.text("agenda.title_hint").into()
                        } else {
                            draft.title.clone()
                        }),
                )
                .when_some(message.map(str::to_owned), |panel, message| {
                    panel.child(
                        div()
                            .mt_3()
                            .text_size(px(10.))
                            .text_color(rgb(theme.warning))
                            .child(message),
                    )
                })
                .child(
                    div()
                        .mt_5()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(action(
                            workspace.clone(),
                            language.text("agenda.cancel"),
                            UiIntent::CloseOverlay,
                            false,
                        ))
                        .child(action(
                            workspace,
                            language.text("agenda.save_inbox"),
                            UiIntent::SubmitCapture,
                            true,
                        )),
                ),
        )
}

pub(crate) fn refile_overlay(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
    targets: Vec<RefileTarget>,
    selected: usize,
    search: &str,
    message: Option<&str>,
) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .absolute()
        .inset_0()
        .bg(gpui::rgba(0x00000052))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(600.))
                .max_h(px(650.))
                .p_5()
                .rounded(px(12.))
                .bg(rgb(theme.background))
                .shadow_lg()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(19.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(language.text("agenda.move_to")),
                )
                .child(
                    div()
                        .mt_1()
                        .text_size(px(10.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(language.text("agenda.refile_hint")),
                )
                .child(
                    div()
                        .mt_4()
                        .h(px(40.))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(theme.border))
                        .text_size(px(12.))
                        .child(if search.is_empty() {
                            language.text("agenda.search_target").into()
                        } else {
                            search.to_owned()
                        }),
                )
                .child(
                    div()
                        .mt_3()
                        .max_h(px(420.))
                        .id("refile-target-scroll")
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(targets.into_iter().enumerate().map(|(index, target)| {
                            let choose = workspace.clone();
                            div()
                                .id(("agenda-refile-target", index))
                                .min_h(px(48.))
                                .px_3()
                                .flex()
                                .items_center()
                                .rounded(px(7.))
                                .when(index == selected, |row| row.bg(rgb(theme.accent_bg)))
                                .cursor_pointer()
                                .hover(move |style| {
                                    style.bg(rgb(if index == selected {
                                        theme.accent_border
                                    } else {
                                        theme.hover
                                    }))
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(target.title.to_string()),
                                        )
                                        .child(
                                            div()
                                                .mt_1()
                                                .text_size(px(9.))
                                                .text_color(rgb(theme.foreground_dim))
                                                .child(
                                                    target
                                                        .path
                                                        .file_name()
                                                        .and_then(|v| v.to_str())
                                                        .unwrap_or("")
                                                        .to_owned(),
                                                ),
                                        ),
                                )
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    choose.update(cx, |this, cx| {
                                        this.agenda.state.refile_selected = index;
                                        cx.notify();
                                    })
                                })
                        })),
                )
                .when_some(message.map(str::to_owned), |panel, message| {
                    panel.child(
                        div()
                            .mt_2()
                            .text_size(px(10.))
                            .text_color(rgb(theme.warning))
                            .child(message),
                    )
                })
                .child(
                    div()
                        .mt_4()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(action(
                            workspace.clone(),
                            language.text("agenda.cancel"),
                            UiIntent::CloseOverlay,
                            false,
                        ))
                        .child(action(
                            workspace,
                            language.text("agenda.move"),
                            UiIntent::SubmitRefile,
                            true,
                        )),
                ),
        )
}

pub(crate) struct WorkflowOverlay<'a> {
    pub language: crate::i18n::Language,
    pub kind: AgendaOverlay,
    pub workspace: Entity<WorkspaceWindow>,
    pub draft: &'a CaptureDraft,
    pub targets: Vec<RefileTarget>,
    pub selected: usize,
    pub search: &'a str,
    pub message: Option<&'a str>,
}

pub(crate) fn workflow_overlay(props: WorkflowOverlay<'_>) -> Option<gpui::Div> {
    let WorkflowOverlay {
        language,
        kind,
        workspace,
        draft,
        targets,
        selected,
        search,
        message,
    } = props;
    match kind {
        AgendaOverlay::Capture => Some(capture_overlay(language, workspace, draft, message)),
        AgendaOverlay::Refile => Some(refile_overlay(
            language, workspace, targets, selected, search, message,
        )),
        AgendaOverlay::Repeat => Some(repeat_overlay(language, workspace)),
        _ => None,
    }
}

fn repeat_overlay(
    language: crate::i18n::Language,
    workspace: Entity<WorkspaceWindow>,
) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .absolute()
        .inset_0()
        .bg(gpui::rgba(0x00000052))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(460.))
                .p_5()
                .rounded(px(12.))
                .bg(rgb(theme.background))
                .shadow_lg()
                .child(
                    div()
                        .text_size(px(19.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(language.text("agenda.complete_repeat")),
                )
                .child(
                    div()
                        .mt_2()
                        .text_size(px(11.))
                        .text_color(rgb(theme.foreground_dim))
                        .child(language.text("agenda.repeat_hint")),
                )
                .child(
                    div()
                        .mt_5()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(action(
                            workspace.clone(),
                            language.text("agenda.complete_occurrence"),
                            UiIntent::ResolveRepeat(
                                crate::agenda::RepeatCompletionAction::Occurrence,
                            ),
                            true,
                        ))
                        .child(action(
                            workspace.clone(),
                            language.text("agenda.complete_series"),
                            UiIntent::ResolveRepeat(crate::agenda::RepeatCompletionAction::Series),
                            false,
                        ))
                        .child(action(
                            workspace,
                            language.text("agenda.cancel"),
                            UiIntent::ResolveRepeat(crate::agenda::RepeatCompletionAction::Cancel),
                            false,
                        )),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Context, IntoElement, Modifiers, Render, Window};

    #[gpui::test]
    fn cancelling_refile_does_not_reopen_the_underlying_inbox_row(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, _| {
            workspace.agenda.state.overlay = AgendaOverlay::Refile;
            workspace.agenda.state.selected = Some(2);
        });
        struct Harness(Entity<WorkspaceWindow>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let workspace = self.0.clone();
                div()
                    .size_full()
                    .relative()
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        workspace.update(cx, |workspace, _| {
                            workspace.agenda.state.overlay = AgendaOverlay::Refile
                        });
                    })
                    .child(refile_overlay(
                        crate::i18n::Language::English,
                        self.0.clone(),
                        vec![],
                        0,
                        "",
                        None,
                    ))
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Harness(workspace.clone()));
        let bounds = cx
            .debug_bounds("agenda-workflow-Cancel")
            .expect("cancel is rendered");
        let center = bounds.center();
        cx.simulate_mouse_move(center, None, Modifiers::default());
        cx.simulate_click(center, Modifiers::default());
        workspace.update(cx, |workspace, _| {
            assert_eq!(workspace.agenda.state.overlay, AgendaOverlay::None);
            assert_eq!(workspace.agenda.state.selected, Some(2));
            assert!(workspace.agenda.state.refile_task.is_none());
        });
    }
}
