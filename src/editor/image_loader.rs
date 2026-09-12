use std::{path::Path, sync::Arc};

use gpui::{App, Asset, ImageCacheError, RenderImage};

#[derive(Clone)]
pub(super) struct LoadedImage {
    pub(super) image: Arc<RenderImage>,
    pub(super) dimensions: (u32, u32),
}

impl LoadedImage {
    pub(super) fn from_render(image: Arc<RenderImage>, scale: f32) -> Self {
        let size = image.size(0);
        Self {
            dimensions: (
                ((size.width.0 as f32 / scale).round() as u32).max(1),
                ((size.height.0 as f32 / scale).round() as u32).max(1),
            ),
            image,
        }
    }
}

/// Keep pixels and logical dimensions from the same bytes, resolved off the UI thread.
/// The body and minimap share this asset, including its resource-change invalidation.
pub(super) enum EditorImageLoader {}

impl Asset for EditorImageLoader {
    type Source = Arc<Path>;
    type Output = Result<LoadedImage, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let is_svg = source
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
        let renderer = cx.svg_renderer();
        let raster = (!is_svg).then(|| {
            cx.fetch_asset::<gpui::ImgResourceLoader>(&source.clone().into())
                .0
        });
        async move {
            if let Some(raster) = raster {
                return raster
                    .await
                    .map(|image| LoadedImage::from_render(image, 1.0));
            }
            let bytes = std::fs::read(source.as_ref())?;
            decode_svg(&bytes, &renderer)
        }
    }
}

pub(super) fn decode_svg(
    bytes: &[u8],
    renderer: &gpui::SvgRenderer,
) -> Result<LoadedImage, ImageCacheError> {
    let image = renderer.render_single_frame(bytes, 1.0)?;
    let mut loaded = LoadedImage::from_render(image, gpui::SMOOTH_SVG_SCALE_FACTOR);
    if let Some(dimensions) = crate::preview::svg_dimensions(bytes) {
        loaded.dimensions = dimensions;
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;

    #[gpui::test]
    async fn image_loader_refreshes_pixels_and_dimensions_together(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!(
            "org-image-loader-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("image.svg");
        let source: Arc<Path> = path.clone().into();
        std::fs::write(&path, br#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="50%" viewBox="0 0 100 100"><rect width="100" height="100"/></svg>"#).unwrap();
        let (task, first) = cx.update(|cx| cx.fetch_asset::<EditorImageLoader>(&source));
        assert!(first);
        let initial = task.await.unwrap();
        assert_eq!(initial.dimensions, (100, 50));
        assert!(
            !cx.update(|cx| cx.fetch_asset::<EditorImageLoader>(&source))
                .1
        );
        let session = cx.new(|_| {
            crate::document::DocumentSession::from_utf8(
                dir.join("doc.org"),
                b"[[file:image.svg]]\n".to_vec(),
            )
            .unwrap()
        });
        let editor = cx.new(|cx| crate::editor::SemanticEditor::new(session, cx));
        editor.update(cx, |editor, _| {
            editor.accept_inline_image_render(&path, initial.clone());
        });
        std::fs::write(&path, br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" viewBox="0 0 100 100"><rect width="100" height="100"/></svg>"#).unwrap();
        editor.update(cx, |editor, cx| editor.refresh_inline_image(&path, cx));
        let (task, first) = cx.update(|cx| cx.fetch_asset::<EditorImageLoader>(&source));
        assert!(first, "resource changes must invalidate the combined asset");
        let replacement = task.await.unwrap();
        assert_eq!(replacement.dimensions, (200, 100));
        assert_ne!(replacement.image.id, initial.image.id);
        // Consuming the prepared result must not reopen the file on the UI thread.
        std::fs::remove_file(&path).unwrap();
        editor.update(cx, |editor, _| {
            assert_eq!(
                editor.accept_inline_image_render(&path, replacement).1,
                (200, 100)
            );
        });
        std::fs::remove_dir(&dir).unwrap();
    }
}
