use smallvec::SmallVec;

use crate::preview::layout::LayoutKey;

use super::{
    MinimapLineIndex, RasterTilePaint, media::MinimapMediaPaint, viewport::MinimapViewport,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GeometryGeneration {
    document_generation: u64,
    layout: LayoutKey,
    rows_signature: u64,
    snapshot_identity: usize,
}

impl GeometryGeneration {
    fn new(document_generation: u64, index: &MinimapLineIndex) -> Self {
        Self {
            document_generation,
            layout: index.layout,
            rows_signature: index.rows_signature,
            snapshot_identity: std::sync::Arc::as_ptr(&index.projection) as usize,
        }
    }
}

/// One immutable paint and interaction commit. Keeping the index and viewport beside every
/// visible artifact prevents a retained raster from being combined with a newer thumb or hit map.
#[derive(Clone)]
pub(super) struct PreparedMinimapFrame {
    pub(super) generation: GeometryGeneration,
    pub(super) tiles: SmallVec<[RasterTilePaint; 6]>,
    pub(super) media: SmallVec<[MinimapMediaPaint; 4]>,
    pub(super) line_index: MinimapLineIndex,
    pub(super) viewport: MinimapViewport,
}

impl PreparedMinimapFrame {
    pub(super) fn new(
        document_generation: u64,
        line_index: MinimapLineIndex,
        viewport: MinimapViewport,
        tiles: SmallVec<[RasterTilePaint; 6]>,
        media: SmallVec<[MinimapMediaPaint; 4]>,
    ) -> Self {
        Self {
            generation: GeometryGeneration::new(document_generation, &line_index),
            tiles,
            media,
            line_index,
            viewport,
        }
    }

    #[cfg(test)]
    pub(super) fn uses_geometry(&self, document_generation: u64, index: &MinimapLineIndex) -> bool {
        self.generation == GeometryGeneration::new(document_generation, index)
    }

    pub(super) fn is_coherent(&self) -> bool {
        self.generation
            == GeometryGeneration::new(self.generation.document_generation, &self.line_index)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        document::Revision,
        preview::layout::{LayoutSnapshot, ResolvedRow},
    };

    fn index(projection: Arc<LayoutSnapshot>) -> MinimapLineIndex {
        MinimapLineIndex {
            layout: LayoutKey {
                document_revision: Revision::INITIAL,
                content_width_px: 800,
                text_metrics_revision: 7,
                fold_revision: 3,
            },
            width: 800,
            minimap_width: 120,
            rows_signature: 11,
            density: super::super::MinimapDensity::Large,
            reading_line_height: 24.0,
            projection,
        }
    }

    #[test]
    fn one_frame_uses_one_geometry_generation() {
        let first_index = index(Arc::new(LayoutSnapshot::new(vec![ResolvedRow::new(
            1, 24.0, false,
        )])));
        let frame = PreparedMinimapFrame::new(
            19,
            first_index.clone(),
            MinimapViewport::default(),
            SmallVec::new(),
            SmallVec::new(),
        );
        assert!(frame.uses_geometry(19, &first_index));
        assert!(frame.is_coherent());

        // Exact refinement publishes another immutable snapshot even when the document and row
        // identities stay unchanged. It must therefore become a distinct frame generation.
        let refined_index = index(Arc::new(LayoutSnapshot::new(vec![ResolvedRow::new(
            1, 24.0, true,
        )])));
        assert!(!frame.uses_geometry(19, &refined_index));
        assert!(!frame.uses_geometry(20, &first_index));
    }
}
