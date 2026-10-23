use super::SemanticEditor;
use crate::{
    document::{ByteOffset, DocumentSnapshot, TextSnapshot},
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
        let Some(mut plots) = editor.plots.host.take() else {
            return;
        };
        let file_path = editor.session.read(cx).file_path().map(ToOwned::to_owned);
        plots
            .document
            .update(cx, |document, _| document.set_file_path(file_path));
        let entries = plots.entries(snapshot.revision(), cx);
        let style = PlotStyle::for_theme(crate::theme::current_theme(), 12.0);
        let sizing = super::inline_image::image_sizing(
            width,
            f32::from(bounds.size.height),
            editor.font_size_px(),
        );
        let mut lines = HashMap::new();
        let mut sources = HashMap::new();
        let mut trailing = BTreeMap::new();
        for entry in entries.iter() {
            let end = entry
                .source
                .image_link
                .map(|range| range.start)
                .unwrap_or(ByteOffset(entry.source.table.end.0.saturating_sub(1)));
            let Ok(line) = snapshot.line_index_at(end) else {
                continue;
            };
            if editor.display_map.is_hidden(line.0) {
                continue;
            }
            sources.insert(line.0, entry.clone());
            if let Some(link) = entry.source.image_link
                && matches!(entry.prepared, crate::plot::PlotPreparation::Ready(_))
                && editor.previews_inline_image_at(link.start)
            {
                editor.inline_image_line_dimensions.borrow_mut().insert(
                    line.0,
                    super::InlineImageMetrics {
                        plot: Some(entry.clone()),
                        line_start: link.start.0,
                        source: entry.dimensions(),
                        spec: entry.source.image_attributes.clone(),
                    },
                );
                editor.apply_inline_image_row_height(line.0, width, f32::from(bounds.size.height));
            }
            let (_, height) = entry.image_size(&sizing);
            if entry.source.image_link.is_none() {
                trailing.insert(line.0, height + 12.0);
            }
            let y = editor.animated_line_start_y(line.0);
            if y + editor.display_map.line_height_px(line.0) < editor.scroll_y - height
                || y > editor.scroll_y + f32::from(bounds.size.height) + height
            {
                continue;
            }
            let presentation = plots.present(entry.clone(), sizing, style.clone(), true, cx);
            lines.insert(line.0, presentation);
        }
        let visible = lines
            .values()
            .map(|plot| plot.entry.id)
            .collect::<HashSet<PlotId>>();
        plots.retain(&visible);
        let anchor = editor.animated_line_at_y(editor.scroll_y);
        let offset = editor.scroll_y - editor.animated_line_start_y(anchor);
        if editor.display_map.set_trailing_heights(trailing) {
            editor.scroll_y = editor.animated_line_start_y(anchor) + offset;
            editor.scroll_at_end = false;
            editor.minimap.cancel_layout_preparation();
            editor.minimap.note_layout_changed();
        }
        editor.plots.lines = lines;
        editor.plots.sources = sources;
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
    pub(crate) fn plot_previews_ready(&self, cx: &App) -> bool {
        self.plots
            .host
            .as_ref()
            .is_none_or(|host| host.previews_ready(self.session.read(cx).revision(), cx))
    }
}

#[cfg(test)]
mod tests;
