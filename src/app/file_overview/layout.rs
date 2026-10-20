use gpui::{Bounds, Pixels, Point, Size, point, px, size};

pub(super) const GAP: f32 = 24.0;
pub(super) const CAPTION: f32 = 52.0;
pub(super) const FOOTER: f32 = 28.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct GridLayout {
    pub columns: usize,
    pub card_width: f32,
    pub preview_height: f32,
    pub row_height: f32,
    pub header: f32,
    pub margin: f32,
    pub left: f32,
    pub height: f32,
    pub total_height: f32,
}

impl GridLayout {
    #[cfg(test)]
    pub fn new(viewport: Size<Pixels>, count: usize) -> Self {
        let source = size(
            viewport.width,
            (viewport.height - px(super::super::TITLEBAR_HEIGHT)).max(px(1.)),
        );
        Self::with_sources(viewport, &vec![source; count])
    }

    pub fn with_sources(viewport: Size<Pixels>, sources: &[Size<Pixels>]) -> Self {
        let count = sources.len();
        let width = f32::from(viewport.width);
        let narrow = width < 680.0;
        let columns = (if narrow {
            1
        } else if width < 1100.0 {
            2
        } else {
            3
        })
        .min(count.max(1));
        let margin = if narrow { 20.0 } else { 32.0 };
        let available_width = ((width - margin * 2.0 - GAP * (columns - 1) as f32)
            / columns as f32)
            .clamp(1.0, 400.0);
        let header = if narrow { 124.0 } else { 88.0 };
        let height =
            (f32::from(viewport.height) - super::super::TITLEBAR_HEIGHT - header - FOOTER).max(1.0);
        let available_height = (height - CAPTION - 2.0 * margin).max(1.0);
        let limit = Bounds::new(
            point(px(0.), px(0.)),
            size(px(available_width), px(available_height)),
        );
        let (card_width, preview_height) =
            sources
                .iter()
                .fold((1.0_f32, 1.0_f32), |(width, height), source| {
                    let fitted = contain(*source, limit);
                    (
                        width.max(f32::from(fitted.size.width)),
                        height.max(f32::from(fitted.size.height)),
                    )
                });
        let row_height = preview_height + CAPTION + GAP;
        let rows = count.div_ceil(columns);
        Self {
            columns,
            card_width,
            preview_height,
            row_height,
            header,
            margin,
            left: ((width - card_width * columns as f32 - GAP * (columns - 1) as f32) / 2.0)
                .max(0.0),
            height,
            total_height: if rows == 0 {
                0.0
            } else {
                margin + rows as f32 * row_height
            },
        }
    }

    /// The visible card is the document itself; the surrounding grid slot has
    /// no painted background or border. Mixed aspect ratios share row slots.
    pub fn document_preview(
        &self,
        index: usize,
        scroll: Pixels,
        source: Size<Pixels>,
    ) -> Bounds<Pixels> {
        let slot = self.preview(index, scroll);
        let mut bounds = contain(source, slot);
        bounds.origin.y = slot.origin.y;
        bounds
    }

    pub fn preview(&self, index: usize, scroll: Pixels) -> Bounds<Pixels> {
        Bounds::new(
            point(
                px(self.left + (index % self.columns) as f32 * (self.card_width + GAP)),
                px(super::super::TITLEBAR_HEIGHT
                    + self.header
                    + self.margin
                    + (index / self.columns) as f32 * self.row_height)
                    + scroll,
            ),
            size(px(self.card_width), px(self.preview_height)),
        )
    }

    pub fn visible_range(&self, count: usize, scroll: Pixels) -> std::ops::Range<usize> {
        self.visible_range_in(
            count,
            Bounds::new(
                point(
                    px(0.),
                    px(super::super::TITLEBAR_HEIGHT + self.header) - self.clamp_scroll(scroll),
                ),
                size(px(1.), px(self.height)),
            ),
        )
    }

    pub fn visible_range_in(
        &self,
        count: usize,
        viewport: Bounds<Pixels>,
    ) -> std::ops::Range<usize> {
        let top = f32::from(viewport.top()) - super::super::TITLEBAR_HEIGHT - self.header;
        let first = ((top - self.margin).max(0.0) / self.row_height).floor() as usize;
        let last =
            ((top + f32::from(viewport.size.height)).max(0.) / self.row_height).ceil() as usize + 1;
        (first.saturating_sub(1) * self.columns).min(count)..(last * self.columns).min(count)
    }

    pub fn clamp_scroll(&self, scroll: Pixels) -> Pixels {
        px(f32::from(scroll).clamp(-(self.total_height - self.height).max(0.0), 0.0))
    }

    pub fn reveal(&self, index: usize, scroll: Pixels) -> Pixels {
        let y = self.margin + (index / self.columns) as f32 * self.row_height;
        let height = self.preview_height + CAPTION;
        let offset = f32::from(scroll);
        px(if y + offset < 0.0 {
            -y
        } else if y + height + offset > self.height {
            (self.height - y - height).min(0.0)
        } else {
            offset
        })
    }
}

pub(super) fn contain(source: Size<Pixels>, target: Bounds<Pixels>) -> Bounds<Pixels> {
    let scale = (f32::from(target.size.width) / f32::from(source.width).max(1.0))
        .min(f32::from(target.size.height) / f32::from(source.height).max(1.0));
    let fitted = size(source.width * scale, source.height * scale);
    Bounds::new(
        target.center() - point(fitted.width / 2.0, fitted.height / 2.0),
        fitted,
    )
}

/// Shrink around the document centre before gathering it into its grid slot.
/// Expanding traverses the same path backwards, including during cancellation.
pub(super) fn document_transition(
    a: Bounds<Pixels>,
    b: Bounds<Pixels>,
    progress: f32,
) -> Bounds<Pixels> {
    let p = progress.clamp(0., 1.);
    let gather = |x: f32| x * x * (2. - x);
    let centre_progress = if (b.size.width - a.size.width).abs() < px(0.001) {
        p
    } else if b.size.width < a.size.width {
        gather(p)
    } else {
        1. - gather(1. - p)
    };
    let dimensions = size(
        a.size.width + (b.size.width - a.size.width) * p,
        a.size.height + (b.size.height - a.size.height) * p,
    );
    let centre = a.center() + (b.center() - a.center()) * centre_progress;
    Bounds::new(
        centre - point(dimensions.width / 2., dimensions.height / 2.),
        dimensions,
    )
}

/// Move the whole grid with the active document. Neighbours enter from beyond
/// the viewport instead of waiting behind a separately shrinking page.
pub(super) fn place_in_transition(
    bounds: Bounds<Pixels>,
    anchor: Bounds<Pixels>,
    document: Bounds<Pixels>,
) -> Bounds<Pixels> {
    let scale = document.size.width / anchor.size.width;
    Bounds::new(
        document.origin + (bounds.origin - anchor.origin) * scale,
        size(bounds.size.width * scale, bounds.size.height * scale),
    )
}

pub(super) fn point_in(bounds: Bounds<Pixels>, position: Point<Pixels>) -> bool {
    bounds.contains(&position)
}

/// Cover only the space outside a document during its live-view handoff.
/// Keeping the interior clear avoids tinting the real page as the cover fades.
pub(super) fn surrounding_regions(
    outer: Bounds<Pixels>,
    document: Bounds<Pixels>,
) -> Vec<Bounds<Pixels>> {
    let inner = document.intersect(&outer);
    if inner.is_empty() {
        return vec![outer];
    }
    [
        Bounds::new(
            outer.origin,
            size(outer.size.width, inner.top() - outer.top()),
        ),
        Bounds::new(
            point(outer.left(), inner.bottom()),
            size(outer.size.width, outer.bottom() - inner.bottom()),
        ),
        Bounds::new(
            point(outer.left(), inner.top()),
            size(inner.left() - outer.left(), inner.size.height),
        ),
        Bounds::new(
            point(inner.right(), inner.top()),
            size(outer.right() - inner.right(), inner.size.height),
        ),
    ]
    .into_iter()
    .filter(|region| !region.is_empty())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_breakpoints_empty_state_and_virtualization_are_bounded() {
        for (width, columns) in [(390., 1), (679., 1), (680., 2), (1099., 2), (1100., 3)] {
            let grid = GridLayout::new(size(px(width), px(800.)), 200);
            assert_eq!(grid.columns, columns);
            assert!(grid.visible_range(200, px(0.)).len() < 20);
            let last = grid.reveal(199, px(0.));
            assert!(grid.visible_range(200, last).contains(&199));
        }
        assert_eq!(GridLayout::new(size(px(1440.), px(900.)), 1).columns, 1);
        assert!(
            GridLayout::new(size(px(390.), px(800.)), 0)
                .visible_range(0, px(0.))
                .is_empty()
        );
    }

    #[test]
    fn document_shrinks_before_gathering_and_expands_along_the_same_path() {
        let source = Bounds::new(point(px(240.), px(38.)), size(px(600.), px(1000.)));
        let card = Bounds::new(point(px(40.), px(150.)), size(px(180.), px(300.)));
        assert_eq!(document_transition(source, card, 0.), source);
        assert_eq!(document_transition(source, card, 1.), card);
        for step in 0..=100 {
            let p = step as f32 / 100.;
            let inward = document_transition(source, card, p);
            let outward = document_transition(card, source, 1. - p);
            assert!((inward.origin.x - outward.origin.x).abs() < px(0.001));
            assert!((inward.origin.y - outward.origin.y).abs() < px(0.001));
            assert!((inward.size.width - outward.size.width).abs() < px(0.001));
            assert!((f32::from(inward.size.width / inward.size.height) - 0.6).abs() < 0.0001);
        }
        let early = document_transition(source, card, 0.1);
        assert!(early.size.width < source.size.width);
        assert!(
            (early.center().x - source.center().x).abs()
                < (card.center().x - source.center().x).abs() * 0.1
        );
    }

    #[test]
    fn neighbours_share_the_document_scale_and_stay_beside_it() {
        let viewport = size(px(1200.), px(900.));
        let source = Bounds::new(point(px(200.), px(38.)), size(px(1000.), px(862.)));
        let grid = GridLayout::with_sources(viewport, &[source.size; 3]);
        let anchor = grid.document_preview(0, px(0.), source.size);
        let neighbour = grid.document_preview(1, px(0.), source.size);
        for step in 0..=100 {
            let p = step as f32 / 100.;
            let document = document_transition(source, anchor, p);
            let placed = place_in_transition(neighbour, anchor, document);
            let scale = document.size.width / anchor.size.width;
            let active = place_in_transition(anchor, anchor, document);
            assert!((active.left() - document.left()).abs() < px(0.001));
            assert!((active.bottom() - document.bottom()).abs() < px(0.001));
            assert!((placed.left() - document.right() - px(GAP) * scale).abs() < px(0.001));
            assert!((placed.size.width - document.size.width).abs() < px(0.001));
            let reverse = place_in_transition(
                neighbour,
                anchor,
                document_transition(anchor, source, 1. - p),
            );
            assert!((placed.origin.x - reverse.origin.x).abs() < px(0.001));
        }
        assert!(place_in_transition(neighbour, anchor, source).left() >= viewport.width);
        assert_eq!(place_in_transition(neighbour, anchor, anchor), neighbour);
    }

    #[test]
    fn zoomed_virtualization_includes_every_visible_row_after_scrolling() {
        let viewport = size(px(900.), px(1050.));
        let grid = GridLayout::new(viewport, 200);
        let body = Bounds::new(
            point(px(0.), px(38.)),
            size(viewport.width, viewport.height - px(38.)),
        );
        let source = body;
        for index in [0, 65, 199] {
            let scroll = grid.reveal(index, px(0.));
            let anchor = grid.document_preview(index, scroll, source.size);
            for p in [0., 0.25, 0.5, 0.75, 1.] {
                let document = document_transition(source, anchor, p);
                let visible = place_in_transition(body, document, anchor);
                let range = grid.visible_range_in(
                    200,
                    Bounds::new(visible.origin - point(px(0.), scroll), visible.size),
                );
                assert!(range.len() < 20);
                for other in 0..200 {
                    let placed = place_in_transition(grid.preview(other, scroll), anchor, document);
                    if !placed.intersect(&body).is_empty() {
                        assert!(
                            range.contains(&other),
                            "row {other}, selected {index}, p={p}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn portrait_and_landscape_frames_are_compact_and_keep_their_own_proportions() {
        let portrait = size(px(600.), px(1000.));
        let wide = size(px(1600.), px(900.));
        for viewport in [size(px(1440.), px(1000.)), size(px(390.), px(800.))] {
            let grid = GridLayout::with_sources(viewport, &[portrait, wide]);
            for (index, source) in [portrait, wide].into_iter().enumerate() {
                let frame = grid.document_preview(index, px(0.), source);
                assert!(
                    (f32::from(frame.size.width / frame.size.height)
                        - f32::from(source.width / source.height))
                    .abs()
                        < 0.0001
                );
                assert!(f32::from(frame.size.height) + CAPTION <= grid.height);
            }
        }
        let grid = GridLayout::with_sources(size(px(1440.), px(1000.)), &[portrait, portrait]);
        let left = grid.document_preview(0, px(0.), portrait);
        let right = grid.document_preview(1, px(0.), portrait);
        assert!((f32::from(right.left() - left.right()) - GAP).abs() < 0.001);
        let narrow = GridLayout::with_sources(size(px(390.), px(800.)), &[portrait]);
        assert!(narrow.reveal(0, px(0.)) >= narrow.clamp_scroll(px(-10000.)));
    }

    #[test]
    fn handoff_cover_never_tints_the_document_or_overlaps_itself() {
        let outer = Bounds::new(point(px(0.), px(0.)), size(px(1200.), px(900.)));
        let document = Bounds::new(point(px(200.), px(50.)), size(px(900.), px(800.)));
        let regions = surrounding_regions(outer, document);
        let area =
            |bounds: Bounds<Pixels>| f32::from(bounds.size.width) * f32::from(bounds.size.height);
        assert_eq!(
            regions.iter().copied().map(area).sum::<f32>(),
            area(outer) - area(document)
        );
        for (index, region) in regions.iter().enumerate() {
            assert!(region.intersect(&document).is_empty());
            for other in &regions[index + 1..] {
                assert!(region.intersect(other).is_empty());
            }
        }
        assert!(surrounding_regions(outer, outer).is_empty());
    }
}
