//! One display recipe for editor rows, table cells, navigation and minimap shaping.

use std::{ops::Range, path::Path};

use crate::{
    document::{DocumentFormat, TextProjection},
    org_semantic::display::{InlineDisplayContext, OrgDisplayOptions, project_inline},
};

use super::syntax::{EditorLineStyle, EditorSemanticSpan, EditorStyleId};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DisplayOverrides {
    pub(super) descriptive_links: Option<bool>,
    pub(super) hide_leading_stars: Option<bool>,
    pub(super) hide_emphasis_markers: Option<bool>,
    pub(super) pretty_entities: Option<bool>,
}

impl DisplayOverrides {
    pub(super) fn apply(self, mut options: OrgDisplayOptions) -> OrgDisplayOptions {
        options.descriptive_links = self.descriptive_links.unwrap_or(options.descriptive_links);
        options.hide_leading_stars = self
            .hide_leading_stars
            .unwrap_or(options.hide_leading_stars);
        options.hide_emphasis_markers = self
            .hide_emphasis_markers
            .unwrap_or(options.hide_emphasis_markers);
        options.pretty_entities = self.pretty_entities.or(options.pretty_entities);
        options
    }
}

impl super::SemanticEditor {
    pub(super) fn source_reveals(&self) -> impl Iterator<Item = crate::document::ByteRange> {
        self.marked
            .as_ref()
            .map(|range| range.bytes)
            .into_iter()
            .chain(self.search_current)
    }
    pub(crate) fn toggle_org_display(
        &mut self,
        feature: crate::command::OrgDisplayFeature,
        cx: &mut gpui::Context<Self>,
    ) {
        use crate::command::OrgDisplayFeature;
        if DocumentFormat::detect(self.session.read(cx).syntax_path()) != Some(DocumentFormat::Org)
        {
            return;
        }
        self.finish_composition(cx);
        self.cancel_table_resize(cx);
        self.dismiss_table_actions(cx);
        let options = self.display_options(self.session.read(cx).syntax_path(), &self.snapshot(cx));
        match feature {
            OrgDisplayFeature::DescriptiveLinks => {
                self.display_overrides.descriptive_links = Some(!options.descriptive_links)
            }
            OrgDisplayFeature::LeadingStars => {
                self.display_overrides.hide_leading_stars = Some(!options.hide_leading_stars)
            }
            OrgDisplayFeature::EmphasisMarkers => {
                self.display_overrides.hide_emphasis_markers = Some(!options.hide_emphasis_markers)
            }
            OrgDisplayFeature::Entities => {
                self.display_overrides.pretty_entities =
                    Some(!options.pretty_entities.unwrap_or(false))
            }
        }
        self.dismiss_inline(cx);
        self.dismiss_todo(cx);
        self.dismiss_timestamp(cx);
        self.dismiss_link_editor(cx);
        self.sync_display_layout(&self.snapshot(cx), cx);
        cx.notify();
    }

    pub(super) fn sync_display_layout(
        &mut self,
        snapshot: &crate::document::DocumentSnapshot,
        cx: &gpui::App,
    ) {
        let options = self.display_options(self.session.read(cx).syntax_path(), snapshot);
        if self
            .applied_display_options
            .replace(options)
            .is_none_or(|previous| previous == options)
        {
            return;
        }
        let anchor = self.display_map.line_at_y(self.scroll_y);
        let fraction = ((self.scroll_y - self.display_map.line_start_y(anchor))
            / self.display_map.line_height_px(anchor).max(1.0))
        .clamp(0.0, 1.0);
        self.display_map.invalidate_layout();
        self.scroll_y = self.display_map.line_start_y(anchor)
            + fraction * self.display_map.line_height_px(anchor);
        self.shape_cache.clear();
        self.table_geometry_cache.borrow_mut().metrics.clear();
        self.hit_rows = std::sync::Arc::from([]);
        self.vertical_goal_x = None;
        self.minimap.invalidate_raster();
        self.layout_reflow_pending = true;
    }

    pub(super) fn display_options(
        &self,
        path: &Path,
        snapshot: &crate::document::DocumentSnapshot,
    ) -> OrgDisplayOptions {
        self.display_overrides
            .apply(self.syntax_service.display_options(path, snapshot))
    }

    pub(super) fn projected_line_at(
        &self,
        snapshot: &crate::document::DocumentSnapshot,
        offset: crate::document::ByteOffset,
        cx: &gpui::App,
    ) -> Option<(crate::document::ByteRange, TextProjection)> {
        let line = snapshot.line_index_at(offset).ok()?;
        let path = self.session.read(cx).syntax_path();
        let styles = super::syntax::SparseEditorStyleSnapshot::query_lines(
            path,
            snapshot,
            &[line.0],
            &self.syntax_service,
        );
        let style = styles.snapshot.line(line.0)?;
        let source = self.display_map.source_line(snapshot, line, Some(offset))?;
        let mut display = source.display;
        let options = self.display_options(path, snapshot);
        let reveal = local_reveals(&display, source.visible_range, self.source_reveals());
        let projection = if style.id == EditorStyleId::Table
            && DocumentFormat::detect(path) == Some(DocumentFormat::Org)
        {
            crate::org_semantic::display::project_table(
                &display.text,
                options,
                Some(display.source_to_display((offset.0 - source.visible_range.start.0) as usize)),
                &reveal,
            )
        } else {
            project_line(path, &display.text, style, options, &reveal)
        };
        display = display.then(projection);
        Some((source.visible_range, display))
    }

    pub(super) fn visible_horizontal_target(
        &self,
        snapshot: &crate::document::DocumentSnapshot,
        offset: crate::document::ByteOffset,
        forward: bool,
        by_word: bool,
        cx: &gpui::App,
    ) -> Option<crate::document::ByteOffset> {
        use crate::document::{Bias, ByteOffset};
        use unicode_segmentation::UnicodeSegmentation;
        let (range, display) = self.projected_line_at(snapshot, offset, cx)?;
        if !display.has_replacements() {
            return None;
        }
        let local = display.source_to_display((offset.0 - range.start.0) as usize);
        let target = if by_word && forward {
            display
                .text
                .split_word_bound_indices()
                .filter(|(_, segment)| super::word_segment(segment))
                .map(|(start, segment)| start + segment.len())
                .find(|boundary| *boundary > local)
                .or((local < display.text.len()).then_some(display.text.len()))
        } else if by_word {
            display
                .text
                .split_word_bound_indices()
                .filter(|(_, segment)| super::word_segment(segment))
                .map(|(start, _)| start)
                .take_while(|boundary| *boundary < local)
                .last()
                .or((local > 0).then_some(0))
        } else if forward {
            display
                .text
                .grapheme_indices(true)
                .map(|(start, _)| start)
                .chain([display.text.len()])
                .find(|boundary| *boundary > local)
        } else {
            display
                .text
                .grapheme_indices(true)
                .map(|(start, _)| start)
                .take_while(|boundary| *boundary < local)
                .last()
        };
        let bias = if forward { Bias::Right } else { Bias::Left };
        let target = ByteOffset(
            range.start.0
                + display.display_to_source_with_bias(target.unwrap_or(local), bias) as u64,
        );
        (target != offset).then_some(target)
    }
}

pub(super) fn local_reveals(
    display: &TextProjection,
    source: crate::document::ByteRange,
    ranges: impl IntoIterator<Item = crate::document::ByteRange>,
) -> Vec<Range<usize>> {
    ranges
        .into_iter()
        .filter_map(|range| {
            let start = range.start.0.max(source.start.0);
            let end = range.end.0.min(source.end.0);
            (start < end).then(|| {
                display.display_range(
                    (start - source.start.0) as usize..(end - source.start.0) as usize,
                )
            })
        })
        .collect()
}

pub(super) fn inline_start(text: &str, style: &EditorLineStyle) -> usize {
    if matches!(style.id, EditorStyleId::Heading(_)) {
        text.bytes().take_while(|byte| *byte == b'*').count()
    } else {
        0
    }
}

pub(super) fn project_line(
    path: &Path,
    text: &str,
    style: &EditorLineStyle,
    mut options: OrgDisplayOptions,
    reveal: &[Range<usize>],
) -> TextProjection {
    if DocumentFormat::detect(path) != Some(DocumentFormat::Org)
        || crate::syntax_highlighting::language_for_path(path).is_some()
        || matches!(
            style.id,
            EditorStyleId::Code | EditorStyleId::CodeBoundary | EditorStyleId::Table
        )
        || style.code_language.is_some()
    {
        return TextProjection::new(text);
    }
    if style.id == EditorStyleId::Comment {
        options.hide_emphasis_markers = false;
    }
    project_inline(
        text,
        options,
        InlineDisplayContext {
            start: inline_start(text, style),
            reveal,
            ..Default::default()
        },
    )
}

pub(super) fn project_spans(spans: &mut Vec<EditorSemanticSpan>, projection: &TextProjection) {
    for span in spans.iter_mut() {
        span.bytes = projection.display_range(span.bytes.clone());
    }
    spans.retain(|span| !span.bytes.is_empty());
}

pub(super) fn mask_heading_stars(
    spans: &mut Vec<EditorSemanticSpan>,
    text: &str,
    style: &EditorLineStyle,
    options: OrgDisplayOptions,
) {
    if options.hide_leading_stars && matches!(style.id, EditorStyleId::Heading(_)) {
        let count = text.bytes().take_while(|byte| *byte == b'*').count();
        if count > 1 {
            spans.push(EditorSemanticSpan {
                bytes: 0..count - 1,
                color: Some(super::syntax::EditorColorToken::Hidden),
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests;
