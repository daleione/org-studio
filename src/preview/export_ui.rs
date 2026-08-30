use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
pub(crate) struct ExportHost {
    task: Option<gpui::Task<()>>,
    cancel: Option<Arc<AtomicBool>>,
    request: u64,
    panel: Option<ExportPanelState>,
    status: Option<ExportRunState>,
}

impl ExportHost {
    pub(super) fn panel(&self) -> Option<&ExportPanelState> {
        self.panel.as_ref()
    }

    pub(super) fn status(&self) -> Option<&ExportRunState> {
        self.status.as_ref()
    }

    pub(super) fn is_open(&self) -> bool {
        self.panel.is_some()
    }

    pub(super) fn clear_status(&mut self) {
        self.status = None;
    }
}

use gpui::{
    Context, Entity, Image, ImageFormat, IntoElement, MouseButton, ObjectFit, Task, div, img,
    prelude::*, px, rgb, rgba,
};

use crate::export::{
    ExportFormat, ExportOptions, ExportSourceFormat, LayoutMode, PaperSize, export_snapshot,
    shared_engine, themes, write_artifacts,
};
use crate::i18n::Language;

use super::{DocumentFormat, PreviewLoadState, WorkspaceWindow, current_theme};

#[derive(Clone)]
pub(super) struct ExportPanelState {
    pub(super) options: ExportOptions,
    pub(super) theme_index: usize,
}

impl Default for ExportPanelState {
    fn default() -> Self {
        Self {
            options: ExportOptions::default(),
            theme_index: themes()
                .iter()
                .position(|theme| theme.id == "minimal-blue")
                .unwrap_or(0),
        }
    }
}

#[derive(Clone)]
pub(super) enum ExportRunState {
    Working(Arc<str>),
    Success { message: Arc<str>, path: PathBuf },
    Error(Arc<str>),
}

impl WorkspaceWindow {
    pub(super) fn show_export_panel(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.state, PreviewLoadState::Ready { .. }) {
            return;
        }
        self.export.panel = Some(ExportPanelState::default());
        self.export.status = None;
        cx.notify();
    }

    pub(super) fn close_export_panel(&mut self, cx: &mut Context<Self>) {
        if matches!(self.export.status, Some(ExportRunState::Working(_))) {
            if let Some(cancel) = self.export.cancel.take() {
                cancel.store(true, Ordering::Release);
            }
            self.export.request = self.export.request.wrapping_add(1);
            self.export.task.take();
            let message = self.language.text("export.cancelled");
            self.export.status = Some(ExportRunState::Error(message.into()));
        } else {
            self.export.panel = None;
            self.export.status = None;
        }
        cx.notify();
    }

    pub(super) fn select_export_format(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        let Some(panel) = self.export.panel.as_mut() else {
            return;
        };
        panel.options.format = format;
        panel.options.layout = if format == ExportFormat::Pdf {
            LayoutMode::Paged
        } else {
            LayoutMode::Continuous
        };
        panel.options.paper = if format == ExportFormat::Pdf {
            PaperSize::A4
        } else {
            PaperSize::Theme
        };
        panel.options.per_page = false;
        self.export.status = None;
        cx.notify();
    }

    pub(super) fn select_export_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(panel) = self.export.panel.as_mut() else {
            return;
        };
        if index >= themes().len() {
            return;
        }
        panel.theme_index = index;
        panel.options.theme_id = themes()[panel.theme_index].id.into();
        self.export.status = None;
        cx.notify();
    }

    pub(super) fn select_export_paper(&mut self, paper: PaperSize, cx: &mut Context<Self>) {
        if let Some(panel) = self.export.panel.as_mut() {
            panel.options.paper = paper;
            panel.options.layout = LayoutMode::Paged;
            panel.options.per_page = false;
            self.export.status = None;
            cx.notify();
        }
    }

    pub(super) fn select_export_ppi(&mut self, ppi: f32, cx: &mut Context<Self>) {
        if let Some(panel) = self.export.panel.as_mut() {
            panel.options.png_ppi = ppi;
            self.export.status = None;
            cx.notify();
        }
    }

    pub(super) fn choose_export_destination(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.export.panel.as_ref() else {
            return;
        };
        let Some(document) = (match &self.state {
            PreviewLoadState::Ready { document } => {
                Some(document.panel.read(cx).document().clone())
            }
            _ => None,
        }) else {
            return;
        };
        let extension = panel.options.format.extension();
        let stem = document
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("document");
        let suggested = format!("{stem}.{extension}");
        let directory = document
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let receiver = cx.prompt_for_new_path(directory, Some(&suggested));
        self.picker_task = Some(cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut path))) = receiver.await {
                let _ = this.update(cx, |this, cx| {
                    if let Some(panel) = &this.export.panel {
                        path.set_extension(panel.options.format.extension());
                    }
                    this.start_export(path, cx)
                });
            }
        }));
    }

    fn start_export(&mut self, destination: PathBuf, cx: &mut Context<Self>) {
        let Some(panel) = self.export.panel.as_ref() else {
            return;
        };
        let Some(document) = (match &self.state {
            PreviewLoadState::Ready { document } => {
                Some(document.panel.read(cx).document().clone())
            }
            _ => None,
        }) else {
            return;
        };
        let options = panel.options.clone();
        let language = self.language;
        let source_format = match document.format {
            DocumentFormat::Org => ExportSourceFormat::Org,
            DocumentFormat::Markdown => ExportSourceFormat::Markdown,
        };
        self.export.request = self.export.request.wrapping_add(1);
        let request = self.export.request;
        let cancel = Arc::new(AtomicBool::new(false));
        self.export.cancel = Some(cancel.clone());
        self.export.status = Some(ExportRunState::Working(
            language.text("export.working").into(),
        ));
        cx.notify();

        let background: Task<Result<(Vec<PathBuf>, usize), String>> =
            cx.background_spawn(async move {
                let artifacts = export_snapshot(
                    shared_engine(),
                    document.text.as_ref(),
                    source_format,
                    &document.path,
                    &options,
                )
                .map_err(|error| error.to_string())?;
                if cancel.load(Ordering::Acquire) {
                    return Err("export cancelled".into());
                }
                let warning_count = artifacts.diagnostics.len();
                let paths = write_artifacts(&destination, options.format, &artifacts.files)
                    .map_err(|error| error.to_string())?;
                Ok((paths, warning_count))
            });
        self.export.task = Some(cx.spawn(async move |this, cx| {
            let result = background.await;
            let _ = this.update(cx, |this, cx| {
                if this.export.request != request {
                    return;
                }
                this.export.task = None;
                this.export.cancel = None;
                this.export.status = Some(match result {
                    Ok((paths, warnings)) => {
                        let path = paths[0].clone();
                        ExportRunState::Success {
                            message: export_success_message(language, paths.len(), warnings).into(),
                            path,
                        }
                    }
                    Err(message) => ExportRunState::Error(message.into()),
                });
                cx.notify();
            });
        }));
    }

    pub(super) fn reveal_export(&self, cx: &mut Context<Self>) {
        if let Some(ExportRunState::Success { path, .. }) = &self.export.status {
            cx.reveal_path(path);
        }
    }
}

fn export_success_message(language: Language, files: usize, warnings: usize) -> String {
    let mut message = format!(
        "{} · {} {}",
        language.text("export.complete"),
        files,
        language.text("export.files")
    );
    if warnings != 0 {
        message.push_str(&format!(
            ", {warnings} {}",
            language.text("export.warnings")
        ));
    }
    message
}

pub(super) fn render_export_panel(
    entity: Entity<WorkspaceWindow>,
    panel: ExportPanelState,
    status: Option<ExportRunState>,
    language: Language,
) -> impl IntoElement {
    let palette = current_theme();
    let working = matches!(status, Some(ExportRunState::Working(_)));
    let format_button = |label: &'static str, format: ExportFormat| {
        let selected = panel.options.format == format;
        let id = match format {
            ExportFormat::Pdf => 0usize,
            ExportFormat::Png => 1,
            ExportFormat::Svg => 2,
        };
        let button_entity = entity.clone();
        div()
            .id(("export-format", id))
            .h(px(36.0))
            .flex_1()
            .px_3()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.0))
            .border_1()
            .border_color(rgb(if selected { 0x3a81c3 } else { palette.border }))
            .bg(rgb(if selected {
                palette.background
            } else {
                palette.background_alt
            }))
            .when(selected, |button| button.shadow_sm())
            .hover(|style| style.border_color(rgb(0x8eb9df)))
            .cursor_pointer()
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                button_entity.update(cx, |this, cx| this.select_export_format(format, cx));
            })
    };
    let paper_button = |id: usize, label: &'static str, paper: PaperSize| {
        let selected = panel.options.paper == paper;
        let paper_entity = entity.clone();
        div()
            .id(("export-paper", id))
            .h(px(36.0))
            .flex_1()
            .px_3()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.0))
            .border_1()
            .border_color(rgb(if selected { 0x3a81c3 } else { palette.border }))
            .bg(rgb(if selected {
                palette.background
            } else {
                palette.background_alt
            }))
            .when(selected, |button| button.shadow_sm())
            .hover(|style| style.border_color(rgb(0x8eb9df)))
            .cursor_pointer()
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                paper_entity.update(cx, |this, cx| this.select_export_paper(paper, cx));
            })
    };
    let theme_grid = themes().iter().enumerate().fold(
        div().flex().flex_wrap().gap_3().pb_2(),
        |grid, (index, theme)| {
            let selected = panel.theme_index == index;
            let theme_entity = entity.clone();
            grid.child(
                div()
                    .id(("export-theme", index))
                    .w(px(168.0))
                    .flex_none()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(if selected { 0x3a81c3 } else { palette.border }))
                    .bg(rgb(palette.background))
                    .when(selected, |card| card.shadow_sm())
                    .hover(|style| style.border_color(rgb(0x8eb9df)))
                    .cursor_pointer()
                    .child(
                        div()
                            .h(px(190.0))
                            .w_full()
                            .overflow_hidden()
                            .bg(rgb(0xffffff))
                            .child(
                                img(Arc::new(Image::from_bytes(
                                    ImageFormat::Png,
                                    theme.thumbnail.to_vec(),
                                )))
                                .size_full()
                                .object_fit(ObjectFit::Cover),
                            ),
                    )
                    .child(
                        div()
                            .h(px(48.0))
                            .px_3()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(if selected {
                                        0x246ca9
                                    } else {
                                        palette.foreground
                                    }))
                                    .child(theme.name(language)),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(px(10.0))
                                    .text_color(rgb(palette.foreground_dim))
                                    .child(theme.family(language)),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        theme_entity.update(cx, |this, cx| this.select_export_theme(index, cx));
                    }),
            )
        },
    );
    let close_entity = entity.clone();
    let export_entity = entity.clone();
    let cancel_entity = entity.clone();
    let reveal_entity = entity.clone();
    let ppi_144_entity = entity.clone();
    let ppi_300_entity = entity.clone();

    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000066))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("export-panel-card")
                .w(px(580.0))
                .max_w_full()
                .max_h(px(740.0))
                .overflow_y_scroll()
                .rounded_lg()
                .border_1()
                .border_color(rgb(palette.border))
                .bg(rgb(palette.background))
                .shadow_lg()
                .p_5()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .child(
                                    div()
                                        .text_size(px(20.0))
                                        .child(language.text("export.title")),
                                )
                                .child(
                                    div()
                                        .mt_1()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.foreground_dim))
                                        .child(language.text("export.subtitle")),
                                ),
                        )
                        .child(
                            div()
                                .id("export-close")
                                .size(px(28.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .text_color(rgb(palette.foreground_dim))
                                .hover(|style| style.bg(rgb(palette.background_alt)))
                                .cursor_pointer()
                                .child("×")
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    close_entity.update(cx, |this, cx| this.close_export_panel(cx));
                                }),
                        ),
                )
                .child(
                    div()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(palette.foreground_dim))
                                .child(language.text("export.format")),
                        )
                        .child(
                            div()
                                .mt_2()
                                .p_1()
                                .flex()
                                .gap_1()
                                .rounded_lg()
                                .bg(rgb(palette.background_alt))
                                .child(format_button("PDF", ExportFormat::Pdf))
                                .child(format_button("PNG", ExportFormat::Png))
                                .child(format_button("SVG", ExportFormat::Svg)),
                        ),
                )
                .child(
                    div()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(palette.foreground_dim))
                                .child(if panel.options.format == ExportFormat::Pdf {
                                    language.text("export.paper")
                                } else {
                                    language.text("export.output")
                                }),
                        )
                        .child(
                            div()
                                .mt_2()
                                .p_1()
                                .flex()
                                .gap_1()
                                .rounded_lg()
                                .bg(rgb(palette.background_alt))
                                .when(panel.options.format == ExportFormat::Pdf, |row| {
                                    row.child(paper_button(0, "A4", PaperSize::A4))
                                        .child(paper_button(1, "A5", PaperSize::A5))
                                        .child(paper_button(2, "B5", PaperSize::B5))
                                })
                                .when(panel.options.format != ExportFormat::Pdf, |row| {
                                    row.child(
                                        div()
                                            .id("export-long-image")
                                            .h(px(36.0))
                                            .min_w(px(100.0))
                                            .px_4()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(7.0))
                                            .border_1()
                                            .border_color(rgb(0x3a81c3))
                                            .bg(rgb(palette.background))
                                            .shadow_sm()
                                            .child(language.text("export.long_image")),
                                    )
                                })
                                .when(panel.options.format == ExportFormat::Png, |row| {
                                    row.child(
                                        div()
                                            .ml_3()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_size(px(12.0))
                                                    .text_color(rgb(palette.foreground_dim))
                                                    .child(language.text("export.quality")),
                                            )
                                            .child(
                                                div()
                                                    .id("export-ppi-144")
                                                    .h(px(36.0))
                                                    .min_w(px(64.0))
                                                    .px_3()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded_md()
                                                    .border_1()
                                                    .border_color(rgb(
                                                        if panel.options.png_ppi == 144.0 {
                                                            0x3a81c3
                                                        } else {
                                                            palette.border
                                                        },
                                                    ))
                                                    .cursor_pointer()
                                                    .child(language.text("export.standard"))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        move |_, _, cx| {
                                                            ppi_144_entity.update(
                                                                cx,
                                                                |this, cx| {
                                                                    this.select_export_ppi(
                                                                        144.0, cx,
                                                                    )
                                                                },
                                                            );
                                                        },
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .id("export-ppi-300")
                                                    .h(px(36.0))
                                                    .min_w(px(64.0))
                                                    .px_3()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded_md()
                                                    .border_1()
                                                    .border_color(rgb(
                                                        if panel.options.png_ppi == 300.0 {
                                                            0x3a81c3
                                                        } else {
                                                            palette.border
                                                        },
                                                    ))
                                                    .cursor_pointer()
                                                    .child(language.text("export.high"))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        move |_, _, cx| {
                                                            ppi_300_entity.update(
                                                                cx,
                                                                |this, cx| {
                                                                    this.select_export_ppi(
                                                                        300.0, cx,
                                                                    )
                                                                },
                                                            );
                                                        },
                                                    ),
                                            ),
                                    )
                                }),
                        ),
                )
                .child(
                    div()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(rgb(palette.foreground_dim))
                                        .child(language.text("export.themes")),
                                )
                                .child(div().text_size(px(11.0)).text_color(rgb(0x3a81c3)).child(
                                    format!(
                                        "{} · {}",
                                        themes()[panel.theme_index].name(language),
                                        themes()[panel.theme_index].family(language)
                                    ),
                                )),
                        )
                        .child(
                            div()
                                .id("export-theme-grid")
                                .mt_2()
                                .h(px(292.0))
                                .p_1()
                                .overflow_y_scroll()
                                .child(theme_grid),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_t_1()
                        .border_color(rgb(palette.border))
                        .pt_4()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.foreground_dim))
                                .when(status.is_none(), |view| {
                                    view.child(language.text("export.snapshot_hint"))
                                })
                                .when_some(status.clone(), |view, status| {
                                    let (message, color) = match status {
                                        ExportRunState::Working(message) => (message, 0x656d76),
                                        ExportRunState::Success { message, .. } => {
                                            (message, 0x2d7d46)
                                        }
                                        ExportRunState::Error(message) => (message, 0xb42318),
                                    };
                                    view.text_color(rgb(color)).child(message.to_string())
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(
                                    matches!(status, Some(ExportRunState::Success { .. })),
                                    |view| {
                                        view.child(
                                            div()
                                                .id("export-reveal")
                                                .h(px(38.0))
                                                .px_4()
                                                .flex()
                                                .items_center()
                                                .rounded_md()
                                                .border_1()
                                                .border_color(rgb(palette.border))
                                                .hover(|style| {
                                                    style.bg(rgb(palette.background_alt))
                                                })
                                                .cursor_pointer()
                                                .child(language.text("export.reveal"))
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    move |_, _, cx| {
                                                        reveal_entity.update(cx, |this, cx| {
                                                            this.reveal_export(cx)
                                                        });
                                                    },
                                                ),
                                        )
                                    },
                                )
                                .when(working, |view| {
                                    view.child(
                                        div()
                                            .id("export-cancel")
                                            .h(px(38.0))
                                            .px_4()
                                            .flex()
                                            .items_center()
                                            .rounded_md()
                                            .border_1()
                                            .border_color(rgb(palette.border))
                                            .cursor_pointer()
                                            .child(language.text("export.cancel"))
                                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                                cancel_entity.update(cx, |this, cx| {
                                                    this.close_export_panel(cx)
                                                });
                                            }),
                                    )
                                })
                                .child(
                                    div()
                                        .id("export-confirm")
                                        .h(px(38.0))
                                        .min_w(px(88.0))
                                        .px_5()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_md()
                                        .bg(rgb(if working { 0xaeb4ba } else { 0x3a81c3 }))
                                        .text_color(rgb(0xffffff))
                                        .when(!working, |button| {
                                            button.cursor_pointer().on_mouse_down(
                                                MouseButton::Left,
                                                move |_, _, cx| {
                                                    export_entity.update(cx, |this, cx| {
                                                        this.choose_export_destination(cx)
                                                    });
                                                },
                                            )
                                        })
                                        .child(if working {
                                            language.text("export.working_action")
                                        } else {
                                            language.text("export.action")
                                        }),
                                ),
                        ),
                ),
        )
}
