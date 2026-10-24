//! Shared GPUI geometry for text, sized inline code and atomic formula objects.
use std::ops::Range;

use gpui::{
    App, Bounds, Pixels, Point, ShapedLine, Size, TextAlign, TextRun, Window, WindowTextSystem,
    point, px, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{InlineSpan, PreviewStyle};

/// Logical-pixel metrics shared by virtual measurement and the rendered text element.
#[derive(Clone, Copy)]
pub(in crate::preview) struct InlineTextMetrics {
    pub font_size: f32,
    pub code_font_size: f32,
    pub line_height: f32,
    pub wrap_width: f32,
    pub foreground: u32,
}

pub(crate) fn needs_sized_inline(spans: &[InlineSpan], style: PreviewStyle) -> bool {
    spans
        .iter()
        .any(|span| span.kind == super::InlineKind::Latex)
        || (style.typography.inline_code_size.is_some() && spans.iter().any(|span| span.is_code()))
}

/// Turn shaped byte boundaries into contiguous UTF-8 ranges, including an empty text row.
pub(in crate::preview) fn wrapped_ranges(text: &str, mut starts: Vec<usize>) -> Vec<Range<usize>> {
    starts.extend([0, text.len()]);
    starts.retain(|&index| text.is_char_boundary(index));
    starts.sort_unstable();
    starts.dedup();
    let mut ranges = starts
        .windows(2)
        .map(|pair| pair[0]..pair[1])
        .collect::<Vec<_>>();
    if ranges.is_empty() {
        ranges.push(0..text.len());
    }
    ranges
}

pub(crate) struct TextFragment {
    pub(crate) range: Range<usize>,
    pub(crate) line: ShapedLine,
    pub(crate) row: usize,
    pub(crate) x: Pixels,
}

pub(crate) struct InlineLayout {
    pub(crate) fragments: Vec<TextFragment>,
    formulas: Vec<FormulaFragment>,
    pub(crate) line_starts: Vec<usize>,
    pub(crate) line_height: Pixels,
    pub(crate) size: Size<Pixels>,
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) len: usize,
    text_align: TextAlign,
    row_offsets: Vec<Pixels>,
    ascent: Pixels,
    descent: Pixels,
}

struct FormulaFragment {
    range: Range<usize>,
    row: usize,
    x: Pixels,
    width: Pixels,
    height: Pixels,
    baseline: Pixels,
    centered: bool,
    image: std::sync::Arc<gpui::RenderImage>,
}

/// Shape whole word/style fragments, then split oversized words only at grapheme boundaries.
/// Both virtual-list measurement and painting use this layout, including its byte coordinates.
pub(crate) fn shape_inline(
    text: &str,
    runs: &[TextRun],
    spans: &[InlineSpan],
    math: &super::math::MathRow,
    metrics: InlineTextMetrics,
    text_system: &WindowTextSystem,
) -> InlineLayout {
    let _span = tracing::info_span!("reading.inline_layout").entered();
    let mut builder = InlineLayoutBuilder::new(metrics.wrap_width);
    let objects = math.objects(crate::math::MathStyle::new(
        metrics.font_size,
        metrics.foreground,
    ));
    let mut objects = objects.iter().peekable();
    let mut consumed = 0;
    let mut run_start = 0;
    for run in runs {
        let run_end = (run_start + run.len).min(text.len());
        let is_code = spans.iter().any(|span| {
            span.is_code() && span.range.start <= run_start && span.range.end >= run_end
        });
        let font_size = px(if is_code {
            metrics.code_font_size
        } else {
            metrics.font_size
        });
        let mut cursor = run_start.max(consumed);
        while cursor < run_end {
            if let Some(object) = objects.peek().filter(|object| object.range.start == cursor) {
                builder.append_formula(object);
                consumed = object.range.end;
                cursor = consumed;
                if object.display && cursor < text.len() && !text[cursor..].starts_with('\n') {
                    builder.new_line(cursor);
                }
                objects.next();
                continue;
            }
            let end = objects
                .peek()
                .map_or(run_end, |object| object.range.start.min(run_end));
            for (word_start, word) in text[cursor..end].split_word_bound_indices() {
                let start = cursor + word_start;
                if word.contains('\n') {
                    builder.new_line(start + word.len());
                    continue;
                }
                let mut word_run = run.clone();
                word_run.len = word.len();
                let shaped =
                    text_system.shape_line(word.to_owned().into(), font_size, &[word_run], None);
                builder.append_word(start, shaped);
            }
            cursor = end;
        }
        run_start = run_end;
    }
    builder.finish(text.len(), metrics.line_height)
}

/// Wrapping owns the cursor and updates fragment geometry in one place.
struct InlineLayoutBuilder {
    fragments: Vec<TextFragment>,
    formulas: Vec<FormulaFragment>,
    line_starts: Vec<usize>,
    wrap_width: Pixels,
    x: Pixels,
    max_width: Pixels,
    ascent: Pixels,
    descent: Pixels,
}

impl InlineLayoutBuilder {
    fn new(wrap_width: f32) -> Self {
        Self {
            fragments: Vec::new(),
            formulas: Vec::new(),
            line_starts: vec![0],
            wrap_width: px(wrap_width.max(1.0)),
            x: px(0.0),
            max_width: px(0.0),
            ascent: px(0.0),
            descent: px(0.0),
        }
    }

    fn new_line(&mut self, start: usize) {
        self.x = px(0.0);
        self.line_starts.push(start);
    }

    fn append_fragment(&mut self, start: usize, line: ShapedLine) {
        let width = line.width();
        self.fragments.push(TextFragment {
            range: start..start + line.len(),
            line,
            row: self.line_starts.len() - 1,
            x: self.x,
        });
        self.x += width;
        self.max_width = self.max_width.max(self.x);
    }

    fn append_word(&mut self, mut start: usize, mut line: ShapedLine) {
        self.ascent = self.ascent.max(line.ascent);
        self.descent = self.descent.max(line.descent);
        if self.x > px(0.0) && self.x + line.width() > self.wrap_width {
            self.new_line(start);
        }
        while line.len() > 0 && line.width() > self.wrap_width - self.x {
            let split = fitting_prefix(&line, self.wrap_width - self.x);
            let (prefix, suffix) = line.split_at(split);
            self.append_fragment(start, prefix);
            start += split;
            line = suffix;
            if line.len() > 0 {
                self.new_line(start);
            }
        }
        if line.len() > 0 {
            self.append_fragment(start, line);
        }
    }

    fn append_formula(&mut self, object: &super::math::FormulaObject) {
        let image = &object.rendered;
        let scale = (f32::from(self.wrap_width) / image.width.max(1.0)).min(1.0);
        let width = px(image.width * scale);
        let height = px(image.height * scale);
        let baseline = px(image.baseline * scale);
        if self.x > px(0.0) && (object.display || self.x + width > self.wrap_width) {
            self.new_line(object.range.start);
        }
        self.ascent = self.ascent.max(baseline);
        self.descent = self.descent.max(height - baseline);
        self.formulas.push(FormulaFragment {
            range: object.range.clone(),
            row: self.line_starts.len() - 1,
            x: self.x,
            width,
            height,
            baseline,
            centered: object.display,
            image: image.image.clone(),
        });
        self.x += width;
        self.max_width = self
            .max_width
            .max(if object.display && self.wrap_width < px(f32::MAX) {
                self.wrap_width
            } else {
                self.x
            });
    }

    fn finish(self, len: usize, configured_line_height: f32) -> InlineLayout {
        let line_height = px(configured_line_height).max(self.ascent + self.descent);
        let row_count = self.line_starts.len();
        InlineLayout {
            fragments: self.fragments,
            formulas: self.formulas,
            line_starts: self.line_starts,
            line_height,
            size: size(self.max_width.ceil(), line_height * row_count as f32),
            bounds: Bounds::default(),
            len,
            text_align: TextAlign::Left,
            row_offsets: vec![px(0.0); row_count],
            ascent: self.ascent,
            descent: self.descent,
        }
    }
}

/// Always consume one grapheme, even when a single glyph exceeds the column width.
fn fitting_prefix(line: &ShapedLine, available_width: Pixels) -> usize {
    let mut split = 0;
    for (index, grapheme) in line.text.grapheme_indices(true) {
        let end = index + grapheme.len();
        if split > 0 && line.x_for_index(end) > available_width {
            break;
        }
        split = end;
    }
    split
}

impl TextFragment {
    fn nearest_index(&self, x: Pixels) -> usize {
        // Include the end boundary. GPUI's glyph-only search jumps past the last glyph
        // when aligned coordinates round slightly, and may split combining characters.
        let distance = |index| f32::from((self.line.x_for_index(index) - x).abs());
        self.line
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain([self.line.len()])
            .min_by(|left, right| distance(*left).total_cmp(&distance(*right)))
            .unwrap_or(0)
    }
}

impl InlineLayout {
    /// A source match inside a rendered formula highlights the whole atomic object.
    pub(in crate::preview) fn visual_range(&self, mut range: Range<usize>) -> Range<usize> {
        if range.is_empty() {
            return range;
        }
        let first = self
            .formulas
            .partition_point(|formula| formula.range.end <= range.start);
        let end = self
            .formulas
            .partition_point(|formula| formula.range.start < range.end);
        if first < end {
            range.start = range.start.min(self.formulas[first].range.start);
            range.end = range.end.max(self.formulas[end - 1].range.end);
        }
        range
    }

    pub(in crate::preview) fn formula_count(&self, clip: Bounds<Pixels>) -> usize {
        self.formulas
            .iter()
            .filter(|formula| self.formula_bounds(formula).intersects(&clip))
            .count()
    }

    pub(in crate::preview) fn visible_range(&self, clip: Bounds<Pixels>) -> Range<usize> {
        let clip = clip.intersect(&self.bounds);
        let first = (((clip.top() - self.bounds.top()) / self.line_height)
            .floor()
            .max(0.0) as usize)
            .min(self.line_starts.len() - 1);
        let end = (((clip.bottom() - self.bounds.top()) / self.line_height)
            .ceil()
            .max(0.0) as usize)
            .min(self.line_starts.len());
        self.line_starts[first]..self.line_starts.get(end).copied().unwrap_or(self.len)
    }
    pub(in crate::preview) fn set_alignment(&mut self, align: TextAlign) {
        self.text_align = align;
    }

    pub(in crate::preview) fn set_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
        let mut widths = vec![px(0.0); self.line_starts.len()];
        for fragment in &self.fragments {
            widths[fragment.row] = widths[fragment.row].max(fragment.x + fragment.line.width());
        }
        for formula in &self.formulas {
            widths[formula.row] = widths[formula.row].max(formula.x + formula.width);
        }
        for (offset, width) in self.row_offsets.iter_mut().zip(widths) {
            let free = (bounds.size.width - width).max(px(0.0));
            *offset = match self.text_align {
                TextAlign::Center => free / 2.0,
                TextAlign::Right => free,
                TextAlign::Left => px(0.0),
            };
        }
    }

    pub(crate) fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        if index > self.len {
            return None;
        }
        let formula = self
            .formulas
            .iter()
            .find(|formula| formula.range.contains(&index))
            .or_else(|| {
                if self
                    .fragments
                    .iter()
                    .any(|fragment| fragment.range.contains(&index))
                {
                    return None;
                }
                self.formulas
                    .iter()
                    .find(|formula| formula.range.end == index)
            });
        if let Some(formula) = formula {
            let x = if index - formula.range.start <= formula.range.len() / 2 {
                px(0.0)
            } else {
                formula.width
            };
            return Some(
                self.bounds.origin
                    + point(
                        self.formula_x(formula) + x,
                        self.line_height * formula.row as f32,
                    ),
            );
        }
        let fragment = self
            .fragments
            .iter()
            .find(|fragment| fragment.range.contains(&index))
            .or_else(|| {
                self.fragments
                    .iter()
                    .rev()
                    .find(|fragment| fragment.range.end == index)
            });
        Some(fragment.map_or(self.bounds.origin, |fragment| {
            self.bounds.origin
                + point(
                    self.row_offsets[fragment.row]
                        + fragment.x
                        + fragment.line.x_for_index(index - fragment.range.start),
                    self.line_height * fragment.row as f32,
                )
        }))
    }

    pub(crate) fn index_for_position(&self, position: Point<Pixels>) -> Result<usize, usize> {
        let local = position - self.bounds.origin;
        let row = ((local.y / self.line_height).floor().max(0.0) as usize)
            .min(self.line_starts.len() - 1);
        let x = local.x - self.row_offsets[row];
        let text = self
            .fragments
            .iter()
            .filter(|fragment| fragment.row == row)
            .map(|fragment| {
                let index = fragment.nearest_index(x - fragment.x);
                (
                    (fragment.x + fragment.line.x_for_index(index) - x).abs(),
                    fragment.range.start + index,
                )
            });
        let formulas = self
            .formulas
            .iter()
            .filter(|formula| formula.row == row)
            .map(|formula| {
                let left = self.formula_x(formula);
                if local.x - left < formula.width / 2.0 {
                    ((local.x - left).abs(), formula.range.start)
                } else {
                    ((local.x - left - formula.width).abs(), formula.range.end)
                }
            });
        let index = text
            .chain(formulas)
            .min_by(|a, b| f32::from(a.0).total_cmp(&f32::from(b.0)))
            .map_or(self.line_starts[row], |(_, index)| index);
        if self.bounds.contains(&position) {
            Ok(index)
        } else {
            Err(index)
        }
    }

    pub(in crate::preview) fn paint(&self, window: &mut Window, cx: &mut App) {
        let baseline = (self.line_height - self.ascent - self.descent) / 2.0 + self.ascent;
        for formula in &self.formulas {
            let bounds = self.formula_bounds(formula);
            if !bounds.intersects(&window.content_mask().bounds) {
                continue;
            }
            let _ = crate::document_preview::image(
                window,
                bounds,
                bounds,
                gpui::Corners::default(),
                formula.image.clone(),
                0,
                false,
            );
        }
        for fragment in &self.fragments {
            let own_baseline = (self.line_height - fragment.line.ascent - fragment.line.descent)
                / 2.0
                + fragment.line.ascent;
            let origin = self.bounds.origin
                + point(
                    self.row_offsets[fragment.row] + fragment.x,
                    self.line_height * fragment.row as f32 + baseline - own_baseline,
                );
            crate::document_preview::shaped(
                &fragment.line,
                origin,
                self.line_height,
                TextAlign::Left,
                None,
                true,
                window,
            );
            let _ = fragment.line.paint_background(
                origin,
                self.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
            crate::document_preview::shaped(
                &fragment.line,
                origin,
                self.line_height,
                TextAlign::Left,
                None,
                false,
                window,
            );
            let _ =
                fragment
                    .line
                    .paint(origin, self.line_height, TextAlign::Left, None, window, cx);
        }
    }

    fn formula_x(&self, formula: &FormulaFragment) -> Pixels {
        if formula.centered {
            (self.bounds.size.width - formula.width).max(px(0.0)) / 2.0
        } else {
            self.row_offsets[formula.row] + formula.x
        }
    }

    fn formula_bounds(&self, formula: &FormulaFragment) -> Bounds<Pixels> {
        let baseline = (self.line_height - self.ascent - self.descent) / 2.0 + self.ascent;
        Bounds::new(
            self.bounds.origin
                + point(
                    self.formula_x(formula),
                    self.line_height * formula.row as f32 + baseline - formula.baseline,
                ),
            size(formula.width, formula.height),
        )
    }
}
