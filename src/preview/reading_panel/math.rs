//! Formula content is immutable; only visible consumers own rendering leases.
use crate::preview::{InlineKind, InlineSpan, ReadingPreviewPanel};
use crate::{
    math::{MathDemand, MathKey, MathService, MathStyle, RenderedFormula},
    org_syntax::math::{Formula, MathMode},
};
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Debug)]
struct InlineFormula {
    range: Range<usize>,
    formula: Formula,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct BindingKey {
    style: MathStyle,
    formulas: Range<usize>,
}

#[derive(Debug)]
struct RowContent {
    formulas: Vec<InlineFormula>,
    bindings: Mutex<HashMap<BindingKey, Weak<RowBinding>>>,
}

#[derive(Clone, Debug, Default)]
pub(in crate::preview) struct MathRow {
    content: Option<Arc<RowContent>>,
    range: Range<usize>,
}

#[derive(Debug)]
struct RowBinding {
    key: BindingKey,
    demands: Vec<MathDemand>,
    requested_at: Instant,
    painted: AtomicBool,
}
impl RowBinding {
    fn versions(&self) -> Vec<u64> {
        self.demands.iter().map(MathDemand::version).collect()
    }
}

pub(in crate::preview) struct FormulaObject {
    pub(in crate::preview) range: Range<usize>,
    pub(in crate::preview) display: bool,
    pub(in crate::preview) rendered: Arc<RenderedFormula>,
}

impl MathRow {
    pub(in crate::preview) fn trace_paint(&self, style: MathStyle, formula_count: usize) {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        if !*ENABLED.get_or_init(|| std::env::var_os("ORG_STUDIO_PROFILE_FRAMES").is_some()) {
            return;
        }
        let Some(content) = &self.content else {
            return;
        };
        let bindings = content.bindings.lock().unwrap();
        for binding in bindings
            .values()
            .filter_map(Weak::upgrade)
            .filter(|binding| binding.key.style == style)
        {
            if binding.painted.load(Ordering::Relaxed)
                || binding.demands.iter().any(MathDemand::is_pending)
            {
                continue;
            }
            let ready = binding
                .demands
                .iter()
                .filter(|demand| demand.ready_image().is_some())
                .count();
            if ready == formula_count && !binding.painted.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "org_math_first_paint formulas={} failed={} elapsed_ms={:.3}",
                    ready,
                    binding.demands.len() - ready,
                    binding.requested_at.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
    }

    pub(in crate::preview) fn new(text: &str, spans: &[InlineSpan]) -> Self {
        let mut formulas = spans
            .iter()
            .filter(|span| span.kind == InlineKind::Latex)
            .filter_map(|span| {
                Some(InlineFormula {
                    range: span.range.clone(),
                    formula: Formula::from_delimited(text.get(span.range.clone())?)?,
                })
            })
            .collect::<Vec<_>>();
        formulas.sort_by_key(|item| item.range.start);
        Self {
            content: (!formulas.is_empty()).then(|| {
                Arc::new(RowContent {
                    formulas,
                    bindings: Mutex::default(),
                })
            }),
            range: 0..text.len(),
        }
    }
    pub(in crate::preview) fn is_empty(&self) -> bool {
        self.formula_indices().is_empty()
    }
    fn formula_indices(&self) -> Range<usize> {
        let Some(content) = &self.content else {
            return 0..0;
        };
        let start = content
            .formulas
            .partition_point(|formula| formula.range.end <= self.range.start);
        let end = content
            .formulas
            .partition_point(|formula| formula.range.start < self.range.end);
        start..end.max(start)
    }
    fn crop_to_formulas(&self, indices: Range<usize>) -> Self {
        let Some(content) = &self.content else {
            return Self::default();
        };
        let start = indices.start.min(content.formulas.len());
        let end = indices.end.min(content.formulas.len());
        if start >= end {
            return Self::default();
        }
        Self {
            content: self.content.clone(),
            range: content.formulas[start].range.start..content.formulas[end - 1].range.end,
        }
    }
    pub(in crate::preview) fn slice(&self, range: Range<usize>) -> Self {
        Self {
            content: self.content.clone(),
            range: self.range.start + range.start..self.range.start + range.end,
        }
    }
    pub(in crate::preview) fn objects(&self, style: MathStyle) -> Vec<FormulaObject> {
        let Some(content) = &self.content else {
            return Vec::new();
        };
        let bindings = content
            .bindings
            .lock()
            .unwrap()
            .values()
            .filter_map(Weak::upgrade)
            .filter(|binding| binding.key.style == style)
            .collect::<Vec<_>>();
        let indices = self.formula_indices();
        content.formulas[indices.clone()]
            .iter()
            .enumerate()
            .filter_map(|(offset, item)| {
                if item.range.start < self.range.start || item.range.end > self.range.end {
                    return None;
                }
                let index = indices.start + offset;
                let rendered = bindings
                    .iter()
                    .find(|binding| binding.key.formulas.contains(&index))
                    .and_then(|binding| binding.demands.get(index - binding.key.formulas.start))
                    .and_then(MathDemand::rendered)
                    .or_else(|| {
                        crate::math::cached_formula(&MathKey {
                            formula: item.formula.clone(),
                            style,
                        })
                    })?;
                Some(FormulaObject {
                    range: item.range.start - self.range.start..item.range.end - self.range.start,
                    display: item.formula.mode == MathMode::Display,
                    rendered,
                })
            })
            .collect()
    }
    pub(in crate::preview) fn errors(&self) -> Vec<Arc<str>> {
        let Some(content) = &self.content else {
            return Vec::new();
        };
        let indices = self.formula_indices();
        let mut errors = content
            .bindings
            .lock()
            .unwrap()
            .values()
            .filter_map(Weak::upgrade)
            .flat_map(|binding| {
                let start = indices.start.max(binding.key.formulas.start);
                let end = indices.end.min(binding.key.formulas.end);
                (start..end)
                    .filter_map(|index| binding.demands[index - binding.key.formulas.start].error())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        errors.sort_unstable();
        errors.dedup();
        errors
    }
    fn bind(
        &self,
        style: MathStyle,
        previous: Option<&RowBinding>,
        cx: &mut gpui::App,
    ) -> Arc<RowBinding> {
        let content = self
            .content
            .as_ref()
            .expect("only formula rows request a binding");
        let mut bindings = content.bindings.lock().unwrap();
        bindings.retain(|_, binding| binding.strong_count() > 0);
        let key = BindingKey {
            style,
            formulas: self.formula_indices(),
        };
        if let Some(binding) = bindings.get(&key).and_then(Weak::upgrade) {
            return binding;
        }
        let requested_at = Instant::now();
        let demands = content.formulas[key.formulas.clone()]
            .iter()
            .enumerate()
            .map(|(index, formula)| {
                let demand = MathDemand::default();
                if let Some(previous) = previous
                    .filter(|previous| {
                        previous.key.style == style
                            && previous
                                .key
                                .formulas
                                .contains(&(index + key.formulas.start))
                    })
                    .and_then(|binding| {
                        binding
                            .demands
                            .get(index + key.formulas.start - binding.key.formulas.start)
                    })
                {
                    demand.inherit(previous);
                }
                demand.request(
                    MathKey {
                        formula: formula.formula.clone(),
                        style,
                    },
                    cx,
                );
                demand
            })
            .collect();
        let binding = Arc::new(RowBinding {
            key: key.clone(),
            demands,
            requested_at,
            painted: AtomicBool::new(false),
        });
        bindings.insert(key, Arc::downgrade(&binding));
        binding
    }
}

struct VisibleMathRow {
    owner: MathOwner,
    content: MathRow,
    binding: Arc<RowBinding>,
    versions: Vec<u64>,
    requested: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MathOwner {
    Org(crate::org_syntax::SyntaxId, u64),
    Markdown(crate::preview::projection::VisualRowId),
}

fn row_owner(document: &crate::preview::PreviewSnapshot, row: usize) -> Option<MathOwner> {
    let row = document.projection.rows.get(row)?;
    match document.format {
        crate::preview::DocumentFormat::Org => {
            let block = document.blocks.nodes().get(row.block_id as usize)?;
            Some(MathOwner::Org(
                block.syntax_id,
                row.source
                    .range
                    .start
                    .0
                    .saturating_sub(block.content.start.0),
            ))
        }
        crate::preview::DocumentFormat::Markdown => Some(MathOwner::Markdown(row.id)),
    }
}

#[derive(Default)]
pub(in crate::preview) struct MathViewport {
    rows: HashMap<(usize, usize), VisibleMathRow>,
    subscription: Option<gpui::Subscription>,
    invalidation_scheduled: bool,
}

impl MathViewport {
    pub(in crate::preview) fn retain(
        &mut self,
        document: &crate::preview::PreviewSnapshot,
        list: &gpui::ListState,
        visible: &[usize],
    ) {
        self.rows.retain(|&(row, _), previous| {
            let retain = previous.requested
                && row_owner(document, row) == Some(previous.owner)
                && visible.binary_search(&row).is_ok_and(|index| {
                    list.item_is_above_viewport(index) != Some(true)
                        && list.item_is_below_viewport(index) != Some(true)
                });
            // Unknown list geometry cannot retain old viewport leases indefinitely.
            // Prepaint renews every live consumer, including during list remeasurement.
            previous.requested = false;
            retain
        });
    }
}

impl ReadingPreviewPanel {
    /// Rebind previously visible formulas at publication, before the new row is measured.
    /// This keeps its old image through the very first frame of a source edit too.
    pub(super) fn refresh_math(
        &mut self,
        style: crate::preview::PreviewStyle,
        cx: &mut gpui::Context<Self>,
    ) {
        let rows = self
            .math
            .rows
            .iter()
            .filter(|&(&(row, _), old)| {
                old.requested && row_owner(self.document(), row) == Some(old.owner)
            })
            .map(|(&(row, slot), old)| (row, slot, old.binding.key.formulas.clone()))
            .collect::<Vec<_>>();
        let document = self.document().clone();
        let Some(map) = &document.display_map else {
            return;
        };
        for (row, slot, indices) in rows {
            let runs = map.runs(row);
            let (formulas, font_size) = if slot == 0 {
                (runs.math, map.layout(row, style).font_size * self.zoom())
            } else {
                let Some(table) = map.table_projection(row) else {
                    continue;
                };
                let Some(cell) = table.display_cell_runs(&runs.text).get(slot - 1) else {
                    continue;
                };
                (
                    cell.math.clone(),
                    style.typography.relative_size(1.0, 12.0) * self.zoom(),
                )
            };
            self.request_math(
                row,
                slot,
                formulas.crop_to_formulas(indices),
                MathStyle::new(font_size, style.palette.foreground),
                cx,
            );
        }
    }

    pub(crate) fn math_previews_ready(&self) -> bool {
        self.math.rows.values().all(|row| {
            row.binding
                .demands
                .iter()
                .all(|demand| !demand.is_pending())
        })
    }
    // Only an unclipped, prepainted text element can enqueue compiler work.
    pub(in crate::preview) fn request_math(
        &mut self,
        row: usize,
        slot: usize,
        formulas: MathRow,
        style: MathStyle,
        cx: &mut gpui::Context<Self>,
    ) {
        let _span = tracing::info_span!("math.request_row").entered();
        if formulas.is_empty() {
            self.math.rows.remove(&(row, slot));
            return;
        }
        if self.math.subscription.is_none() {
            let service = MathService::shared(cx);
            self.math.subscription =
                Some(cx.observe(&service, |this, _, cx| this.math_changed(cx)));
        }
        let Some(owner) = row_owner(self.document(), row) else {
            return;
        };
        let mut existing = self
            .math
            .rows
            .get_mut(&(row, slot))
            .filter(|previous| previous.owner == owner);
        if let Some(existing) = existing.as_mut() {
            existing.requested = true;
        }
        if existing.as_ref().is_some_and(|existing| {
            existing.binding.key.style == style
                && existing.binding.key.formulas == formulas.formula_indices()
                && existing
                    .content
                    .content
                    .as_ref()
                    .zip(formulas.content.as_ref())
                    .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
        }) {
            return;
        }
        let already_paintable = formulas.objects(style).len() == formulas.formula_indices().len();
        let binding = formulas.bind(style, existing.map(|row| row.binding.as_ref()), cx);
        // A cache hit may change geometry during prepaint; invalidate only after this frame.
        let versions = if already_paintable {
            binding.versions()
        } else {
            vec![u64::MAX; binding.demands.len()]
        };
        self.math.rows.insert(
            (row, slot),
            VisibleMathRow {
                owner,
                content: formulas,
                binding,
                versions,
                requested: true,
            },
        );
        if !already_paintable && !self.math.invalidation_scheduled {
            self.math.invalidation_scheduled = true;
            let panel = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = panel.update(cx, |panel, cx| {
                    panel.math.invalidation_scheduled = false;
                    panel.math_changed(cx);
                });
            });
        }
    }

    fn math_changed(&mut self, cx: &mut gpui::Context<Self>) {
        let _span = tracing::info_span!("math.invalidate").entered();
        let changed = self
            .math
            .rows
            .iter_mut()
            .filter_map(|(&(row, _), visible)| {
                let next = visible.binding.versions();
                if visible.versions == next {
                    return None;
                }
                visible.versions = next;
                Some(row)
            })
            .collect::<std::collections::HashSet<_>>();
        if changed.is_empty() {
            return;
        }
        self.remeasure_math_rows(&changed);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::inline_layout::{InlineTextMetrics, shape_inline};
    use gpui::AppContext;
    use gpui::{point, px, size};

    #[gpui::test]
    fn math_adjacent_wrapped_formulas_have_distinct_selection_positions(
        cx: &mut gpui::TestAppContext,
    ) {
        let parsed = crate::org_syntax::inline::parse(r"\(a\)\(b\)");
        let row = MathRow::new(&parsed.text, &parsed.spans);
        let style = MathStyle::new(16.0, 0x202020);
        let _binding = cx.update(|cx| row.bind(style, None, cx));
        cx.run_until_parked();
        let objects = row.objects(style);
        let width = objects
            .iter()
            .map(|object| object.rendered.width)
            .fold(0.0, f32::max)
            + 0.1;
        let window = cx.add_empty_window();
        let layout = window.update(|window, _| {
            shape_inline(
                &parsed.text,
                &[gpui::TextRun {
                    len: parsed.text.len(),
                    font: gpui::font("Menlo"),
                    ..Default::default()
                }],
                &parsed.spans,
                &row,
                InlineTextMetrics {
                    font_size: 16.0,
                    code_font_size: 16.0,
                    line_height: 24.0,
                    wrap_width: width,
                    foreground: 0x202020,
                },
                window.text_system(),
            )
        });
        assert_eq!(layout.line_starts.len(), 2);
        let position = layout.position_for_index(objects[1].range.start).unwrap();
        assert_eq!(position.y, layout.line_height);
        assert_eq!(position.x, px(0.0));
    }

    #[gpui::test]
    fn math_errors_are_limited_to_the_requested_slice(cx: &mut gpui::TestAppContext) {
        let parsed = crate::org_syntax::inline::parse(r"\(\unknownCommand{x}\) then \(y\)");
        let row = MathRow::new(&parsed.text, &parsed.spans);
        let _binding = cx.update(|cx| row.bind(MathStyle::new(16.0, 0x202020), None, cx));
        cx.run_until_parked();
        assert_eq!(row.errors().len(), 1);
        let range = parsed.spans.last().unwrap().range.clone();
        assert!(row.slice(range).errors().is_empty());
    }

    #[gpui::test]
    fn math_requests_follow_visible_wrapped_lines_and_allow_two_windows(
        cx: &mut gpui::TestAppContext,
    ) {
        let text = (500_000..501_000)
            .map(|index| format!("\\(v_{{{index}}}\\) "))
            .collect::<String>();
        let parsed = crate::org_syntax::inline::parse(&text);
        let row = MathRow::new(&parsed.text, &parsed.spans);
        let style = MathStyle::new(16.0, 0x202020);
        let window = cx.add_empty_window();
        let mut layout = window.update(|window, _| {
            shape_inline(
                &parsed.text,
                &[gpui::TextRun {
                    len: parsed.text.len(),
                    font: gpui::font("Menlo"),
                    ..Default::default()
                }],
                &parsed.spans,
                &row,
                InlineTextMetrics {
                    font_size: 16.0,
                    code_font_size: 16.0,
                    line_height: 24.0,
                    wrap_width: 160.0,
                    foreground: 0x202020,
                },
                window.text_system(),
            )
        });
        let clip = gpui::Bounds::new(point(px(0.0), px(0.0)), size(px(160.0), px(48.0)));
        layout.set_bounds(gpui::Bounds::new(clip.origin, layout.size));
        let first = row.slice(layout.visible_range(clip));
        let first = cx.update(|cx| first.bind(style, None, cx));
        assert!(
            first.demands.len() < 10,
            "a viewport must not request all 1000 formulas in one paragraph"
        );
        layout.set_bounds(gpui::Bounds::new(point(px(0.0), px(-5_000.0)), layout.size));
        let second = row.slice(layout.visible_range(clip));
        let second = cx.update(|cx| second.bind(style, None, cx));
        assert!(second.demands.len() < 10);
        assert!(second.key.formulas.start > first.key.formulas.end);
        cx.run_until_parked();
        assert!(
            first
                .demands
                .iter()
                .chain(&second.demands)
                .all(|demand| demand.ready_image().is_some())
        );
        assert_eq!(
            row.objects(style).len(),
            first.demands.len() + second.demands.len()
        );
    }

    #[gpui::test]
    fn math_source_publication_keeps_the_previous_image_before_first_measurement(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::document::{ByteRange, DocumentBuffer, EditTransaction, TextEdit, TextSnapshot};
        let source = r"Value \(p_{98217} + q\).";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let previous = Arc::new(crate::preview::loading::derive_preview(
            "math-edit.org".into(),
            before.clone(),
        ));
        let panel = cx.new(|_| ReadingPreviewPanel::new(previous.clone(), 80.0));
        let style = *crate::preview::preview_style(crate::preview::PreviewStyleId::Base);
        let key = MathStyle::new(
            previous
                .display_map
                .as_ref()
                .unwrap()
                .layout(0, style)
                .font_size,
            style.palette.foreground,
        );
        panel.update(cx, |panel, cx| {
            panel.request_math(
                0,
                0,
                previous.display_map.as_ref().unwrap().runs(0).math,
                key,
                cx,
            )
        });
        cx.run_until_parked();
        let old = previous
            .display_map
            .as_ref()
            .unwrap()
            .runs(0)
            .math
            .objects(key)[0]
            .rendered
            .clone();
        let offset = source.find('q').unwrap() as u64;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(offset, offset + 1), "r")],
            ))
            .unwrap();
        let next = Arc::new(crate::preview::loading::derive_preview_incremental(
            "math-edit.org".into(),
            buffer.snapshot(),
            Some(&previous),
            std::slice::from_ref(&delta),
        ));
        panel.update(cx, |panel, cx| {
            panel.map_viewport_through_delta(&delta);
            panel.replace_document_with_style(next.clone(), style, cx);
            let objects = next.display_map.as_ref().unwrap().runs(0).math.objects(key);
            assert_eq!(
                objects.len(),
                1,
                "source publication must bind the old image before layout or paint"
            );
            assert!(Arc::ptr_eq(&objects[0].rendered, &old));
        });
        cx.run_until_parked();
        assert!(!Arc::ptr_eq(
            &next.display_map.as_ref().unwrap().runs(0).math.objects(key)[0].rendered,
            &old
        ));

        let before = buffer.snapshot();
        let start = source.find(r"\(").unwrap() as u64;
        let end = (source.find(r"\)").unwrap() + 2) as u64;
        let delta = buffer
            .commit(EditTransaction::new(
                before.revision(),
                vec![TextEdit::new(ByteRange::new(start, end), "plain text")],
            ))
            .unwrap();
        let plain = Arc::new(crate::preview::loading::derive_preview_incremental(
            "math-edit.org".into(),
            buffer.snapshot(),
            Some(&next),
            std::slice::from_ref(&delta),
        ));
        panel.update(cx, |panel, cx| {
            panel.map_viewport_through_delta(&delta);
            panel.replace_document_with_style(plain, style, cx);
            assert!(
                panel.math.rows.is_empty(),
                "removing formulas must release their viewport leases at publication"
            );
            assert!(panel.math_previews_ready());
        });
    }

    #[gpui::test]
    fn math_viewport_releases_old_leases_when_list_geometry_is_unknown(
        cx: &mut gpui::TestAppContext,
    ) {
        let source = "Line \\(v_{98012}\\).\n".repeat(100);
        let snapshot = crate::document::DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let document = Arc::new(crate::preview::loading::derive_preview(
            "math-scroll.org".into(),
            snapshot,
        ));
        let panel = cx.new(|_| ReadingPreviewPanel::new(document, 80.0));
        panel.update(cx, |panel, cx| {
            for row in (0..100).rev() {
                panel.retain_visible_media();
                let formulas = panel
                    .document()
                    .display_map
                    .as_ref()
                    .unwrap()
                    .runs(row)
                    .math;
                panel.request_math(row, 0, formulas, MathStyle::new(16.0, 0x202020), cx);
                assert!(
                    panel.math.rows.len() <= 2,
                    "unknown offscreen rows cannot accumulate strong image leases"
                );
            }
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn math_layout_preserves_source_hit_testing_and_independent_window_styles(
        cx: &mut gpui::TestAppContext,
    ) {
        let parsed = crate::org_syntax::inline::parse(r"A \(\frac{a+b}{c+d}\) tail.");
        let row = MathRow::new(&parsed.text, &parsed.spans);
        let style = MathStyle::new(16.0, 0x202020);
        let large = MathStyle::new(24.0, 0xf0f0f0);
        let (binding, other) =
            cx.update(|cx| (row.bind(style, None, cx), row.bind(large, None, cx)));
        cx.run_until_parked();
        let objects = row.objects(style);
        let bigger = row.objects(large);
        assert_eq!(objects.len(), 1);
        assert!(bigger[0].rendered.width > objects[0].rendered.width * 1.4);
        assert!(!Arc::ptr_eq(&objects[0].rendered, &bigger[0].rendered));
        let window = cx.add_empty_window();
        let mut layout = window.update(|window, _| {
            shape_inline(
                &parsed.text,
                &[gpui::TextRun {
                    len: parsed.text.len(),
                    font: gpui::font("Menlo"),
                    ..Default::default()
                }],
                &parsed.spans,
                &row,
                InlineTextMetrics {
                    font_size: 16.0,
                    code_font_size: 16.0,
                    line_height: 24.0,
                    wrap_width: 300.0,
                    foreground: 0x202020,
                },
                window.text_system(),
            )
        });
        layout.set_bounds(gpui::Bounds::new(
            point(px(10.0), px(20.0)),
            size(px(300.0), layout.size.height),
        ));
        let range = &objects[0].range;
        assert_eq!(
            layout.visual_range(range.start + 2..range.start + 7),
            range.clone(),
            "searching part of the LaTeX source must visibly highlight its formula"
        );
        let left = layout.position_for_index(range.start).unwrap();
        let right = layout.position_for_index(range.end).unwrap();
        assert!((f32::from(right.x - left.x) - objects[0].rendered.width).abs() < 0.1);
        assert_eq!(
            layout.index_for_position(left + point(px(1.0), px(1.0))),
            Ok(range.start)
        );
        assert_eq!(
            layout.index_for_position(right - point(px(1.0), px(-1.0))),
            Ok(range.end)
        );
        drop(other);
        assert!(
            row.content
                .as_ref()
                .unwrap()
                .bindings
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.style == large)
                .all(|(_, binding)| binding.upgrade().is_none()),
            "offscreen consumers release their leases"
        );
        assert_eq!(
            row.objects(large).len(),
            1,
            "cached images paint without a new request"
        );
        assert_eq!(row.objects(style).len(), 1);
        let edited = crate::org_syntax::inline::parse(r"A \(\frac{c+d}{e+f}\) tail.");
        let edited = MathRow::new(&edited.text, &edited.spans);
        let edited_binding = cx.update(|cx| {
            let edited_binding = edited.bind(style, Some(&binding), cx);
            assert!(
                Arc::ptr_eq(&edited.objects(style)[0].rendered, &objects[0].rendered),
                "keep the previous image and geometry until replacement decoding finishes"
            );
            edited_binding
        });
        cx.run_until_parked();
        assert!(!Arc::ptr_eq(
            &edited.objects(style)[0].rendered,
            &objects[0].rendered
        ));
        drop(edited_binding);
        drop(binding);
        assert!(
            row.content
                .as_ref()
                .unwrap()
                .bindings
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.style == style)
                .all(|(_, binding)| binding.upgrade().is_none())
        );
        let reparsed = MathRow::new(&parsed.text, &parsed.spans);
        assert_eq!(
            reparsed.objects(style).len(),
            1,
            "unchanged formulas survive document reparsing without a source flash"
        );
    }
}
