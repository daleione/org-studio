use super::*;
use crate::{
    components::UiTextExt,
    typography::{ui_font, ui_px},
};
use gpui::{IntoElement, MouseButton, Role, div, prelude::*, px, rgb};

impl WorkspaceWindow {
    pub(crate) fn render_code_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let prompt = self.babel_confirmation.as_ref()?;
        if self.content_route != ContentRoute::Document
            || !self.document_session().is_some_and(|session| {
                let document = session.read(cx);
                document.id() == prompt.document && document.revision() == prompt.revision
            })
        {
            self.answer_code_confirmation(false, window, cx);
            return None;
        }
        let theme = crate::theme::current_theme();
        let dark = crate::theme::effective_theme_is_dark();
        let background = if dark {
            theme.elevated
        } else {
            theme.background
        };
        let border = if dark {
            theme.border_hover
        } else {
            theme.border
        };
        let width = (window.viewport_size().width - px(32.))
            .max(px(1.))
            .min(px(600.));
        let compact = width < px(520.);
        let title = self.buffer_text("运行这段代码？", "Run this code?");
        let name = prompt.name.clone().unwrap_or_else(|| {
            format!(
                "{} {}",
                prompt.language,
                self.buffer_text("代码块", "code block")
            )
        });
        let source_name = prompt
            .origin
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let directory = short_directory(
            &prompt.directory,
            &prompt.document_path,
            self.buffer_text("文档所在目录", "Document directory"),
        );
        let note = if prompt.ask_each_time {
            self.buffer_text(
                "此文档要求每次运行前确认。",
                "This document asks before every run.",
            )
        } else {
            self.buffer_text(
                "本次会话内记住已确认的来源",
                "Remember approved sources for this session.",
            )
        };
        let scope = if prompt.has_scripts {
            self.buffer_text(
                "代码在本机运行，可以修改文件。",
                "Code runs locally and can change files.",
            )
        } else {
            self.buffer_text(
                "数据库查询使用只读事务。",
                "Database queries use read-only transactions.",
            )
        };
        let dependency_count = prompt.dependencies.iter().map(|d| d.count).sum::<usize>();
        let description = if dependency_count == 0 && !prompt.writes_result {
            self.buffer_text(
                "仅运行代码，本次不写回结果。",
                "Run the code without writing results to the document.",
            )
            .to_owned()
        } else if dependency_count == 0 {
            self.buffer_text(
                "执行后，结果将写回当前文档。",
                "Results will be written to the current document.",
            )
            .to_owned()
        } else {
            match self.language {
                crate::i18n::Language::Chinese => {
                    format!("先执行 {dependency_count} 个依赖代码块，再运行此代码。")
                }
                crate::i18n::Language::English => {
                    format!("Run {dependency_count} dependency block(s), then this code.")
                }
            }
        };
        let buttons = [
            (
                "code-confirm-cancel",
                self.buffer_text("取消", "Cancel"),
                false,
            ),
            (
                "code-confirm-run",
                self.buffer_text("运行代码", "Run Code"),
                true,
            ),
        ]
        .into_iter()
        .zip(&prompt.focuses)
        .map(|((id, label, approved), focus)| {
            let entity = cx.entity();
            div()
                .id(id)
                .debug_selector(move || id.into())
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
                .border_color(rgb(if approved { theme.accent } else { border }))
                .bg(rgb(if approved { theme.accent } else { background }))
                .text_color(rgb(if approved { 0xffffff } else { theme.foreground }))
                .font_weight(gpui::FontWeight::MEDIUM)
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
                    this.answer_code_confirmation(approved, window, cx)
                }))
                .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.answer_code_confirmation(approved, window, cx)
                    })
                })
        });
        let preview = div()
            .id("code-confirm-preview")
            .debug_selector(|| "code-confirm-preview".into())
            .flex_none()
            .min_w_0()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(theme.border))
            .overflow_hidden()
            .bg(rgb(if dark {
                theme.background_alt
            } else {
                theme.surface
            }))
            .child(
                div()
                    .px(px(14.))
                    .py(px(11.))
                    .border_b_1()
                    .border_color(rgb(theme.divider))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                value("code-confirm-name", name.clone(), name)
                                    .text_size(ui_px(14.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD),
                            )
                            .child(
                                value(
                                    "code-confirm-origin",
                                    source_name,
                                    prompt.origin.to_string_lossy().into_owned(),
                                )
                                .text_size(ui_px(11.))
                                .text_color(rgb(theme.foreground_muted)),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(px(5.))
                            .bg(rgb(theme.accent).opacity(0.1))
                            .text_color(rgb(theme.accent))
                            .text_size(ui_px(11.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child_text(prompt.language),
                    ),
            )
            .child(
                div()
                    .px(px(14.))
                    .py(px(12.))
                    .font(ui_font("Menlo"))
                    .text_size(ui_px(12.))
                    .line_height(ui_px(21.))
                    .text_color(rgb(theme.code_foreground))
                    .children(prompt.preview.iter().enumerate().map(|(index, line)| {
                        div()
                            .flex()
                            .gap(px(12.))
                            .min_w_0()
                            .child(
                                div()
                                    .w(ui_px(16.))
                                    .flex_none()
                                    .text_right()
                                    .text_color(rgb(theme.foreground_dim))
                                    .child_text((index + 1).to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .whitespace_nowrap()
                                    .child_text(if line.is_empty() {
                                        " ".to_owned()
                                    } else {
                                        line.clone()
                                    }),
                            )
                    }))
                    .when(prompt.line_count > prompt.preview.len(), |preview| {
                        preview.child(
                            div()
                                .mt(px(5.))
                                .pl(ui_px(28.))
                                .text_size(ui_px(11.))
                                .text_color(rgb(theme.foreground_muted))
                                .child_text(match self.language {
                                    crate::i18n::Language::Chinese => {
                                        format!("… 共 {} 行，将执行完整代码", prompt.line_count)
                                    }
                                    crate::i18n::Language::English => format!(
                                        "… {} lines; the full block will run",
                                        prompt.line_count
                                    ),
                                }),
                        )
                    }),
            );
        let dependencies = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_size(ui_px(11.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(theme.foreground_muted))
                    .child_text(self.buffer_text("同时执行的依赖", "Dependencies to run")),
            )
            .child(
                div()
                    .id("code-confirm-dependencies")
                    .debug_selector(|| "code-confirm-dependencies".into())
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(
                        prompt
                            .dependencies
                            .iter()
                            .enumerate()
                            .map(|(i, dependency)| {
                                div()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .flex_none()
                                            .size(px(5.))
                                            .rounded_full()
                                            .bg(rgb(theme.accent).opacity(0.6)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap(px(1.))
                                            .child(
                                                value(
                                                    ("code-confirm-dependency-name", i),
                                                    dependency.name.clone(),
                                                    dependency.hint.clone(),
                                                )
                                                .text_size(ui_px(12.))
                                                .font_weight(gpui::FontWeight::MEDIUM),
                                            )
                                            .child(
                                                value(
                                                    ("code-confirm-dependency-detail", i),
                                                    dependency.detail.clone(),
                                                    dependency.hint.clone(),
                                                )
                                                .text_size(ui_px(11.))
                                                .text_color(rgb(theme.foreground_muted)),
                                            ),
                                    )
                                    .when(dependency.count > 1, |row| {
                                        row.child(
                                            div()
                                                .flex_none()
                                                .text_size(ui_px(11.))
                                                .text_color(rgb(theme.foreground_muted))
                                                .child_text(format!("×{}", dependency.count)),
                                        )
                                    })
                            }),
                    ),
            );
        let context = div()
            .flex_none()
            .border_t_1()
            .border_color(rgb(theme.divider))
            .pt(px(13.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(metadata(
                "code-confirm-document",
                self.buffer_text("结果", "Results"),
                if prompt.writes_result {
                    prompt.document_name.clone()
                } else {
                    self.buffer_text("不写回文档", "Do not write to document")
                        .to_owned()
                },
                if prompt.writes_result {
                    prompt.document_path.to_string_lossy().into_owned()
                } else {
                    self.buffer_text(
                        "此代码块使用 silent / none 结果模式",
                        "This block uses silent / none results",
                    )
                    .to_owned()
                },
            ))
            .child(metadata(
                "code-confirm-directory",
                self.buffer_text("运行目录", "Directory"),
                directory,
                prompt.directory.to_string_lossy().into_owned(),
            ))
            .child(
                div()
                    .text_size(ui_px(11.))
                    .text_color(rgb(theme.foreground_muted))
                    .child_text(scope),
            );
        let header = div()
            .flex_none()
            .flex()
            .items_start()
            .gap(px(12.))
            .child(
                div()
                    .flex_none()
                    .size(ui_px(40.))
                    .rounded(px(10.))
                    .bg(rgb(theme.accent).opacity(0.08))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        gpui::svg()
                            .data(include_bytes!(
                                "../../../../assets/icons/agenda/file-code.svg"
                            ))
                            .size(ui_px(24.))
                            .text_color(rgb(theme.accent)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(ui_px(22.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child_text(title),
                    )
                    .child(
                        div()
                            .text_size(ui_px(12.))
                            .text_color(rgb(theme.foreground_muted))
                            .child_text(description),
                    ),
            );
        let body = div()
            .id("code-confirm-body")
            .min_h_0()
            .flex_shrink(1.)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(preview)
            .when(!prompt.dependencies.is_empty(), |body| {
                body.child(dependencies)
            })
            .child(context);
        let footer = div()
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
                    .text_color(rgb(theme.foreground_muted))
                    .child_text(note),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .children(buttons),
            );
        Some(
            div()
                .id("code-execution-confirmation")
                .debug_selector(|| "code-execution-confirmation".into())
                .absolute()
                .inset_0()
                .occlude()
                .cursor_default()
                .bg(gpui::black().opacity(if dark { 0.5 } else { 0.16 }))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("code-execution-dialog")
                        .debug_selector(|| "code-execution-dialog".into())
                        .role(Role::Dialog)
                        .aria_label(title)
                        .w(width)
                        .max_h((window.viewport_size().height - px(32.)).max(px(1.)))
                        .p(px(if compact { 20. } else { 26. }))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(rgb(border))
                        .bg(rgb(background))
                        .shadow_lg()
                        .overflow_hidden()
                        .font(ui_font(".SystemUIFont"))
                        .text_size(ui_px(13.))
                        .line_height(ui_px(20.))
                        .text_color(rgb(theme.foreground))
                        .flex()
                        .flex_col()
                        .gap(px(20.))
                        .child(header)
                        .child(body)
                        .child(footer),
                )
                .into_any_element(),
        )
    }
}

fn metadata(id: &'static str, label: &'static str, text: String, hint: String) -> gpui::Div {
    let theme = crate::theme::current_theme();
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap(px(12.))
        .text_size(ui_px(12.))
        .child(
            div()
                .flex_none()
                .w(ui_px(54.))
                .text_color(rgb(theme.foreground_muted))
                .child_text(label),
        )
        .child(
            value(id, text, hint)
                .debug_selector(move || id.into())
                .flex_1(),
        )
}

fn value(id: impl Into<gpui::ElementId>, text: String, hint: String) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .min_w_0()
        .truncate()
        .tooltip(move |_, cx| cx.new(|_| ValueHint(hint.clone().into())).into())
        .child_text(text)
}

struct ValueHint(gpui::SharedString);
impl gpui::Render for ValueHint {
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
