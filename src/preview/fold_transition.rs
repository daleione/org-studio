use std::{collections::HashSet, ops::Range, sync::Arc, time::Instant};

use crate::org_syntax::BlockId;
use gpui::{ListState, Window};

use super::{PreviewSnapshot, PreviewStyle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FoldDirection {
    Collapse,
    Expand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FoldListEdit {
    pub(crate) range: Range<usize>,
    pub(crate) new_count: usize,
}

impl FoldListEdit {
    pub(crate) fn replace(range: Range<usize>, new_count: usize) -> Self {
        Self { range, new_count }
    }

    pub(crate) fn insert(at: usize, new_count: usize) -> Self {
        Self::replace(at..at, new_count)
    }
}

#[derive(Clone)]
pub(crate) struct FoldSegment {
    /// Start of the represented range in the final semantic projection.
    pub(crate) target_start: usize,
    /// Position of this segment in the temporary physical list.
    pub(crate) transition_index: usize,
    /// Number of final rows represented by the temporary segment.
    pub(crate) target_len: usize,
    /// Viewport-bounded rows painted inside the segment.
    pub(crate) rendered_rows: Arc<[usize]>,
    pub(crate) distance: f32,
}

#[derive(Clone)]
pub(crate) struct FoldTransition {
    pub(crate) revision: u64,
    pub(crate) suppressed_markers: Arc<HashSet<BlockId>>,
    pub(crate) direction: FoldDirection,
    pub(crate) segments: Arc<[FoldSegment]>,
    pub(crate) initial_edits: Arc<[FoldListEdit]>,
    pub(crate) started_at: Option<Instant>,
    pub(crate) progress: f32,
    target_item_count: usize,
}

#[derive(Clone, Copy)]
pub(crate) enum FoldMeasurement<'a> {
    Rendered(&'a Window),
    Estimated,
}

pub(crate) struct FoldTransitionInput<'a> {
    pub(crate) current_rows: &'a [usize],
    pub(crate) target_rows: &'a [usize],
    pub(crate) document: &'a PreviewSnapshot,
    pub(crate) list_state: &'a ListState,
    pub(crate) viewport_height: f32,
    pub(crate) available_width: f32,
    pub(crate) zoom: f32,
    pub(crate) style: PreviewStyle,
    pub(crate) measurement: FoldMeasurement<'a>,
}

pub(crate) struct FoldTransitionPlan {
    direction: FoldDirection,
    segments: Vec<FoldSegment>,
    initial_edits: Vec<FoldListEdit>,
    target_item_count: usize,
}

impl FoldTransition {
    pub(crate) fn new(
        revision: u64,
        suppressed_markers: HashSet<BlockId>,
        direction: FoldDirection,
        segments: Vec<FoldSegment>,
        initial_edits: Vec<FoldListEdit>,
        target_item_count: usize,
    ) -> Self {
        debug_assert!(!segments.is_empty());
        debug_assert!(segments.windows(2).all(|pair| {
            pair[0].target_start + pair[0].target_len <= pair[1].target_start
                && pair[0].transition_index < pair[1].transition_index
        }));
        debug_assert!(segments.iter().all(|segment| {
            segment.target_start + segment.target_len <= target_item_count
                && !segment.rendered_rows.is_empty()
                && segment.distance > f32::EPSILON
        }));
        Self {
            revision,
            suppressed_markers: Arc::new(suppressed_markers),
            direction,
            segments: segments.into(),
            initial_edits: initial_edits.into(),
            started_at: None,
            progress: 0.0,
            target_item_count,
        }
    }

    pub(crate) fn segment_at(&self, transition_index: usize) -> Option<&FoldSegment> {
        self.segments
            .binary_search_by_key(&transition_index, |segment| segment.transition_index)
            .ok()
            .map(|index| &self.segments[index])
    }

    pub(crate) fn target_index_for_item(&self, transition_index: usize) -> usize {
        let mut physical_cursor = 0usize;
        let mut target_cursor = 0usize;
        for segment in self.segments.iter() {
            if transition_index < segment.transition_index {
                return target_cursor + transition_index - physical_cursor;
            }
            if transition_index == segment.transition_index {
                return segment.target_start;
            }
            physical_cursor = segment.transition_index + 1;
            target_cursor = segment.target_start + segment.target_len;
        }
        target_cursor + transition_index - physical_cursor
    }

    pub(crate) fn transition_item_count(&self) -> usize {
        self.segments
            .iter()
            .fold(self.target_item_count, |count, segment| {
                count - segment.target_len + 1
            })
    }

    pub(crate) fn completion_edits(&self) -> impl Iterator<Item = FoldListEdit> + '_ {
        self.segments.iter().rev().map(|segment| {
            FoldListEdit::replace(
                segment.transition_index..segment.transition_index + 1,
                segment.target_len,
            )
        })
    }
}

impl FoldTransitionPlan {
    pub(crate) fn build(input: FoldTransitionInput<'_>) -> Option<Self> {
        let disappearing = difference(input.current_rows, input.target_rows);
        let appearing = difference(input.target_rows, input.current_rows);
        match (disappearing.is_empty(), appearing.is_empty()) {
            (false, true) => Self::collapse(&disappearing, input),
            (true, false) => Self::expand(&appearing, input),
            _ => None,
        }
    }

    pub(crate) fn into_transition(
        self,
        revision: u64,
        suppressed_markers: HashSet<BlockId>,
    ) -> FoldTransition {
        FoldTransition::new(
            revision,
            suppressed_markers,
            self.direction,
            self.segments,
            self.initial_edits,
            self.target_item_count,
        )
    }

    fn collapse(disappearing: &[usize], input: FoldTransitionInput<'_>) -> Option<Self> {
        let source_segments = contiguous_segments(disappearing, input.current_rows)?;
        let mut transition_segments = Vec::new();
        let mut initial_edits = Vec::with_capacity(source_segments.len());
        let mut disappeared_before = 0usize;
        for (current_start, rows) in source_segments {
            let target_start = current_start.checked_sub(disappeared_before)?;
            let boundary = item_boundary(input.list_state, current_start);
            let is_near_viewport = match (boundary, input.measurement) {
                (Some(top), _) => top < input.viewport_height,
                (None, FoldMeasurement::Estimated) => true,
                (None, FoldMeasurement::Rendered(_)) => false,
            };
            let mut replacement_count = 0usize;
            if is_near_viewport {
                let (rendered_rows, distance) = collapse_geometry(
                    &rows,
                    current_start,
                    input.document,
                    input.list_state,
                    input.viewport_height,
                );
                if distance > f32::EPSILON {
                    let transition_index = target_start + transition_segments.len();
                    transition_segments.push(FoldSegment {
                        target_start,
                        transition_index,
                        target_len: 0,
                        rendered_rows: rendered_rows.into(),
                        distance,
                    });
                    replacement_count = 1;
                }
            }
            initial_edits.push(FoldListEdit::replace(
                current_start..current_start + rows.len(),
                replacement_count,
            ));
            disappeared_before += rows.len();
        }
        initial_edits.reverse();
        (!transition_segments.is_empty()).then_some(Self {
            direction: FoldDirection::Collapse,
            segments: transition_segments,
            initial_edits,
            target_item_count: input.target_rows.len(),
        })
    }

    fn expand(appearing: &[usize], input: FoldTransitionInput<'_>) -> Option<Self> {
        let segments = contiguous_segments(appearing, input.target_rows)?;
        let mut transition_segments = Vec::new();
        let mut initial_edits = Vec::new();
        let mut appeared_before = 0usize;
        let mut compression = 0usize;
        for (target_start, rows) in segments {
            let target_len = rows.len();
            let old_index = target_start.checked_sub(appeared_before)?;
            let boundary = item_boundary(input.list_state, old_index);
            let transition_index = target_start.checked_sub(compression)?;
            let is_near_viewport = match (boundary, input.measurement) {
                (Some(top), _) => top < input.viewport_height,
                (None, FoldMeasurement::Estimated) => true,
                (None, FoldMeasurement::Rendered(_)) => false,
            };
            if is_near_viewport {
                let (rendered_rows, distance) = expansion_geometry(&rows, boundary, &input);
                if distance > f32::EPSILON {
                    transition_segments.push(FoldSegment {
                        target_start,
                        transition_index,
                        target_len,
                        rendered_rows: rendered_rows.into(),
                        distance,
                    });
                    initial_edits.push(FoldListEdit::insert(transition_index, 1));
                    compression += target_len.saturating_sub(1);
                } else {
                    initial_edits.push(FoldListEdit::insert(transition_index, target_len));
                }
            } else {
                initial_edits.push(FoldListEdit::insert(transition_index, target_len));
            }
            appeared_before += target_len;
        }
        (!transition_segments.is_empty()).then_some(Self {
            direction: FoldDirection::Expand,
            segments: transition_segments,
            initial_edits,
            target_item_count: input.target_rows.len(),
        })
    }
}

fn difference(left: &[usize], right: &[usize]) -> Vec<usize> {
    let mut right_cursor = 0usize;
    left.iter()
        .copied()
        .filter(|value| {
            while right.get(right_cursor).is_some_and(|right| right < value) {
                right_cursor += 1;
            }
            right.get(right_cursor) != Some(value)
        })
        .collect()
}

fn contiguous_segments(changed: &[usize], rows: &[usize]) -> Option<Vec<(usize, Vec<usize>)>> {
    let mut segments: Vec<(usize, Vec<usize>)> = Vec::new();
    for row in changed.iter().copied() {
        let target_index = rows.binary_search(&row).ok()?;
        if let Some((start, rows)) = segments.last_mut()
            && *start + rows.len() == target_index
        {
            rows.push(row);
        } else {
            segments.push((target_index, vec![row]));
        }
    }
    Some(segments)
}

fn item_boundary(list_state: &ListState, index: usize) -> Option<f32> {
    list_state
        .bounds_for_item(index)
        .map(|bounds| f32::from(bounds.top()))
        .or_else(|| {
            index.checked_sub(1).and_then(|position| {
                list_state
                    .bounds_for_item(position)
                    .map(|bounds| f32::from(bounds.bottom()))
            })
        })
}

fn collapse_geometry(
    changed: &[usize],
    first_position: usize,
    _document: &PreviewSnapshot,
    list_state: &ListState,
    viewport_height: f32,
) -> (Vec<usize>, f32) {
    let boundary = list_state
        .bounds_for_item(first_position)
        .map(|bounds| bounds.top());
    let exact_distance = boundary.and_then(|boundary| {
        list_state
            .bounds_for_item(first_position + changed.len())
            .map(|bounds| f32::from(bounds.top() - boundary))
    });
    let viewport_distance = boundary
        .map(|boundary| viewport_height - f32::from(boundary))
        .unwrap_or(viewport_height)
        .max(24.0);
    let distance_limit = exact_distance
        .unwrap_or(viewport_distance)
        .min(viewport_distance);

    let mut cursor = 0.0_f32;
    let mut rendered_rows = Vec::new();
    for (offset, row) in changed.iter().copied().enumerate() {
        let position = first_position + offset;
        let measured = boundary.and_then(|boundary| {
            list_state.bounds_for_item(position).map(|bounds| {
                (
                    f32::from(bounds.top() - boundary),
                    f32::from(bounds.size.height),
                )
            })
        });
        let (top, height) = measured.unwrap_or((cursor, 24.0));
        cursor = cursor.max(top + height);
        if top < distance_limit {
            rendered_rows.push(row);
        }
        if cursor >= distance_limit {
            break;
        }
    }
    (
        rendered_rows,
        exact_distance.unwrap_or(cursor).min(viewport_distance),
    )
}

fn expansion_geometry(
    changed: &[usize],
    boundary: Option<f32>,
    input: &FoldTransitionInput<'_>,
) -> (Vec<usize>, f32) {
    let distance_limit = boundary
        .map(|boundary| input.viewport_height - boundary)
        .unwrap_or(input.viewport_height)
        .max(24.0);
    let mut cursor = 0.0_f32;
    let mut rendered_rows = Vec::new();
    for row in changed.iter().copied() {
        let height = match input.measurement {
            FoldMeasurement::Rendered(window) => input
                .document
                .display_map
                .as_ref()
                .map(|map| {
                    map.display_lines(
                        row,
                        input.available_width,
                        input.zoom,
                        input.style,
                        window.text_system(),
                    )
                    .parent_height
                    .max(24.0)
                })
                .unwrap_or(24.0),
            FoldMeasurement::Estimated => {
                estimated_row_height(input.document, row, input.zoom, input.style)
            }
        };
        if cursor < distance_limit {
            rendered_rows.push(row);
        }
        cursor += height;
        if cursor >= distance_limit {
            break;
        }
    }
    (rendered_rows, cursor.min(distance_limit))
}

fn estimated_row_height(
    document: &PreviewSnapshot,
    row: usize,
    zoom: f32,
    style: PreviewStyle,
) -> f32 {
    document
        .display_map
        .as_ref()
        .map(|map| {
            let layout = map.layout(row, style).scaled(zoom);
            layout.fixed_height.unwrap_or(layout.min_height).max(24.0)
        })
        .unwrap_or(24.0)
}

#[cfg(test)]
mod tests {
    use super::{
        FoldDirection, FoldListEdit, FoldSegment, FoldTransition, contiguous_segments, difference,
    };
    use std::{collections::HashSet, sync::Arc};

    fn segment(target_start: usize, transition_index: usize, target_len: usize) -> FoldSegment {
        FoldSegment {
            target_start,
            transition_index,
            target_len,
            rendered_rows: Arc::from([target_start]),
            distance: 24.0,
        }
    }

    #[test]
    fn segmented_transition_owns_every_projection_mapping() {
        let transition = FoldTransition::new(
            1,
            HashSet::new(),
            FoldDirection::Expand,
            vec![segment(1, 1, 2), segment(4, 3, 2)],
            vec![FoldListEdit::insert(1, 1), FoldListEdit::insert(3, 1)],
            8,
        );

        assert_eq!(transition.transition_item_count(), 6);
        assert_eq!(transition.target_index_for_item(0), 0);
        assert_eq!(transition.target_index_for_item(2), 3);
        assert_eq!(transition.target_index_for_item(4), 6);
        assert_eq!(
            transition.completion_edits().collect::<Vec<_>>(),
            vec![
                FoldListEdit::replace(3..4, 2),
                FoldListEdit::replace(1..2, 2),
            ]
        );
    }

    #[test]
    fn projection_difference_and_segments_preserve_document_order() {
        assert_eq!(difference(&[1, 2, 3, 5, 8], &[1, 5]), vec![2, 3, 8]);
        assert_eq!(difference(&[1, 5], &[1, 2, 3, 5, 8]), Vec::<usize>::new());
        assert_eq!(
            contiguous_segments(&[2, 3, 8], &[1, 2, 3, 5, 8]),
            Some(vec![(1, vec![2, 3]), (4, vec![8])])
        );
    }
}
