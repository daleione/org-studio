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
    pub(crate) fn panel(&self) -> Option<&ExportPanelState> {
        self.panel.as_ref()
    }

    pub(crate) fn status(&self) -> Option<&ExportRunState> {
        self.status.as_ref()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.panel.is_some()
    }

    pub(crate) fn clear_status(&mut self) {
        self.status = None;
    }
}

use gpui::{
    Context, Entity, Image, ImageFormat, IntoElement, MouseButton, ObjectFit, Task, div, img,
    prelude::*, px, rgb, rgba,
};

use crate::export::{
    ExportFormat, ExportOptions, ExportSourceFormat, LayoutMode, Orientation, PaperSize,
    export_snapshot, export_templates, shared_engine, write_artifacts,
};
use crate::i18n::Language;

use crate::{
    app::{WorkspaceLoadState, WorkspaceWindow},
    preview::DocumentFormat,
    theme::current_theme,
};

#[derive(Clone)]
pub(crate) struct ExportPanelState {
    pub(crate) options: ExportOptions,
    pub(crate) template_index: usize,
}

impl Default for ExportPanelState {
    fn default() -> Self {
        Self {
            options: ExportOptions::default(),
            template_index: export_templates()
                .iter()
                .position(|template| template.id == "minimal-blue")
                .unwrap_or(0),
        }
    }
}

#[derive(Clone)]
pub(crate) enum ExportRunState {
    Working(Arc<str>),
    Success { message: Arc<str>, path: PathBuf },
    Error(Arc<str>),
}

impl WorkspaceWindow {
    pub(crate) fn current_export_source(
        &self,
        cx: &gpui::App,
    ) -> Option<(crate::document::DocumentSnapshot, PathBuf, DocumentFormat)> {
        self.state.ready().map(|document| {
            let session = document.session.read(cx);
            let path = session.path().to_path_buf();
            let format = DocumentFormat::from_path(&path);
            (session.snapshot(), path, format)
        })
    }

    pub(crate) fn show_export_panel(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.state, WorkspaceLoadState::Ready { .. }) {
            return;
        }
        self.export.panel = Some(ExportPanelState::default());
        self.export.status = None;
        cx.notify();
    }

    pub(crate) fn close_export_panel(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn select_export_format(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
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
            PaperSize::TemplateDefault
        };
        panel.options.per_page = false;
        self.export.status = None;
        cx.notify();
    }

    pub(crate) fn select_export_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(panel) = self.export.panel.as_mut() else {
            return;
        };
        if index >= export_templates().len() {
            return;
        }
        panel.template_index = index;
        panel.options.template_id = export_templates()[panel.template_index].id.into();
        self.export.status = None;
        cx.notify();
    }

    pub(crate) fn select_export_paper(&mut self, paper: PaperSize, cx: &mut Context<Self>) {
        if let Some(panel) = self.export.panel.as_mut() {
            panel.options.paper = paper;
            panel.options.layout = LayoutMode::Paged;
            panel.options.per_page = false;
            self.export.status = None;
            cx.notify();
        }
    }

    pub(crate) fn select_export_orientation(
        &mut self,
        orientation: Orientation,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.export.panel.as_mut() {
            panel.options.orientation = orientation;
            self.export.status = None;
            cx.notify();
        }
    }

    pub(crate) fn select_export_ppi(&mut self, ppi: f32, cx: &mut Context<Self>) {
        if let Some(panel) = self.export.panel.as_mut() {
            panel.options.png_ppi = ppi;
            self.export.status = None;
            cx.notify();
        }
    }

    pub(crate) fn choose_export_destination(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.export.panel.as_ref() else {
            return;
        };
        let Some((_, path, _)) = self.current_export_source(cx) else {
            return;
        };
        let extension = panel.options.format.extension();
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("document");
        let suggested = format!("{stem}.{extension}");
        let directory = path.parent().unwrap_or_else(|| std::path::Path::new("."));
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
        let Some((snapshot, path, source_document_format)) = self.current_export_source(cx) else {
            return;
        };
        let options = panel.options.clone();
        let language = self.language;
        let source_format = match source_document_format {
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
                let artifacts =
                    export_snapshot(shared_engine(), &snapshot, source_format, &path, &options)
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

    pub(crate) fn reveal_export(&self, cx: &mut Context<Self>) {
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

pub(crate) fn render_export_panel(
    entity: Entity<WorkspaceWindow>,
    panel: ExportPanelState,
    status: Option<ExportRunState>,
    language: Language,
) -> impl IntoElement {
    let palette = current_theme();
    let working = matches!(status.as_ref(), Some(ExportRunState::Working(_)));
    let selected_template = &export_templates()[panel.template_index];

    let section_label = |label: &'static str| {
        div()
            .text_size(px(12.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(palette.foreground))
            .child(label)
    };

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
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(if selected {
                palette.accent
            } else {
                palette.background_alt
            }))
            .bg(rgb(if selected {
                palette.background
            } else {
                palette.background_alt
            }))
            .text_color(rgb(if selected {
                palette.accent
            } else {
                palette.foreground
            }))
            .font_weight(if selected {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .when(selected, |button| button.shadow_sm())
            .hover(|style| style.border_color(rgb(palette.accent_border)))
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
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(if selected {
                palette.accent
            } else {
                palette.border
            }))
            .bg(rgb(palette.background))
            .text_color(rgb(if selected {
                palette.accent
            } else {
                palette.foreground
            }))
            .when(selected, |button| button.shadow_sm())
            .hover(|style| style.border_color(rgb(palette.accent_border)))
            .cursor_pointer()
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                paper_entity.update(cx, |this, cx| this.select_export_paper(paper, cx));
            })
    };

    let orientation_button = |id: usize, label: &'static str, orientation: Orientation| {
        let selected = panel.options.orientation == orientation
            || (orientation == Orientation::Portrait
                && panel.options.orientation == Orientation::TemplateDefault);
        let orientation_entity = entity.clone();
        div()
            .id(("export-orientation", id))
            .h(px(36.0))
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(if selected {
                palette.accent
            } else {
                palette.border
            }))
            .bg(rgb(palette.background))
            .text_color(rgb(if selected {
                palette.accent
            } else {
                palette.foreground
            }))
            .when(selected, |button| button.shadow_sm())
            .hover(|style| style.border_color(rgb(palette.accent_border)))
            .cursor_pointer()
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                orientation_entity.update(cx, |this, cx| {
                    this.select_export_orientation(orientation, cx)
                });
            })
    };

    let theme_grid = export_templates().iter().enumerate().fold(
        div().flex().flex_wrap().gap_2().pb_2(),
        |grid, (index, template)| {
            let selected = panel.template_index == index;
            let theme_entity = entity.clone();
            grid.child(
                div()
                    .id(("export-template", index))
                    .w(px(104.0))
                    .flex_none()
                    .when(selected, |card| card.border_2())
                    .when(!selected, |card| card.border_1())
                    .border_color(rgb(if selected {
                        palette.accent
                    } else {
                        palette.border
                    }))
                    .bg(rgb(palette.background))
                    .when(selected, |card| card.shadow_sm())
                    .hover(|style| style.border_color(rgb(palette.accent_border)))
                    .cursor_pointer()
                    .child(
                        div().h(px(116.0)).w_full().bg(rgb(0xffffff)).child(
                            img(Arc::new(Image::from_bytes(
                                ImageFormat::Png,
                                template.thumbnail.to_vec(),
                            )))
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                        ),
                    )
                    .child(
                        div()
                            .h(px(32.0))
                            .px_2()
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .child(
                                div()
                                    .w_full()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_align(gpui::TextAlign::Center)
                                    .text_size(px(10.0))
                                    .text_color(rgb(if selected {
                                        palette.accent
                                    } else {
                                        palette.foreground
                                    }))
                                    .child(template.name(language)),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        theme_entity.update(cx, |this, cx| this.select_export_theme(index, cx));
                    }),
            )
        },
    );

    let close_entity = entity.clone();
    let cancel_entity = entity.clone();
    let export_entity = entity.clone();
    let reveal_entity = entity.clone();
    let ppi_144_entity = entity.clone();
    let ppi_300_entity = entity.clone();
    let format_name = match panel.options.format {
        ExportFormat::Pdf => "PDF",
        ExportFormat::Png => "PNG",
        ExportFormat::Svg => "SVG",
    };
    let action_label = format!("{} {format_name}", language.text("export.action"));

    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .p_4()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000066))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("export-panel-card")
                .debug_selector(|| "export-panel-card".to_owned())
                .w(px(880.0))
                .h(px(640.0))
                .max_w_full()
                .max_h_full()
                .overflow_hidden()
                .rounded(px(14.0))
                .border_1()
                .border_color(rgb(palette.border))
                .bg(rgb(palette.background))
                .shadow_lg()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(74.0))
                        .flex_none()
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(rgb(palette.divider))
                        .child(
                            div()
                                .child(
                                    div()
                                        .text_size(px(21.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(language.text("export.title")),
                                )
                                .child(
                                    div()
                                        .mt(px(3.0))
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.foreground_dim))
                                        .child(language.text("export.subtitle")),
                                ),
                        )
                        .child(
                            div()
                                .id("export-close")
                                .size(px(38.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(rgb(palette.border))
                                .bg(rgb(palette.background_alt))
                                .text_size(px(19.0))
                                .text_color(rgb(palette.foreground_dim))
                                .hover(|style| {
                                    style
                                        .bg(rgb(palette.hover))
                                        .border_color(rgb(palette.border_hover))
                                })
                                .cursor_pointer()
                                .child("×")
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    close_entity.update(cx, |this, cx| this.close_export_panel(cx));
                                }),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .p_4()
                        .flex()
                        .gap_4()
                        .child(
                            div()
                                .id("export-preview-stage")
                                .debug_selector(|| "export-preview-stage".to_owned())
                                .w(px(350.0))
                                .h_full()
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    img(Arc::new(Image::from_bytes(
                                        ImageFormat::Png,
                                        selected_template.thumbnail.to_vec(),
                                    )))
                                    .w(px(324.0))
                                    .h(px(458.0))
                                    .object_fit(ObjectFit::Contain)
                                    .shadow(vec![
                                        gpui::BoxShadow {
                                            color: gpui::hsla(0.0, 0.0, 0.0, 0.14),
                                            offset: gpui::point(px(0.0), px(0.0)),
                                            blur_radius: px(16.0),
                                            spread_radius: px(-2.0),
                                            inset: false,
                                        },
                                        gpui::BoxShadow {
                                            color: gpui::hsla(0.0, 0.0, 0.0, 0.08),
                                            offset: gpui::point(px(0.0), px(1.0)),
                                            blur_radius: px(4.0),
                                            spread_radius: px(-1.0),
                                            inset: false,
                                        },
                                    ]),
                                ),
                        )
                        .child(
                            div()
                                .id("export-settings-panel")
                                .debug_selector(|| "export-settings-panel".to_owned())
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .gap_4()
                                .child(
                                    div()
                                        .child(section_label(language.text("export.format")))
                                        .child(
                                            div()
                                                .mt_2()
                                                .p_1()
                                                .flex()
                                                .gap_1()
                                                .rounded(px(10.0))
                                                .bg(rgb(palette.background_alt))
                                                .child(format_button("PDF", ExportFormat::Pdf))
                                                .child(format_button("PNG", ExportFormat::Png))
                                                .child(format_button("SVG", ExportFormat::Svg)),
                                        ),
                                )
                                .child(
                                    div()
                                        .child(section_label(if panel.options.format
                                            == ExportFormat::Pdf
                                        {
                                            language.text("export.page")
                                        } else {
                                            language.text("export.output")
                                        }))
                                        .child(
                                            div()
                                                .mt_2()
                                                .flex()
                                                .gap_2()
                                                .when(
                                                    panel.options.format == ExportFormat::Pdf,
                                                    |row| {
                                                        row.child(
                                                            div()
                                                                .flex_1()
                                                                .p_1()
                                                                .flex()
                                                                .gap_1()
                                                                .rounded(px(10.0))
                                                                .bg(rgb(palette.background_alt))
                                                                .child(paper_button(
                                                                    0,
                                                                    "A4",
                                                                    PaperSize::A4,
                                                                ))
                                                                .child(paper_button(
                                                                    1,
                                                                    "A5",
                                                                    PaperSize::A5,
                                                                ))
                                                                .child(paper_button(
                                                                    2,
                                                                    "B5",
                                                                    PaperSize::B5,
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .w(px(168.0))
                                                                .p_1()
                                                                .flex()
                                                                .gap_1()
                                                                .rounded(px(10.0))
                                                                .bg(rgb(palette.background_alt))
                                                                .child(orientation_button(
                                                                    0,
                                                                    language.text(
                                                                        "export.portrait",
                                                                    ),
                                                                    Orientation::Portrait,
                                                                ))
                                                                .child(orientation_button(
                                                                    1,
                                                                    language.text(
                                                                        "export.landscape",
                                                                    ),
                                                                    Orientation::Landscape,
                                                                )),
                                                        )
                                                    },
                                                )
                                                .when(
                                                    panel.options.format != ExportFormat::Pdf,
                                                    |row| {
                                                        row.child(
                                                            div()
                                                                .id("export-long-image")
                                                                .h(px(44.0))
                                                                .flex_1()
                                                                .px_4()
                                                                .flex()
                                                                .items_center()
                                                                .justify_between()
                                                                .rounded(px(10.0))
                                                                .border_1()
                                                                .border_color(rgb(palette.accent))
                                                                .bg(rgb(palette.background))
                                                                .text_color(rgb(palette.accent))
                                                                .child(language.text(
                                                                    "export.long_image",
                                                                ))
                                                                .child("✓"),
                                                        )
                                                    },
                                                ),
                                        )
                                        .when(
                                            panel.options.format == ExportFormat::Png,
                                            |section| {
                                                section.child(
                                                    div()
                                                        .mt_2()
                                                        .flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .child(
                                                            div()
                                                                .text_size(px(11.0))
                                                                .text_color(rgb(
                                                                    palette.foreground_dim,
                                                                ))
                                                                .child(language.text(
                                                                    "export.quality",
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex()
                                                                .gap_2()
                                                                .child(
                                                                    div()
                                                                        .id("export-ppi-144")
                                                                        .h(px(32.0))
                                                                        .px_3()
                                                                        .flex()
                                                                        .items_center()
                                                                        .rounded(px(8.0))
                                                                        .border_1()
                                                                        .border_color(rgb(
                                                                            if panel.options.png_ppi
                                                                                == 144.0
                                                                            {
                                                                                palette.accent
                                                                            } else {
                                                                                palette.border
                                                                            },
                                                                        ))
                                                                        .text_color(rgb(
                                                                            if panel.options.png_ppi
                                                                                == 144.0
                                                                            {
                                                                                palette.accent
                                                                            } else {
                                                                                palette.foreground
                                                                            },
                                                                        ))
                                                                        .cursor_pointer()
                                                                        .child(format!(
                                                                            "{} · 144 PPI",
                                                                            language.text(
                                                                                "export.standard"
                                                                            )
                                                                        ))
                                                                        .on_mouse_down(
                                                                            MouseButton::Left,
                                                                            move |_, _, cx| {
                                                                                ppi_144_entity.update(
                                                                                    cx,
                                                                                    |this, cx| {
                                                                                        this.select_export_ppi(144.0, cx)
                                                                                    },
                                                                                );
                                                                            },
                                                                        ),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .id("export-ppi-300")
                                                                        .h(px(32.0))
                                                                        .px_3()
                                                                        .flex()
                                                                        .items_center()
                                                                        .rounded(px(8.0))
                                                                        .border_1()
                                                                        .border_color(rgb(
                                                                            if panel.options.png_ppi
                                                                                == 300.0
                                                                            {
                                                                                palette.accent
                                                                            } else {
                                                                                palette.border
                                                                            },
                                                                        ))
                                                                        .text_color(rgb(
                                                                            if panel.options.png_ppi
                                                                                == 300.0
                                                                            {
                                                                                palette.accent
                                                                            } else {
                                                                                palette.foreground
                                                                            },
                                                                        ))
                                                                        .cursor_pointer()
                                                                        .child(format!(
                                                                            "{} · 300 PPI",
                                                                            language.text(
                                                                                "export.high"
                                                                            )
                                                                        ))
                                                                        .on_mouse_down(
                                                                            MouseButton::Left,
                                                                            move |_, _, cx| {
                                                                                ppi_300_entity.update(
                                                                                    cx,
                                                                                    |this, cx| {
                                                                                        this.select_export_ppi(300.0, cx)
                                                                                    },
                                                                                );
                                                                            },
                                                                        ),
                                                                ),
                                                        ),
                                                )
                                            },
                                        ),
                                )
                                .child(
                                    div()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .child(section_label(language.text(
                                                    "export.templates",
                                                )))
                                                .child(
                                                    div()
                                                        .text_size(px(10.0))
                                                        .text_color(rgb(palette.accent))
                                                        .child(format!(
                                                            "{} · {}",
                                                            export_templates().len(),
                                                            language.text("export.themes")
                                                        )),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .id("export-template-grid")
                                                .mt_2()
                                                .w_full()
                                                .h(px(178.0))
                                                .overflow_y_scroll()
                                                .restrict_scroll_to_axis()
                                                .child(theme_grid),
                                        ),
                                )
                                .child(
                                    div()
                                        .rounded(px(10.0))
                                        .border_1()
                                        .border_color(rgb(palette.divider))
                                        .bg(rgb(palette.background_alt))
                                        .p_3()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .text_size(px(11.0))
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                        .child(language.text("export.advanced")),
                                                )
                                                .child(
                                                    div()
                                                        .mt(px(3.0))
                                                        .text_size(px(10.0))
                                                        .text_color(rgb(palette.foreground_dim))
                                                        .child(language.text(
                                                            "export.advanced_summary",
                                                        )),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .ml_3()
                                                .flex_none()
                                                .text_size(px(10.0))
                                                .text_color(rgb(palette.foreground_dim))
                                                .child(language.text("export.template_default")),
                                        ),
                                ),
                        ),
                )
                .child(
                    div()
                        .h(px(66.0))
                        .flex_none()
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_t_1()
                        .border_color(rgb(palette.divider))
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.foreground_dim))
                                .when(status.is_none(), |view| {
                                    view.child(format!(
                                        "{} · {format_name}",
                                        language.text("export.snapshot_hint")
                                    ))
                                })
                                .when_some(status.clone(), |view, status| {
                                    let (message, color) = match status {
                                        ExportRunState::Working(message) => {
                                            (message, palette.foreground_muted)
                                        }
                                        ExportRunState::Success { message, .. } => {
                                            (message, palette.success)
                                        }
                                        ExportRunState::Error(message) => (message, palette.error),
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
                                    matches!(status.as_ref(), Some(ExportRunState::Success { .. })),
                                    |view| {
                                        view.child(
                                            div()
                                                .id("export-reveal")
                                                .h(px(38.0))
                                                .px_4()
                                                .flex()
                                                .items_center()
                                                .rounded(px(9.0))
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
                                .child(
                                    div()
                                        .id("export-cancel")
                                        .h(px(38.0))
                                        .px_4()
                                        .flex()
                                        .items_center()
                                        .rounded(px(9.0))
                                        .bg(rgb(palette.background_alt))
                                        .hover(|style| style.bg(rgb(palette.hover)))
                                        .cursor_pointer()
                                        .child(language.text("export.cancel"))
                                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                            cancel_entity.update(cx, |this, cx| {
                                                this.close_export_panel(cx)
                                            });
                                        }),
                                )
                                .child(
                                    div()
                                        .id("export-confirm")
                                        .h(px(38.0))
                                        .min_w(px(112.0))
                                        .px_5()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(9.0))
                                        .bg(rgb(if working {
                                            palette.foreground_disabled
                                        } else {
                                            palette.accent
                                        }))
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
                                            language.text("export.working_action").to_owned()
                                        } else {
                                            action_label
                                        }),
                                ),
                        ),
                ),
        )
}
