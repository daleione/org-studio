use super::*;
use crate::app::status_line::{
    self,
    shell::{ShellKind, ShellRequest, ShellShape},
};
use gpui::{AnyElement, MouseButton, div, prelude::*, px, rgb};

const INPUT_HEIGHT: f32 = 46.;
const DETAIL_HEIGHT: f32 = 26.;
const LIST_INSET: f32 = 6.;
const SHELL_BORDER: f32 = 1.;
const MAX_COMMAND_WIDTH: f32 = 600.;

impl WorkspaceWindow {
    pub(crate) fn command_shell_request(&mut self, window: &Window) -> Option<ShellRequest> {
        let s = self.command_line.session.as_mut()?;
        s.visible_limit = ((f32::from(window.viewport_size().height) - 252.) / CANDIDATE_ROW_HEIGHT)
            .floor()
            .clamp(1., 6.) as usize;
        let pane = s.pane;
        let returning = s.returning;
        let expanded = matches!(s.phase, Phase::Input);
        let rows = s.candidates.len().clamp(1, s.visible_limit);
        let available = (self.document_pane_width(f32::from(window.viewport_size().width), pane)
            - 2. * status_line::FLOATING_STATUS_INSET)
            .max(1.);
        let height = if expanded {
            INPUT_HEIGHT
                + DETAIL_HEIGHT
                + rows as f32 * CANDIDATE_ROW_HEIGHT
                + 2. * LIST_INSET
                + 2. * SHELL_BORDER
        } else {
            status_line::FLOATING_STATUS_HEIGHT
        };
        Some(ShellRequest {
            kind: ShellKind::Command,
            pane,
            available,
            returning,
            target: if returning {
                ShellShape::status(available)
            } else {
                ShellShape {
                    width: available.min(MAX_COMMAND_WIDTH),
                    height,
                    expansion_height: height - status_line::FLOATING_STATUS_HEIGHT,
                    content_opacity: 1.,
                }
            },
        })
    }

    pub(crate) fn finish_command_return(&mut self, retained: bool) {
        if !retained
            && self
                .command_line
                .session
                .as_ref()
                .is_some_and(|s| s.returning)
        {
            self.command_line.session = None;
        }
    }

    pub(crate) fn command_bar(
        &self,
        pane: PaneSide,
        width: f32,
        status: Option<AnyElement>,
        cx: &gpui::App,
        entity: Entity<Self>,
    ) -> Option<AnyElement> {
        if !self.status.shell_owns(ShellKind::Command, pane) {
            return None;
        }
        let s = self.command_line.session.as_ref()?;
        let available = (width - 2. * status_line::FLOATING_STATUS_INSET).max(1.);
        let shape = self.status.sample_shell(available, Instant::now());
        let theme = crate::theme::current_theme();
        let narrow = shape.width < 430.;
        let scroll = entity.clone();
        let close_button = close_button(entity.clone());
        let content = match &s.phase {
            Phase::Input => self.command_input(s, narrow, entity.clone()),
            Phase::Running(started) => div()
                .size_full()
                .px(px(12.))
                .gap(px(10.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(rgb(theme.foreground_dim))
                .child(div().text_color(rgb(theme.accent)).child("◌"))
                .child(div().flex_1().child(if started.elapsed() >= RUNNING_DELAY {
                    self.command_text("正在对齐表格…", "Aligning tables…")
                } else {
                    "table-align"
                }))
                .child(close_button)
                .into_any_element(),
            Phase::Result { message, undo, .. } => {
                let can_undo = undo.is_some_and(|revision| {
                    self.document_session().is_some_and(|doc| {
                        doc.read(cx).id() == s.document && doc.read(cx).revision() == revision
                    })
                });
                let undo_entity = entity.clone();
                div()
                    .size_full()
                    .px(px(12.))
                    .gap(px(8.))
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(rgb(theme.foreground))
                    .child(div().text_color(rgb(theme.success)).child("✓"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .child(message.clone()),
                    )
                    .when(can_undo, |row| {
                        row.child(
                            div()
                                .id("command-line-undo")
                                .debug_selector(|| "command-line-undo".into())
                                .px(px(7.))
                                .h(px(28.))
                                .flex()
                                .items_center()
                                .text_color(rgb(theme.accent))
                                .cursor_pointer()
                                .child(self.command_text("撤销", "Undo"))
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    cx.stop_propagation();
                                    undo_entity.update(cx, |w, cx| w.undo_command_result(cx));
                                }),
                        )
                    })
                    .child(close_button)
                    .into_any_element()
            }
        };
        Some(
            status_line::floating_status_container(shape.height)
                .id("command-line-shell")
                .debug_selector(|| "command-line-shell".into())
                .bg(rgb(theme.elevated))
                .border_color(rgb(theme.border))
                .occlude()
                .on_scroll_wheel(move |event, _, cx| {
                    cx.stop_propagation();
                    let pixels = -f32::from(event.delta.pixel_delta(px(CANDIDATE_ROW_HEIGHT)).y);
                    scroll.update(cx, |w, cx| {
                        if let Some(s) = w.command_line.session.as_mut()
                            && !s.returning
                            && matches!(s.phase, Phase::Input)
                            && !s.candidates.is_empty()
                            && s.scroll_candidates(pixels, event.touch_phase)
                        {
                            cx.notify();
                        }
                    });
                })
                .left(px((width - shape.width) / 2.))
                .right(gpui::auto())
                .w(px(shape.width))
                .child(
                    div()
                        // Absolute offsets already start inside the parent's border. Avoid a
                        // square opaque background covering the outline and rounded corners.
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .rounded(px(10. - SHELL_BORDER))
                        .overflow_hidden()
                        .opacity(shape.content_opacity)
                        .child(content),
                )
                .when(shape.content_opacity < 1., |shell| {
                    shell.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(px(status_line::FLOATING_STATUS_HEIGHT - 2. * SHELL_BORDER))
                            .rounded(px(10. - SHELL_BORDER))
                            .overflow_hidden()
                            .opacity(1. - shape.content_opacity)
                            .children(status),
                    )
                })
                .into_any_element(),
        )
    }

    fn command_input(&self, s: &Session, narrow: bool, entity: Entity<Self>) -> AnyElement {
        let theme = crate::theme::current_theme();
        let begin = s.visible_start();
        let rows = s
            .candidates
            .iter()
            .enumerate()
            .skip(begin)
            .take(s.visible_limit)
            .map(|(index, entry)| candidate_row(s, index, entry, entity.clone()))
            .collect::<Vec<_>>();
        let detail = s
            .error
            .clone()
            .or_else(|| {
                s.candidates
                    .get(s.selected)
                    .map(|entry| entry.description.clone())
            })
            .unwrap_or_else(|| {
                self.command_text("没有匹配的命令", "No matching command")
                    .into()
            });
        let execute = entity.clone();
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .absolute()
                    .bottom(px(INPUT_HEIGHT + DETAIL_HEIGHT))
                    .left(px(LIST_INSET))
                    .right(px(LIST_INSET))
                    .py(px(LIST_INSET))
                    .flex()
                    .flex_col()
                    .children(rows)
                    .when(s.candidates.is_empty(), |view| {
                        view.child(
                            div()
                                .h(px(CANDIDATE_ROW_HEIGHT))
                                .flex()
                                .items_center()
                                .px(px(10.))
                                .text_size(px(12.))
                                .text_color(rgb(theme.foreground_muted))
                                .child(self.command_text("没有匹配的命令", "No matching command")),
                        )
                    }),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(INPUT_HEIGHT))
                    .w_full()
                    .h(px(DETAIL_HEIGHT))
                    .border_t_1()
                    .border_color(rgb(theme.border))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .text_size(px(10.))
                    .text_color(rgb(if s.error.is_some() {
                        theme.error
                    } else {
                        theme.foreground_dim
                    }))
                    .overflow_hidden()
                    .child(div().text_ellipsis().child(detail)),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .w_full()
                    .h(px(INPUT_HEIGHT))
                    .px(px(9.))
                    .gap(px(6.))
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(theme.border))
                    .child(
                        div()
                            .w(px(18.))
                            .text_size(px(20.))
                            .text_color(rgb(theme.accent))
                            .child(":"),
                    )
                    .child(div().flex_1().min_w_0().child(s.input.clone()))
                    .when(!narrow, |row| {
                        row.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(theme.foreground_dim))
                                .child(format!(
                                    "⌃1–{} · Tab",
                                    s.candidates.len().min(s.visible_limit).max(1)
                                )),
                        )
                    })
                    .child(
                        div()
                            .id("command-line-execute")
                            .debug_selector(|| "command-line-execute".into())
                            .px(px(8.))
                            .h(px(28.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .text_size(px(11.))
                            .text_color(rgb(theme.accent))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(theme.hover)))
                            .child("↵")
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                execute.update(cx, |w, cx| {
                                    w.command_input_key("enter", Modifiers::default(), cx);
                                });
                            }),
                    )
                    .child(close_button(entity)),
            )
            .into_any_element()
    }
}

fn candidate_row(
    s: &Session,
    index: usize,
    entry: &Entry,
    entity: Entity<WorkspaceWindow>,
) -> impl IntoElement {
    let theme = crate::theme::current_theme();
    let begin = s.visible_start();
    let chosen = index == s.selected;
    let select = entity;
    div()
        .id(("command-candidate", index))
        .debug_selector(move || format!("command-candidate-{index}"))
        .h(px(CANDIDATE_ROW_HEIGHT))
        .flex_none()
        .w_full()
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(5.))
        .bg(rgb(if chosen {
            theme.selected
        } else {
            theme.elevated
        }))
        .text_color(rgb(theme.foreground))
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(if chosen { theme.selected } else { theme.hover })))
        .child(
            div()
                .debug_selector(move || format!("command-candidate-number-{}", index - begin + 1))
                .w(px(18.))
                .flex_none()
                .font_family("Menlo")
                .text_size(px(12.))
                .text_color(rgb(theme.accent))
                .child((index - begin + 1).to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family("Menlo")
                .text_size(px(13.))
                .text_ellipsis()
                .child(entry.input.clone()),
        )
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            select.update(cx, |w, cx| {
                w.activate_command_candidate(index, cx);
            });
        })
}

fn close_button(entity: Entity<WorkspaceWindow>) -> impl IntoElement {
    let theme = crate::theme::current_theme();
    let close = entity;
    div()
        .id("command-line-close")
        .debug_selector(|| "command-line-close".into())
        .w(px(28.))
        .h(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .text_color(rgb(theme.foreground_dim))
        .hover(|s| s.bg(rgb(theme.hover)))
        .child("×")
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            close.update(cx, |w, cx| w.close_command_line(cx));
        })
}
