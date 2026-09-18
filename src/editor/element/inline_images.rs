//! Resolves the inline image previews visible in the current frame,
//! publishing measured dimensions back to the editor line layout.
use std::collections::HashMap;

use crate::document::DocumentSnapshot;

use super::state::InlineImage;
use super::*;

pub(super) fn resolve_inline_images(
    host: &gpui::Entity<SemanticEditor>,
    snapshot: &DocumentSnapshot,
    bounds: Bounds<Pixels>,
    wrap_width: f32,
    window: &mut Window,
    cx: &mut App,
) -> HashMap<u64, InlineImage> {
    let inline_image_candidates = {
        let editor = host.read(cx);
        let document_path = editor.session.read(cx).syntax_path().to_path_buf();
        if crate::document::DocumentFormat::from_path(&document_path)
            == crate::document::DocumentFormat::Org
        {
            let visible_lines = editor.animated_visible_line_range(
                snapshot,
                editor.scroll_y,
                f32::from(bounds.size.height),
            );
            visible_lines
                .filter(|line| !editor.display_map.is_hidden(*line))
                .filter_map(|line| {
                    let range = snapshot.line_content_range(LineIndex(line)).ok()?;
                    if !editor.previews_inline_image_at(range.start) {
                        return None;
                    }
                    let text = snapshot.copy_range(range);
                    let target = crate::org_syntax::standalone_image_path(&text)?;
                    let path = crate::preview::resolve_image_path(&document_path, target);
                    let cached = editor.cached_inline_image_render(&path);
                    if let Some((_, dimensions)) = &cached {
                        editor
                            .inline_image_line_dimensions
                            .borrow_mut()
                            .insert(line, (range.start.0, dimensions.0, dimensions.1));
                    }
                    Some((line, range.start.0, path, cached))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    };

    inline_image_candidates
        .into_iter()
        .filter_map(|(line, line_start, path, cached)| {
            let previous_dimensions = cached.as_ref().map(|(_, dimensions)| *dimensions);
            let path: Arc<std::path::Path> = path.into();
            let loaded =
                window.use_asset::<crate::editor::image_loader::EditorImageLoader>(&path, cx);
            let (image, dimensions) = match loaded {
                Some(Ok(image)) => host
                    .read(cx)
                    .accept_inline_image_render(path.as_ref(), image),
                Some(Err(_)) => {
                    let changed = host.read(cx).fail_inline_image_render(path.as_ref());
                    if changed {
                        host.update(cx, |editor, cx| {
                            editor
                                .inline_image_line_dimensions
                                .borrow_mut()
                                .remove(&line);
                            editor.display_map.invalidate_line_layout(line);
                            cx.notify();
                        });
                    }
                    return None;
                }
                None => cached?,
            };
            if previous_dimensions != Some(dimensions) {
                host.update(cx, |editor, cx| {
                    editor
                        .inline_image_line_dimensions
                        .borrow_mut()
                        .insert(line, (line_start, dimensions.0, dimensions.1));
                    editor.display_map.invalidate_line_layout(line);
                    cx.notify();
                });
            } else {
                host.read(cx)
                    .inline_image_line_dimensions
                    .borrow_mut()
                    .insert(line, (line_start, dimensions.0, dimensions.1));
            }
            let (width, height) = crate::preview::fitted_image_size(
                dimensions.0,
                dimensions.1,
                wrap_width.min(INLINE_IMAGE_MAX_WIDTH),
            );
            Some((line, (image, width, height, path)))
        })
        .collect::<std::collections::HashMap<_, _>>()
}
