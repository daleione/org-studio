use std::path::{Path, PathBuf};

pub(super) struct MinimapImagePaint {
    pub(super) path: PathBuf,
    pub(super) row_y: f32,
    pub(super) row_height: f32,
}

pub(super) fn image_path(document_path: &Path, text: &str) -> Option<PathBuf> {
    if crate::document::DocumentFormat::from_path(document_path)
        != crate::document::DocumentFormat::Org
    {
        return None;
    }
    crate::org_syntax::standalone_image_path(text)
        .map(|source| crate::preview::resolve_image_path(document_path, source))
}

pub(super) fn geometry(
    natural_width: f32,
    natural_height: f32,
    minimap_width: f32,
    row_y: f32,
    row_height: f32,
) -> Option<(f32, f32, f32, f32)> {
    if natural_width <= 0.0 || natural_height <= 0.0 || minimap_width <= 0.0 || row_height <= 0.0 {
        return None;
    }
    const INSET: f32 = 5.0;
    let available_width = (minimap_width - INSET * 2.0).max(1.0);
    // The row is already expressed in Editor visual units. Fit uniformly into that row so
    // loading media cannot change the minimap's vertical geometry.
    let scale = (available_width / natural_width)
        .min(row_height / natural_height)
        .min(1.0);
    let width = natural_width * scale;
    let height = natural_height * scale;
    Some((INSET, row_y + (row_height - height) * 0.5, width, height))
}

#[cfg(test)]
mod tests {
    use super::{geometry, image_path};
    use std::path::Path;

    #[test]
    fn resolves_org_image_rows_but_not_markdown_source() {
        assert_eq!(
            image_path(
                Path::new("/tmp/project/note.org"),
                "[[file:images/diagram.png]]"
            ),
            Some(std::path::PathBuf::from("/tmp/project/images/diagram.png"))
        );
        assert_eq!(
            image_path(
                Path::new("/tmp/project/note.md"),
                "[[file:images/diagram.png]]"
            ),
            None
        );
        assert_eq!(
            image_path(Path::new("/tmp/project/note.org"), "ordinary text"),
            None
        );
    }

    #[test]
    fn thumbnail_is_bounded_and_preserves_aspect_ratio() {
        assert_eq!(
            geometry(400.0, 200.0, 96.0, 12.0, 60.0),
            Some((5.0, 20.5, 86.0, 43.0))
        );
        assert_eq!(
            geometry(20.0, 10.0, 96.0, 12.0, 60.0),
            Some((5.0, 37.0, 20.0, 10.0))
        );
        assert_eq!(
            geometry(100.0, 400.0, 96.0, 0.0, 100.0),
            Some((5.0, 0.0, 25.0, 100.0))
        );
        assert_eq!(geometry(0.0, 10.0, 96.0, 0.0, 60.0), None);
    }
}
