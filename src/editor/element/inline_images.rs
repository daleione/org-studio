//! Resolves the inline image previews visible in the current frame,
//! publishing measured dimensions back to the editor line layout.
use std::collections::HashMap;

use crate::document::DocumentSnapshot;
use crate::editor::InlineImageMetrics;

use crate::editor::inline_image::{image_sizing, resolved_image_size};

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
    let sizing = {
        let editor = host.read(cx);
        image_sizing(
            wrap_width,
            f32::from(bounds.size.height),
            editor.font_size_px(),
        )
    };
    let linked = host
        .read(cx)
        .plots
        .lines
        .iter()
        .filter(|(_, plot)| plot.entry.source.image_link.is_some())
        .map(|(line, plot)| (*line, plot.clone()))
        .collect::<HashMap<_, _>>();
    let inline_image_candidates = {
        let editor = host.read(cx);
        let document_path = editor.session.read(cx).syntax_path().to_path_buf();
        if crate::document::DocumentFormat::from_path(&document_path)
            == crate::document::DocumentFormat::Org
            && crate::syntax_highlighting::language_for_path(&document_path).is_none()
        {
            let visible_lines = editor.animated_visible_line_range(
                snapshot,
                editor.scroll_y,
                f32::from(bounds.size.height),
            );
            visible_lines
                .filter(|line| !editor.display_map.is_hidden(*line))
                .filter_map(|line| {
                    if linked.contains_key(&line) {
                        return None;
                    }
                    let range = snapshot.line_content_range(LineIndex(line)).ok()?;
                    if !editor.previews_inline_image_at(range.start) {
                        return None;
                    }
                    let text = snapshot.copy_range(range);
                    let target = crate::org_syntax::standalone_image_path(&text)?;
                    let path = crate::preview::resolve_image_path(&document_path, target);
                    let cached = editor.cached_inline_image_render(&path);
                    let spec = editor.inline_image_attribute_spec(snapshot, range.start);
                    if let Some((_, dimensions)) = &cached {
                        editor.inline_image_line_dimensions.borrow_mut().insert(
                            line,
                            InlineImageMetrics {
                                plot: None,
                                line_start: range.start.0,
                                source: *dimensions,
                                spec: spec.clone(),
                            },
                        );
                    }
                    Some((line, range.start, path, cached, spec))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    };

    let mut images = inline_image_candidates
        .into_iter()
        .filter_map(|(line, line_start, path, cached, spec)| {
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
            // The cache keeps the authored attributes only: a drag is layered on
            // top at resolve time so it can never re-enter this notify path.
            let metrics = InlineImageMetrics {
                plot: None,
                line_start: line_start.0,
                source: dimensions,
                spec,
            };
            if previous_dimensions != Some(dimensions) {
                host.update(cx, |editor, cx| {
                    editor
                        .inline_image_line_dimensions
                        .borrow_mut()
                        .insert(line, metrics.clone());
                    editor.display_map.invalidate_line_layout(line);
                    cx.notify();
                });
            } else {
                host.read(cx)
                    .inline_image_line_dimensions
                    .borrow_mut()
                    .insert(line, metrics.clone());
            }
            let (width, height) = resolved_image_size(host.read(cx), &metrics, &sizing);
            Some((
                line,
                InlineImage {
                    image,
                    width,
                    height,
                    line_start,
                },
            ))
        })
        .collect::<std::collections::HashMap<_, _>>();
    for (line, plot) in linked {
        let line_start = plot.entry.source.image_link.unwrap().start;
        if !host.read(cx).previews_inline_image_at(line_start) {
            continue;
        }
        let Some(rendered) = plot
            .state
            .as_ref()
            .and_then(crate::plot::PlotRenderState::rendered)
        else {
            continue;
        };
        let metrics = InlineImageMetrics {
            plot: Some(plot.entry.clone()),
            line_start: line_start.0,
            source: plot.entry.dimensions(),
            spec: plot.entry.source.image_attributes.clone(),
        };
        host.read(cx)
            .inline_image_line_dimensions
            .borrow_mut()
            .insert(line, metrics.clone());
        let (width, height) = resolved_image_size(host.read(cx), &metrics, &sizing);
        images.insert(
            line,
            InlineImage {
                image: rendered.image.clone(),
                width,
                height,
                line_start,
            },
        );
    }
    images
}
