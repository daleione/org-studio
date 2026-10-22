use super::*;
use crate::{document::Selection, plot::render_plot_surface};
use gpui::{AnyElement, AvailableSpace, IntoElement, canvas, div, prelude::*};

pub(super) fn prepaint(
    host: &gpui::Entity<SemanticEditor>,
    rows: &[PaintRow],
    gutter_right: Pixels,
    handles: &mut Vec<InlineImageResizeHandlePaint>,
    window: &mut Window,
    cx: &mut App,
) -> (Vec<AnyElement>, Vec<AnyElement>) {
    if host.read(cx).fold_animation.is_some() {
        return (Vec::new(), Vec::new());
    }
    let presentations = host.read(cx).plots.lines.clone();
    let chinese = host.read(cx).ui_language == crate::i18n::Language::Chinese;
    let mut output_errors = Vec::new();
    let plots = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let presentation = presentations.get(&row.hit.line.0)?;
            if presentation.entry.source.image_link.is_some() {
                let error = crate::plot::render_plot_output_error(presentation)?;
                let image = row.inline_image.as_ref()?;
                let mut overlay = div()
                    .relative()
                    .w(image.bounds.size.width)
                    .h(image.bounds.size.height)
                    .child(error)
                    .into_any_element();
                overlay.prepaint_as_root(
                    image.bounds.origin,
                    size(
                        AvailableSpace::Definite(image.bounds.size.width),
                        AvailableSpace::Definite(image.bounds.size.height),
                    ),
                    window,
                    cx,
                );
                output_errors.push(overlay);
                return None;
            }
            let origin = point(row.hit.text_origin_x, row.hit.visible_bottom + px(6.0));
            let marker_x = gutter_right - px(GUTTER_PADDING) - row.gutter_layout.width() / 2.;
            let marker_top = if row.visual_rows > 1 {
                row.hit.visible_bottom + px(3.0)
            } else {
                row.gutter_number_span().1 + px(3.0)
            };
            let marker_bottom = rows
                .get(index + 1)
                .filter(|next| next.inline_image.is_none())
                .map_or(origin.y + px(presentation.height), |next| {
                    next.gutter_number_span().0 - px(3.0)
                });
            handles.push(InlineImageResizeHandlePaint::new(
                Bounds::new(
                    origin,
                    size(px(presentation.width), px(presentation.height)),
                ),
                row.hit.line.0,
                presentation.entry.source.declaration.start,
                Some(presentation.entry.id),
                window,
            ));
            let entry = presentation.entry.clone();
            let document = presentation.document.clone();
            let demand = presentation.demand.clone();
            let selected = presentation.entry.display.lock().unwrap().selected;
            let editor = host.downgrade();
            let source_editor = editor.clone();
            let declaration = entry.source.declaration;
            let mut element = div()
                .relative()
                .w(px(presentation.width))
                .h(px(presentation.height))
                .child(render_plot_surface(presentation, chinese).on_mouse_down(
                    gpui::MouseButton::Left,
                    move |event, window, cx| {
                        if let Some(demand) = &demand {
                            demand.retry();
                        }
                        let _ = editor.update(cx, |editor, cx| {
                            if !editor.image_resize_down(event, cx) {
                                document
                                    .update(cx, |document, cx| document.select(Some(entry.id), cx));
                            }
                            window.focus(&editor.focus_handle, cx);
                            cx.notify();
                        });
                        cx.stop_propagation();
                    },
                ))
                .child(
                    div()
                        .id(("plot-source", presentation.entry.id.0))
                        .absolute()
                        .left(marker_x - origin.x - px(10.0))
                        .top(marker_top - origin.y)
                        .w(px(20.0))
                        .h((marker_bottom - marker_top).max(px(0.0)))
                        .flex()
                        .justify_center()
                        .cursor_pointer()
                        .child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, window, _| {
                                    let stroke = px(1.0);
                                    let dash_length = px(8.0);
                                    let gap = px(6.0);
                                    let theme = current_theme();
                                    let (color, alpha) = if selected {
                                        (theme.link, 0xcc)
                                    } else {
                                        (theme.line_number, 0x88)
                                    };
                                    let color = rgba((color << 8) | alpha);
                                    let mut y = bounds.top();
                                    while y < bounds.bottom() {
                                        crate::document_preview::quad(
                                            window,
                                            fill(
                                                Bounds::new(
                                                    point(bounds.left(), y),
                                                    size(
                                                        stroke,
                                                        dash_length.min(bounds.bottom() - y),
                                                    ),
                                                ),
                                                color,
                                            )
                                            .corner_radii(stroke / 2.),
                                        );
                                        y += dash_length + gap;
                                    }
                                },
                            )
                            .w(px(1.0))
                            .h_full(),
                        )
                        .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                            let _ = source_editor.update(cx, |editor, cx| {
                                editor.set_selection(Selection::caret(declaration.start), cx);
                                window.focus(&editor.focus_handle, cx);
                            });
                            cx.stop_propagation();
                        }),
                )
                .into_any_element();
            element.prepaint_as_root(
                origin,
                size(
                    AvailableSpace::Definite(px(presentation.width)),
                    AvailableSpace::Definite(px(presentation.height)),
                ),
                window,
                cx,
            );
            Some(element)
        })
        .collect();
    (plots, output_errors)
}
