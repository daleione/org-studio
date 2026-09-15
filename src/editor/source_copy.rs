//! Copy code blocks without changing selection or document state.
use super::*;
use crate::components::selection_style::HOVER_BACKGROUND;
use crate::document::{
    DocumentFormat, LineCursor,
    markdown::{fence_close, fence_open},
};
use gpui::{
    Corners, Edges, Hitbox, HitboxBehavior, ShapedLine, TextAlign, TextRun, point, quad, size,
};

#[derive(Clone, Copy)]
pub(super) struct CopyHit {
    pub bounds: Bounds<Pixels>,
    pub revision: Revision,
    pub offset: ByteOffset,
}
#[derive(Default)]
pub(super) struct SourceCopy {
    pub buttons: Vec<CopyHit>,
    pub feedback: Option<(Revision, ByteOffset)>,
    hovered: Option<ByteOffset>,
    press: Option<(CopyHit, Point<Pixels>)>,
    task: Option<Task<()>>,
    hover_task: Option<Task<()>>,
    tooltip_visible: bool,
}

pub(super) struct CopyButtonPaint {
    pub hit: CopyHit,
    hitbox: Hitbox,
    check: Option<ShapedLine>,
}
impl CopyButtonPaint {
    pub fn new(
        bounds: Bounds<Pixels>,
        revision: Revision,
        offset: ByteOffset,
        copied: bool,
        window: &mut Window,
    ) -> Self {
        let check = copied.then(|| {
            window.text_system().shape_line(
                "✓".into(),
                px(14.),
                &[TextRun {
                    len: "✓".len(),
                    font: gpui::font(".SystemUIFont"),
                    color: rgb(current_theme().link).into(),
                    ..Default::default()
                }],
                None,
            )
        });
        Self {
            hit: CopyHit {
                bounds,
                revision,
                offset,
            },
            hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
            check,
        }
    }
    pub fn paint(&self, window: &mut Window, cx: &mut App) {
        let hovered = self.hitbox.is_hovered(window);
        if hovered {
            window.set_cursor_style(gpui::CursorStyle::PointingHand, &self.hitbox);
        }
        let theme = current_theme();
        if hovered {
            window.paint_quad(quad(
                self.hit.bounds,
                Corners::all(px(5.)),
                rgb(HOVER_BACKGROUND),
                Edges::default(),
                gpui::transparent_black(),
                gpui::BorderStyle::default(),
            ));
        }
        if let Some(check) = &self.check {
            let _ = check.paint(
                point(
                    self.hit.bounds.center().x - check.width() / 2.,
                    self.hit.bounds.top(),
                ),
                self.hit.bounds.size.height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        } else {
            let color = if hovered {
                theme.link
            } else {
                theme.foreground_dim
            };
            // A transparent 14 px copy symbol; the rear sheet stops at the front
            // sheet instead of painting an opaque patch over the code background.
            let origin = self.hit.bounds.center() - point(px(7.), px(7.));
            let mut rear = gpui::PathBuilder::stroke(px(1.2));
            for (index, (x, y)) in [(4., 10.), (1., 10.), (1., 1.), (10., 1.), (10., 4.)]
                .into_iter()
                .enumerate()
            {
                let p = origin + point(px(x), px(y));
                if index == 0 {
                    rear.move_to(p);
                } else {
                    rear.line_to(p);
                }
            }
            if let Ok(path) = rear.build() {
                window.paint_path(path, rgb(color));
            }
            window.paint_quad(quad(
                Bounds::new(origin + point(px(4.), px(4.)), size(px(9.), px(9.))),
                Corners::all(px(1.5)),
                gpui::transparent_black(),
                Edges::all(px(1.2)),
                rgb(color),
                gpui::BorderStyle::default(),
            ));
        }
    }
}

/// Read only the chosen block, retaining its exact body bytes and line endings.
fn block_body(
    snapshot: &DocumentSnapshot,
    format: DocumentFormat,
    offset: ByteOffset,
) -> Option<String> {
    let mut lines = LineCursor::within(snapshot, ByteRange::new(offset.0, snapshot.len_bytes()))?;
    let opening = lines.next_line()?;
    let header = opening.text.trim();
    let logical = opening.text.trim_end_matches(['\r', '\n']);
    let fence = match format {
        DocumentFormat::Org => {
            if !header
                .split_whitespace()
                .next()?
                .eq_ignore_ascii_case("#+begin_src")
            {
                return None;
            }
            None
        }
        DocumentFormat::Markdown => Some(fence_open(logical)?),
    };
    let start = opening.range.end;
    let mut end = ByteOffset(snapshot.len_bytes());
    while let Some(line) = lines.next_line() {
        let closed = if let Some((marker, count, _)) = &fence {
            let logical = line.text.trim_end_matches(['\r', '\n']);
            fence_close(logical, *marker, *count)
        } else {
            line.text.trim().eq_ignore_ascii_case("#+end_src")
        };
        if closed {
            end = line.range.start;
            break;
        }
    }
    Some(snapshot.copy_range(ByteRange::new(start.0, end.0)))
}

impl SemanticEditor {
    pub(super) fn source_copy_down(&mut self, event: &MouseDownEvent) -> bool {
        self.source_copy.press = self
            .source_copy
            .buttons
            .iter()
            .find(|hit| hit.bounds.contains(&event.position))
            .copied()
            .map(|hit| (hit, event.position));
        self.source_copy.press.is_some()
    }
    pub(super) fn source_copy_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let hovered = self
            .source_copy
            .buttons
            .iter()
            .find(|hit| hit.bounds.contains(&position))
            .map(|hit| hit.offset);
        if hovered != self.source_copy.hovered {
            self.source_copy.hovered = hovered;
            self.source_copy.hover_task = None;
            self.source_copy.tooltip_visible = false;
            if hovered.is_some() {
                let executor = cx.background_executor().clone();
                self.source_copy.hover_task = Some(cx.spawn(async move |this, cx| {
                    executor.timer(Duration::from_millis(500)).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.source_copy.hovered == hovered {
                            this.source_copy.tooltip_visible = true;
                            cx.notify();
                        }
                    });
                }));
            }
            cx.notify();
        }
        if self.source_copy.press.is_some_and(|(_, start)| {
            (position.x - start.x).abs() > px(4.) || (position.y - start.y).abs() > px(4.)
        }) {
            self.source_copy.press = None;
        }
    }
    pub(super) fn source_copy_tooltip(&self, window: &Window) -> Option<gpui::AnyElement> {
        if !self.source_copy.tooltip_visible {
            return None;
        }
        let hit = self.source_copy.buttons.iter().find(|hit| {
            Some(hit.offset) == self.source_copy.hovered
                && hit.bounds.contains(&window.mouse_position())
        })?;
        let copied = self.source_copy.feedback == Some((hit.revision, hit.offset));
        let theme = current_theme();
        let label = self.ui_language.text(if copied {
            "inline.code_copied"
        } else {
            "inline.copy_code"
        });
        let width = window
            .text_system()
            .shape_line(
                label.into(),
                px(11.),
                &[TextRun {
                    len: label.len(),
                    font: gpui::font(".SystemUIFont"),
                    ..Default::default()
                }],
                None,
            )
            .width()
            + px(18.);
        let viewport = self.viewport?;
        let y = if hit.bounds.bottom() + px(34.) <= viewport.bottom() {
            hit.bounds.bottom() + px(6.)
        } else {
            hit.bounds.top() - px(34.)
        };
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(point(
                        (hit.bounds.right() - width).max(viewport.left() + px(8.)),
                        y,
                    ))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id("source-copy-tooltip")
                            .debug_selector(|| "source-copy-tooltip".into())
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .shadow_md()
                            .bg(rgb(theme.background))
                            .border_1()
                            .border_color(rgb(theme.border))
                            .font_family(".SystemUIFont")
                            .text_size(px(11.))
                            .line_height(px(16.))
                            .text_color(rgb(theme.foreground))
                            .child(label),
                    ),
            )
            .with_priority(11)
            .into_any_element(),
        )
    }
    pub(super) fn source_copy_up(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((hit, _)) = self.source_copy.press.take() else {
            return;
        };
        let snapshot = self.snapshot(cx);
        if !hit.bounds.contains(&position) || snapshot.revision() != hit.revision {
            return;
        }
        let format = DocumentFormat::from_path(self.session.read(cx).syntax_path());
        let executor = cx.background_executor().clone();
        self.source_copy.task = Some(cx.spawn(async move |this, cx| {
            let body = executor
                .spawn(async move { block_body(&snapshot, format, hit.offset) })
                .await;
            let Some(body) = body else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(body));
                this.source_copy.feedback = Some((hit.revision, hit.offset));
                cx.notify();
            });
            executor.timer(Duration::from_millis(1600)).await;
            let _ = this.update(cx, |this, cx| {
                this.source_copy.feedback = None;
                cx.notify();
            });
        }));
    }
}

#[cfg(test)]
mod tests;
