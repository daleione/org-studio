use super::{DocumentFormat, ReadingPreviewPanel};
use crate::plot::{PlotEntry, PlotHost, PlotPresentation, PlotStyle};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub(crate) struct PlotProjection {
    pub(super) entry: Arc<PlotEntry>,
    pub(crate) sizing: Arc<Mutex<super::ImageSizing>>,
}
impl PlotProjection {
    pub(super) fn new(entry: Arc<PlotEntry>) -> Self {
        Self {
            entry,
            sizing: Arc::new(Mutex::new(super::ImageSizing {
                auto_width: 480.0,
                max_width: 480.0,
                max_height: 1800.0,
                font_size: 16.0,
            })),
        }
    }
}

impl ReadingPreviewPanel {
    pub(crate) fn derived_document(&self) -> &Arc<super::PreviewSnapshot> {
        self.plot_source.as_ref().unwrap_or(self.document())
    }

    pub(crate) fn attach_plots(
        &mut self,
        file_path: Option<std::path::PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        let document = self.derived_document().clone();
        if document.format != DocumentFormat::Org {
            return;
        }
        if self.plots.is_none() {
            cx.on_release(|panel, cx| {
                panel.cancel_plot_resize(cx);
            })
            .detach();
            self.plots = Some(PlotHost::new(document.text.clone(), Self::sync_plots, cx));
        } else if let Some(host) = self.plots.as_mut() {
            host.refresh(document.text.clone(), None, cx);
        }
        if let Some(host) = self.plots.as_ref() {
            host.document
                .update(cx, |document, _| document.set_file_path(file_path));
        }
        self.sync_plots(cx);
    }

    fn sync_plots(&mut self, cx: &mut gpui::Context<Self>) {
        let base = self.derived_document().clone();
        let source_document = self.document().clone();
        let Some(host) = self.plots.as_mut() else {
            return;
        };
        let mut entries = host.entries(source_document.revision, cx);
        if !host.is_current() && entries.is_empty() {
            entries =
                crate::plot::collect_plots(source_document.text.as_ref(), &source_document.blocks)
                    .into_iter()
                    .map(PlotEntry::placeholder)
                    .collect::<Vec<_>>()
                    .into();
        } else if !host.is_current() {
            let nodes = source_document.blocks.nodes();
            entries = entries
                .iter()
                .map(|entry| {
                    let mut entry = (**entry).clone();
                    let index =
                        nodes.partition_point(|node| node.source.start < entry.source.table.end);
                    if let Some(owner) = index.checked_sub(1)
                        && nodes[owner].source.end == entry.source.table.end
                        && matches!(nodes[owner].kind, crate::org_syntax::BlockKind::TableRow)
                    {
                        entry.source.owner = owner as crate::org_syntax::BlockId;
                    }
                    Arc::new(entry)
                })
                .collect::<Vec<_>>()
                .into();
        }
        let signature = (
            source_document.revision,
            host.is_current(),
            entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        );
        if self.plot_signature.as_ref() == Some(&signature) {
            let display_revision = host.document.read(cx).display_revision;
            if display_revision != self.plot_display_revision {
                self.plot_display_revision = display_revision;
                self.invalidate_plot_sizes(cx);
            }
            return;
        }
        self.plot_signature = Some(signature.clone());
        if entries.is_empty()
            && !self.plot_rows.iter().any(|(index, _)| {
                source_document
                    .projection
                    .rows
                    .get(*index)
                    .is_some_and(|row| {
                        matches!(row.kind, super::projection::VisualRowKind::Plot(_))
                    })
            })
        {
            self.plot_rows.clear();
            return;
        }
        let mut document = (*base).clone();
        let (projection, rows) = document.projection.with_plots(&entries);
        let viewport_height = self
            .viewport_revision_key
            .map(|(_, height)| f32::from_bits(height))
            .unwrap_or(900.0);
        for (row, _) in &rows {
            if let Some(super::projection::VisualRow {
                kind: super::projection::VisualRowKind::Plot(plot),
                ..
            }) = projection.rows.get(*row)
            {
                let mut sizing = plot.sizing.lock().unwrap();
                sizing.max_height = viewport_height * 2.0;
                sizing.font_size =
                    super::preview_style(self.plot_style).typography.body_size * self.zoom();
            }
        }
        document.projection = Arc::new(projection);
        document.display_map = Some(Arc::new(super::display_map::build_display_map(&document)));
        document.update = super::DerivedUpdate::Full;
        self.replace_document_with_style(
            Arc::new(document),
            *super::preview_style(self.plot_style),
            cx,
        );
        self.plot_source = Some(base);
        self.plot_rows = rows;
        self.plot_signature = Some(signature);
        cx.notify();
    }

    pub(super) fn plot_presentation(
        &mut self,
        projection: &PlotProjection,
        active: bool,
        width: f32,
        style: super::PreviewStyle,
        cx: &mut gpui::Context<Self>,
    ) -> Option<PlotPresentation> {
        self.plot_style = style.id;
        let font_size = style.typography.body_size * self.zoom();
        let host = self.plots.as_mut()?;
        let sizing = {
            let mut sizing = projection.sizing.lock().unwrap();
            sizing.max_width = width;
            sizing.font_size = font_size;
            *sizing
        };
        Some(host.present(
            projection.entry.clone(),
            sizing,
            plot_style(style),
            active,
            cx,
        ))
    }

    pub(crate) fn plot_previews_ready(&self, cx: &gpui::App) -> bool {
        self.plots
            .as_ref()
            .is_none_or(|host| host.previews_ready(self.document().revision, cx))
    }
}

pub(super) fn plot_style(style: super::PreviewStyle) -> PlotStyle {
    let theme = crate::theme::current_theme();
    let mut plot = PlotStyle::for_theme(theme, 12.0);
    plot.foreground = style.palette.foreground;
    plot.background = if theme.editor_background == crate::theme::ORG_STUDIO_DARK.editor_background
    {
        style.palette.surface_elevated
    } else {
        style.palette.background
    };
    plot.colors[0] = style.palette.accent;
    if style.id != super::PreviewStyleId::Base {
        plot.fonts.insert(0, style.typography.body_family.into());
    }
    plot
}

pub(super) struct PlotResizeSession {
    document: gpui::Entity<crate::plot::PlotDocument>,
    id: crate::plot::PlotId,
    target: Option<super::PreviewActionTarget>,
    pointer: (f32, f32),
    start_size: (f32, f32),
    current_size: (f32, f32),
    previous_size: (Option<f32>, Option<f32>),
    axis: crate::plot::PlotResizeAxis,
    column_width: f32,
    moved: bool,
}

impl ReadingPreviewPanel {
    fn invalidate_plot_sizes(&mut self, cx: &mut gpui::Context<Self>) {
        self.remeasure_plot_rows();
        self.invalidate_minimap_layout();
        cx.notify();
    }

    pub(super) fn cancel_plot_resize(&mut self, cx: &mut gpui::App) {
        if let Some(drag) = self.plot_resize.take() {
            drag.document.update(cx, |document, cx| {
                document.resize_canvas(
                    drag.id,
                    drag.previous_size.0,
                    drag.previous_size.1,
                    false,
                    cx,
                )
            });
        }
    }

    pub(super) fn start_plot_resize(
        &mut self,
        plot: &PlotPresentation,
        pointer: (f32, f32),
        column_width: f32,
        target: Option<super::PreviewActionTarget>,
        axis: crate::plot::PlotResizeAxis,
        cx: &mut gpui::Context<Self>,
    ) {
        self.cancel_plot_resize(cx);
        let display = plot.entry.display.lock().unwrap();
        self.plot_resize = Some(PlotResizeSession {
            document: plot.document.clone(),
            id: plot.entry.id,
            target,
            pointer,
            start_size: (plot.width, plot.height),
            current_size: (plot.width, plot.height),
            previous_size: (display.requested_width, display.requested_height),
            column_width,
            axis,
            moved: false,
        });
    }

    pub(super) fn move_plot_resize(&mut self, pointer: (f32, f32), cx: &mut gpui::Context<Self>) {
        let Some(drag) = &mut self.plot_resize else {
            return;
        };
        let (width, height) = drag.axis.resize(
            drag.start_size,
            (pointer.0 - drag.pointer.0, pointer.1 - drag.pointer.1),
            drag.column_width,
        );
        if (width - drag.current_size.0).abs() < 0.25 && (height - drag.current_size.1).abs() < 0.25
        {
            return;
        }
        drag.current_size = (width, height);
        drag.moved = true;
        drag.document.update(cx, |document, cx| {
            document.resize_canvas(drag.id, Some(width), Some(height), true, cx)
        });
    }

    pub(super) fn finish_plot_resize(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> Option<super::PreviewAction> {
        let drag = self.plot_resize.take()?;
        let (width, height) = if drag.moved {
            (Some(drag.current_size.0), Some(drag.current_size.1))
        } else {
            drag.previous_size
        };
        drag.document.update(cx, |document, cx| {
            document.resize_canvas(drag.id, width, height, false, cx)
        });
        if !drag.moved {
            return None;
        }
        drag.target.map(|target| super::PreviewAction::ResizeImage {
            target,
            size: Some((
                drag.current_size.0.round() as u32,
                drag.current_size.1.round() as u32,
            )),
        })
    }

    pub(super) fn reset_plot_size(
        &mut self,
        plot: &PlotPresentation,
        cx: &mut gpui::Context<Self>,
    ) {
        plot.document
            .update(cx, |document, cx| document.reset_size(plot.entry.id, cx));
    }
}

#[cfg(test)]
mod tests;
