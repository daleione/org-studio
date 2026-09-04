use std::sync::Arc;

use gpui::RenderImage;

use crate::{
    document::DocumentFormat,
    org_syntax::BlockKind,
    preview::{
        PreviewSnapshot,
        markdown::MarkdownKind,
        projection::{VisualRow, VisualRowKind},
    },
};

#[derive(Clone)]
pub(super) struct MinimapMediaPaint {
    pub(super) image: Arc<RenderImage>,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) row_y: f32,
    pub(super) row_height: f32,
}

pub(super) fn geometry(
    reading_width: f32,
    reading_height: f32,
    reading_line_height: f32,
    minimap_line_height: f32,
    minimap_width: f32,
    row_y: f32,
    row_height: f32,
) -> Option<(f32, f32, f32, f32)> {
    if !reading_width.is_finite()
        || !reading_height.is_finite()
        || reading_width <= 0.0
        || reading_height <= 0.0
        || reading_line_height <= 0.0
        || minimap_line_height <= 0.0
        || row_height <= 0.0
    {
        return None;
    }
    let inset = 5.0;
    let available_width = (minimap_width - inset * 2.0).max(1.0);
    // Preserve the Reading aspect ratio and vertical scale. Filling the minimap width
    // independently would change the document geometry while the viewport crosses media.
    let scale = (minimap_line_height / reading_line_height)
        .min(available_width / reading_width)
        .min(row_height / reading_height)
        .min(1.0);
    Some((inset, row_y, reading_width * scale, reading_height * scale))
}

pub(super) fn image(
    document: &PreviewSnapshot,
    visual: &VisualRow,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) -> Option<Arc<RenderImage>> {
    match &visual.kind {
        VisualRowKind::Image { .. } => {
            let source = match document.format {
                DocumentFormat::Org => {
                    match &document.blocks.nodes().get(visual.block_id as usize)?.kind {
                        BlockKind::Image { path } => path.as_ref(),
                        _ => return None,
                    }
                }
                DocumentFormat::Markdown => {
                    match &document.markdown_blocks.get(visual.block_id as usize)?.kind {
                        MarkdownKind::Image { path } => path.as_str(),
                        _ => return None,
                    }
                }
            };
            let path = crate::preview::resolve_image_path(&document.path, source);
            let resource: gpui::Resource = path.into();
            window
                .use_asset::<gpui::ImgResourceLoader>(&resource, cx)?
                .ok()
        }
        VisualRowKind::Diagram(crate::preview::diagram::DiagramProjection::Ready {
            image, ..
        }) => image.clone().use_render_image(window, cx),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::geometry;

    #[test]
    fn preserves_aspect_ratio_and_row_bounds() {
        let (x, y, width, height) = geometry(800.0, 400.0, 27.52, 3.8, 110.0, 20.0, 80.0).unwrap();
        assert!((width / height - 2.0).abs() < 0.001);
        assert_eq!(x, 5.0);
        assert_eq!(y, 20.0);
        assert!(x + width <= 105.0);
        assert!(y + height <= 100.0);

        let (_, tall_y, tall_width, tall_height) =
            geometry(200.0, 800.0, 27.52, 3.8, 110.0, -10.0, 60.0).unwrap();
        assert!((tall_width / tall_height - 0.25).abs() < 0.001);
        assert_eq!(tall_y, -10.0);
        assert!(tall_y + tall_height <= 50.001);
    }
}
