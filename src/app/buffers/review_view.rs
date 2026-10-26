use super::*;
use crate::{
    components::UiTextExt,
    typography::{ui_px, ui_size},
};
use gpui::{
    AnyElement, MouseButton, PathBuilder, Toggled, canvas, div, point, prelude::*, px, rgb,
};

const HEADER: f32 = 44.;
const FILE: f32 = 56.;
const LIST_HEADER: f32 = 32.;
const ROW: f32 = 48.;
const SUMMARY: f32 = 28.;
const ERROR: f32 = 40.;
const BUTTON: f32 = 36.;
const FOOTER_PADDING: f32 = 10.;

impl SaveReview {
    fn vertical_buttons(&self, available: f32) -> bool {
        available
            < ui_size(if self.single_confirmation() {
                320.
            } else {
                360.
            })
    }

    pub(super) fn panel_size(&self, available: f32, max_height: f32) -> (f32, f32) {
        let count = if self.single_confirmation() { 2. } else { 1. };
        let footer = if self.vertical_buttons(available) {
            count * BUTTON + (count - 1.) * 6. + 2. * FOOTER_PADDING
        } else {
            BUTTON + 2. * FOOTER_PADDING
        };
        let body = if self.single_confirmation() {
            FILE
        } else {
            LIST_HEADER + ROW * self.entries.len().min(6) as f32 + SUMMARY
        };
        (
            available.min(ui_size(if self.single_confirmation() {
                480.
            } else {
                580.
            })),
            (ui_size(HEADER + body + footer + if self.error.is_some() { ERROR } else { 0. }) + 2.)
                .min(max_height),
        )
    }
}

struct ReviewHint(gpui::SharedString);

impl gpui::Render for ReviewHint {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::current_theme();
        div()
            .max_w(px(520.))
            .p(px(8.))
            .rounded(px(6.))
            .shadow_md()
            .bg(rgb(theme.elevated))
            .border_1()
            .border_color(rgb(theme.border))
            .text_size(ui_px(12.))
            .text_color(rgb(theme.foreground))
            .child_text(self.0.clone())
    }
}

/// The reserved slot keeps button geometry stable as keyboard focus moves.
fn return_hint(focus: gpui::FocusHandle) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            if !focus.is_focused(window) {
                return;
            }
            let mut path = PathBuilder::stroke(ui_px(1.2));
            for (index, (x, y)) in [(10., 2.), (10., 7.), (2., 7.), (5., 4.)]
                .into_iter()
                .enumerate()
            {
                let p = point(bounds.origin.x + ui_px(x), bounds.origin.y + ui_px(y));
                if index == 0 {
                    path.move_to(p);
                } else {
                    path.line_to(p);
                }
            }
            path.move_to(point(
                bounds.origin.x + ui_px(2.),
                bounds.origin.y + ui_px(7.),
            ));
            path.line_to(point(
                bounds.origin.x + ui_px(5.),
                bounds.origin.y + ui_px(10.),
            ));
            if let Ok(path) = path.build() {
                crate::document_preview::path(window, path, window.text_style().color);
            }
        },
    )
    .size(ui_px(12.))
    .flex_none()
}

fn checkbox(state: Toggled, disabled: bool) -> gpui::Div {
    let theme = crate::theme::current_theme();
    let marked = state != Toggled::False;
    div()
        .size(ui_px(16.))
        .flex_none()
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(if marked {
            theme.accent
        } else {
            theme.border_hover
        }))
        .bg(rgb(if marked {
            theme.accent
        } else {
            theme.background
        }))
        .text_color(if marked {
            rgb(0xffffff)
        } else {
            rgb(theme.foreground)
        })
        .text_size(ui_px(12.))
        .flex()
        .items_center()
        .justify_center()
        .when(disabled, |s| s.opacity(0.5))
        .child_text(match state {
            Toggled::True => "✓",
            Toggled::Mixed => "−",
            Toggled::False => "",
        })
}

impl WorkspaceWindow {
    fn review_title(&self, kind: ReviewKind) -> &'static str {
        self.language.text(match kind {
            ReviewKind::Save => "buffer.review.title_save",
            ReviewKind::Close(_) => "buffer.review.title_close",
            ReviewKind::Quit => "buffer.review.title_quit",
            ReviewKind::Window => "buffer.review.title_window",
        })
    }

    fn review_submit_label(&self, review: &SaveReview) -> String {
        let (save, discard) = review.selection_counts();
        let key = if review.running {
            "buffer.review.saving"
        } else if matches!(review.kind, ReviewKind::Close(_)) {
            "buffer.review.save"
        } else if review.kind == ReviewKind::Save {
            if save > 0 {
                "buffer.review.save_selected"
            } else {
                "buffer.review.done"
            }
        } else if review.single_confirmation() {
            "buffer.review.save_exit"
        } else if save == 0 && discard > 0 {
            "buffer.review.discard_exit"
        } else if discard == 0 {
            "buffer.review.save_all_exit"
        } else {
            "buffer.review.save_selected_exit"
        };
        self.language.text(key).replace(
            "{action}",
            self.language.text(if review.kind == ReviewKind::Quit {
                "buffer.review.quit"
            } else {
                "buffer.review.close"
            }),
        )
    }

    pub(crate) fn review_buttons(
        &self,
        review: &SaveReview,
        vertical: bool,
        entity: Entity<Self>,
    ) -> gpui::Div {
        let theme = crate::theme::current_theme();
        let buttons = review
            .controls()
            .iter()
            .copied()
            .filter(|c| matches!(c, ReviewControl::Discard | ReviewControl::Submit))
            .map(|control| {
                let (id, label) = match control {
                    ReviewControl::Discard => (
                        "buffer-review-discard-all",
                        self.language.text("buffer.review.discard").to_owned(),
                    ),
                    ReviewControl::Submit => {
                        ("buffer-review-submit", self.review_submit_label(review))
                    }
                    _ => unreachable!(),
                };
                let disabled = review.running
                    || !matches!(
                        self.save.interaction,
                        crate::app::save::SaveInteraction::Idle
                    );
                let destructive = control == ReviewControl::Discard
                    || control == ReviewControl::Submit
                        && !review.single_confirmation()
                        && review.destructive_submit();
                let primary = control == ReviewControl::Submit && !destructive;
                let background = rgb(if primary {
                    theme.accent
                } else {
                    theme.background
                });
                let foreground = if primary {
                    rgb(0xffffff)
                } else {
                    rgb(if destructive {
                        theme.error
                    } else {
                        theme.foreground
                    })
                };
                let click_entity = entity.clone();
                let accessible_entity = entity.clone();
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .role(gpui::Role::Button)
                    .aria_label(label.clone())
                    .track_focus(review.focus(control))
                    .tab_index(0)
                    .h(ui_px(if review.inline { 28. } else { BUTTON }))
                    .flex_none()
                    .p(px(2.))
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .rounded(px(8.))
                    .focus(|s| s.border_color(rgb(theme.accent)))
                    .when(vertical, |s| s.w_full())
                    .when(disabled, |s| s.opacity(0.5))
                    .when(!disabled, |s| s.cursor_pointer())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        click_entity
                            .update(cx, |w, cx| w.buffer_review_action(control, window, cx));
                    })
                    .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                        cx.stop_propagation();
                        accessible_entity
                            .update(cx, |w, cx| w.buffer_review_action(control, window, cx));
                    })
                    .child(
                        div()
                            .size_full()
                            .px(ui_px(10.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(if primary {
                                background
                            } else {
                                rgb(theme.border)
                            })
                            .bg(background)
                            .text_color(foreground)
                            .text_size(ui_px(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(ui_px(6.))
                            .when(!disabled, |s| {
                                s.hover(move |s| s.bg(background.blend(foreground.opacity(0.08))))
                            })
                            .child_text(label)
                            .child(return_hint(review.focus(control).clone())),
                    )
            });
        div()
            .flex()
            .gap(ui_px(6.))
            .when(vertical, |s| s.flex_col().w_full())
            .when(!vertical, |s| s.items_center())
            .children(buttons)
    }

    fn review_file(
        &self,
        entry: &ReviewEntry,
        index: usize,
        entity: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let document = entry.session.read(cx);
        let theme = crate::theme::current_theme();
        let path = document.file_path();
        let detail = path
            .map(|path| {
                let parent = path.parent().unwrap_or(Path::new(""));
                std::env::var_os("HOME")
                    .and_then(|home| {
                        parent
                            .strip_prefix(home)
                            .ok()
                            .map(|p| format!("~/{}", p.display()))
                    })
                    .unwrap_or_else(|| parent.display().to_string())
            })
            .unwrap_or_else(|| self.language.text("buffer.review.save_location").to_owned());
        let hint: gpui::SharedString = format!(
            "{}\n{}",
            path.map(|p| p.display().to_string())
                .unwrap_or_else(|| document.display_name()),
            self.language.text("buffer.review.view_document"),
        )
        .into();
        let id = entry.id;
        let accessible_entity = entity.clone();
        div()
            .id(("buffer-review-document", index))
            .debug_selector(move || format!("buffer-review-document-{index}"))
            .role(gpui::Role::Button)
            .aria_label(document.display_name())
            .aria_description(hint.clone())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .justify_center()
            .when(!self.buffer_busy(), |s| s.cursor_pointer())
            .tooltip(move |_, cx| cx.new(|_| ReviewHint(hint.clone())).into())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                entity.update(cx, |w, cx| w.view_buffer_review_document(id, window, cx));
            })
            .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                accessible_entity.update(cx, |w, cx| w.view_buffer_review_document(id, window, cx));
            })
            .child(
                div()
                    .truncate()
                    .text_size(ui_px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(theme.foreground))
                    .hover(|s| s.text_color(rgb(theme.accent)))
                    .child_text(document.display_name()),
            )
            .child(
                div()
                    .truncate()
                    .text_size(ui_px(11.))
                    .text_color(rgb(theme.foreground_muted))
                    .child_text(detail),
            )
            .into_any_element()
    }

    pub(super) fn review_view(
        &self,
        review: &SaveReview,
        entity: Entity<Self>,
        available: f32,
        cx: &App,
    ) -> AnyElement {
        let theme = crate::theme::current_theme();
        let (save, discard) = review.selection_counts();
        let vertical = review.vertical_buttons(available);
        let mut body = div()
            .id("buffer-review")
            .debug_selector(|| "buffer-review".into())
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .role(gpui::Role::Dialog)
            .aria_label(self.review_title(review.kind))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(ui_px(HEADER))
                    .flex_none()
                    .px(ui_px(18.))
                    .flex()
                    .items_center()
                    .gap(ui_px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ui_px(14.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(theme.foreground))
                            .child_text(self.review_title(review.kind)),
                    )
                    .when(review.entries.len() > 1, |s| {
                        s.child(
                            div()
                                .flex_none()
                                .text_size(ui_px(11.))
                                .text_color(rgb(theme.foreground_muted))
                                .child_text(
                                    self.language
                                        .text("buffer.review.document_count")
                                        .replace("{count}", &review.entries.len().to_string()),
                                ),
                        )
                    }),
            );
        if review.single_confirmation() {
            body = body.child(
                div()
                    .id("buffer-review-single")
                    .flex_1()
                    .min_h_0()
                    .px(ui_px(18.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .h(ui_px(FILE))
                            .flex()
                            .items_center()
                            .child(self.review_file(&review.entries[0], 0, entity.clone(), cx)),
                    ),
            );
        } else {
            let state = if save == 0 {
                Toggled::False
            } else if discard == 0 {
                Toggled::True
            } else {
                Toggled::Mixed
            };
            let all_entity = entity.clone();
            let accessible_entity = entity.clone();
            body = body.child(
                div()
                    .id("buffer-review-all")
                    .debug_selector(|| "buffer-review-all".into())
                    .role(gpui::Role::CheckBox)
                    .aria_label(self.language.text("buffer.review.save_all"))
                    .aria_toggled(state)
                    .track_focus(review.focus(ReviewControl::All))
                    .tab_index(0)
                    .h(ui_px(LIST_HEADER))
                    .flex_none()
                    .mx(ui_px(14.))
                    .px(ui_px(4.))
                    .flex()
                    .items_center()
                    .gap(ui_px(10.))
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .rounded(px(6.))
                    .focus(|s| s.border_color(rgb(theme.accent)))
                    .when(!review.running, |s| s.cursor_pointer())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        all_entity.update(cx, |w, cx| {
                            w.buffer_review_action(ReviewControl::All, window, cx)
                        });
                    })
                    .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                        accessible_entity.update(cx, |w, cx| {
                            w.buffer_review_action(ReviewControl::All, window, cx)
                        });
                    })
                    .child(checkbox(state, review.running || save + discard == 0))
                    .child(
                        div()
                            .text_size(ui_px(12.))
                            .text_color(rgb(theme.foreground))
                            .child_text(self.language.text("buffer.review.save_all")),
                    ),
            );
            let rows = review.entries.iter().enumerate().map(|(index, entry)| {
                let choice_entity = entity.clone();
                let accessible_entity = entity.clone();
                let id = entry.id;
                let next = !entry.save;
                let state = if entry.save || entry.done {
                    Toggled::True
                } else {
                    Toggled::False
                };
                let choice = div()
                    .id(("buffer-review-choice", index))
                    .debug_selector(move || format!("buffer-review-choice-{index}"))
                    .role(gpui::Role::CheckBox)
                    .aria_toggled(state)
                    .aria_label(self.language.text("buffer.review.save"))
                    .size(ui_px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!review.running && !entry.done, |s| s.cursor_pointer())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        choice_entity.update(cx, |w, cx| {
                            if let Some(r) = w.buffers.review().filter(|r| !r.running) {
                                r.focus(ReviewControl::List).focus(window, cx);
                                w.set_buffer_review_choice(id, next, cx);
                            }
                        });
                    })
                    .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                        accessible_entity
                            .update(cx, |w, cx| w.set_buffer_review_choice(id, next, cx));
                    })
                    .child(checkbox(state, review.running || entry.done));
                div()
                    .id(("buffer-review-row", index))
                    .debug_selector(move || format!("buffer-review-row-{index}"))
                    .h(ui_px(ROW))
                    .flex_none()
                    .px(ui_px(4.))
                    .flex()
                    .items_center()
                    .gap(ui_px(6.))
                    .rounded(px(5.))
                    .when(
                        index == review.selected && !review.single_confirmation(),
                        |s| s.bg(rgb(theme.hover)),
                    )
                    .child(choice)
                    .child(self.review_file(entry, index, entity.clone(), cx))
                    .when(entry.done || !entry.save, |s| {
                        s.child(
                            div()
                                .flex_none()
                                .text_size(ui_px(11.))
                                .text_color(rgb(if entry.done || review.kind == ReviewKind::Save {
                                    theme.foreground_muted
                                } else {
                                    theme.error
                                }))
                                .child_text(self.language.text(if entry.done {
                                    "buffer.review.saved"
                                } else if review.kind == ReviewKind::Save {
                                    "buffer.review.skip"
                                } else {
                                    "buffer.review.discard"
                                })),
                        )
                    })
            });
            body = body.child(
                div()
                    .id("buffer-review-list")
                    .debug_selector(|| "buffer-review-list".into())
                    .role(gpui::Role::ListBox)
                    .aria_label(self.language.text("buffer.review.documents"))
                    .track_focus(review.focus(ReviewControl::List))
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .mx(ui_px(10.))
                    .px(ui_px(2.))
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .rounded(px(6.))
                    .focus(|s| s.border_color(rgb(theme.accent)))
                    .track_scroll(&self.buffers.scroll)
                    .overflow_y_scroll()
                    .children(rows),
            );
            body = body.child(
                div()
                    .h(ui_px(SUMMARY))
                    .flex_none()
                    .px(ui_px(18.))
                    .flex()
                    .items_center()
                    .text_size(ui_px(11.))
                    .text_color(rgb(theme.foreground_muted))
                    .child_text(
                        self.language
                            .text(if review.kind == ReviewKind::Save {
                                "buffer.review.summary_skip"
                            } else {
                                "buffer.review.summary"
                            })
                            .replace("{save}", &save.to_string())
                            .replace("{discard}", &discard.to_string()),
                    ),
            );
        }
        body.when_some(review.error.clone(), |s, error| {
            s.child(
                div()
                    .id("buffer-review-error")
                    .h(ui_px(ERROR))
                    .flex_none()
                    .px(ui_px(18.))
                    .debug_selector(|| "buffer-review-error".into())
                    .text_size(ui_px(11.))
                    .text_color(rgb(theme.error))
                    .line_clamp(2)
                    .tooltip({
                        let error = error.clone();
                        move |_, cx| cx.new(|_| ReviewHint(error.clone().into())).into()
                    })
                    .child_text(error),
            )
        })
        .child(
            div()
                .flex_none()
                .px(ui_px(14.))
                .py(ui_px(FOOTER_PADDING))
                .border_t_1()
                .border_color(rgb(theme.divider))
                .flex()
                .justify_end()
                .child(self.review_buttons(review, vertical, entity)),
        )
        .into_any_element()
    }
}
