use super::{
    SemanticEditor,
    element::{scroll_is_at_end, stabilized_scroll_y},
};
use crate::{
    document::{DocumentSnapshot, TextSnapshot},
    plot::{PlotDemand, PlotEntry, PlotHost, PlotId, PlotPresentation, PlotStyle},
};
use gpui::{App, Bounds, Pixels};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

#[derive(Default)]
pub(super) struct EditorPlots {
    pub(super) host: Option<PlotHost>,
    pub(super) lines: HashMap<u64, PlotPresentation>,
    pub(super) sources: HashMap<u64, Arc<PlotEntry>>,
    pub(super) minimap_demands: RefCell<HashMap<PlotId, PlotDemand>>,
    pub(super) minimap_geometry: RefCell<Option<PlotMinimapGeometry>>,
}

pub(super) struct PlotMinimapGeometry {
    pub(super) generation: u64,
    pub(super) images: Vec<PlotMinimapImage>,
}

pub(super) struct PlotMinimapImage {
    pub(super) id: PlotId,
    pub(super) top_units: f32,
    pub(super) height_units: f32,
}

pub(super) fn prepare(
    host: &gpui::Entity<SemanticEditor>,
    snapshot: &DocumentSnapshot,
    width: f32,
    bounds: Bounds<Pixels>,
    cx: &mut App,
) {
    host.update(cx, |editor, cx| {
        let Some(plots) = editor.plots.host.as_ref() else {
            return;
        };
        let file_path = editor.session.read(cx).file_path().map(ToOwned::to_owned);
        plots
            .document
            .update(cx, |document, _| document.set_file_path(file_path));
        let viewport_height = f32::from(bounds.size.height);
        let was_at_end = editor.scroll_at_end
            || scroll_is_at_end(
                editor.scroll_y,
                viewport_height,
                editor.animated_document_height(),
            );
        let anchor = editor.animated_line_at_y(editor.scroll_y);
        let offset = editor.scroll_y - editor.animated_line_start_y(anchor);
        if editor.sync_plot_layout(snapshot, width, viewport_height, cx) {
            editor.scroll_y = stabilized_scroll_y(
                was_at_end,
                editor.animated_line_start_y(anchor) + offset,
                viewport_height,
                editor.animated_document_height(),
            );
            editor.scroll_at_end = was_at_end;
            editor.minimap.cancel_layout_preparation();
            editor.minimap.note_layout_changed();
        }
        let Some(mut plots) = editor.plots.host.take() else {
            return;
        };
        let style = PlotStyle::for_theme(crate::theme::current_theme(), 12.0);
        let sizing =
            super::inline_image::image_sizing(width, viewport_height, editor.font_size_px());
        let mut lines = HashMap::new();
        for (&line, entry) in &editor.plots.sources {
            let (_, height) = entry.image_size(&sizing);
            let y = editor.animated_line_start_y(line);
            if y + editor.display_map.line_height_px(line) < editor.scroll_y - height
                || y > editor.scroll_y + f32::from(bounds.size.height) + height
            {
                continue;
            }
            let presentation = plots.present(entry.clone(), sizing, style.clone(), true, cx);
            lines.insert(line, presentation);
        }
        let visible = lines
            .values()
            .map(|plot| plot.entry.id)
            .collect::<HashSet<PlotId>>();
        plots.retain(&visible);
        editor.plots.lines = lines;
        editor.plots.host = Some(plots);
    });
}

impl EditorPlots {
    pub(super) fn new(host: Option<PlotHost>) -> Self {
        Self {
            host,
            ..Self::default()
        }
    }
}

impl SemanticEditor {
    /// Restores plot geometry after a source-line splice, before its caller
    /// clamps the viewport to the new document height. Also used by prepaint
    /// when a completed plot index or a canvas resize changes that geometry.
    pub(super) fn sync_plot_layout(
        &mut self,
        snapshot: &DocumentSnapshot,
        width: f32,
        viewport_height: f32,
        cx: &mut App,
    ) -> bool {
        let Some(plots) = self.plots.host.as_mut() else {
            return false;
        };
        let entries = plots.entries(snapshot.revision(), cx);
        let sizing = super::inline_image::image_sizing(width, viewport_height, self.font_size_px());
        let mut sources = HashMap::new();
        let mut trailing = BTreeMap::new();
        let mut linked_changed = false;
        for entry in entries.iter() {
            let end = entry
                .source
                .image_link
                .map(|range| range.start)
                .unwrap_or(entry.source.source_end());
            let Ok(mut line) = snapshot.line_index_at(end) else {
                continue;
            };
            if entry.source.image_link.is_none()
                && line.0 > 0
                && snapshot
                    .line_range(line)
                    .is_ok_and(|range| range.start == end)
            {
                line.0 -= 1;
            }
            if self.display_map.is_hidden(line.0) {
                continue;
            }
            sources.insert(line.0, entry.clone());
            if let Some(link) = entry.source.image_link
                && matches!(entry.prepared, crate::plot::PlotPreparation::Ready(_))
                && self.previews_inline_image_at(link.start)
            {
                self.inline_image_line_dimensions.borrow_mut().insert(
                    line.0,
                    super::InlineImageMetrics {
                        plot: Some(entry.clone()),
                        line_start: link.start.0,
                        source: entry.dimensions(),
                        spec: entry.source.image_attributes.clone(),
                    },
                );
                linked_changed |=
                    self.apply_inline_image_row_height(line.0, width, viewport_height);
            }
            if entry.source.image_link.is_none() {
                trailing.insert(line.0, entry.image_size(&sizing).1 + 12.0);
            }
        }
        self.plots.sources = sources;
        self.display_map.set_trailing_heights(trailing) || linked_changed
    }

    pub(crate) fn plot_previews_ready(&self, cx: &App) -> bool {
        self.plots
            .host
            .as_ref()
            .is_none_or(|host| host.previews_ready(self.session.read(cx).revision(), cx))
    }
}

#[cfg(test)]
mod tests;
