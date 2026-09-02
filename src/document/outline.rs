use std::{collections::HashSet, hash::Hash};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum GlobalVisibility {
    #[default]
    All,
    Overview,
    Contents,
}

impl GlobalVisibility {
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::All => Self::Overview,
            Self::Overview => Self::Contents,
            Self::Contents => Self::All,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalVisibility {
    Empty,
    Folded,
    Children,
    Subtree,
}

pub(crate) const fn next_local_visibility(
    current: Option<LocalVisibility>,
    has_children: bool,
) -> LocalVisibility {
    match current {
        Some(LocalVisibility::Folded) if has_children => LocalVisibility::Children,
        Some(LocalVisibility::Folded | LocalVisibility::Children) => LocalVisibility::Subtree,
        None | Some(LocalVisibility::Empty | LocalVisibility::Subtree) => LocalVisibility::Folded,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutlineHeading<Id> {
    pub(crate) position: usize,
    pub(crate) id: Id,
    pub(crate) level: u16,
}

pub(crate) struct OutlineCycleProjection<Id> {
    pub(crate) visibility: LocalVisibility,
    pub(crate) visible_positions: Vec<usize>,
    pub(crate) fold_markers: HashSet<Id>,
}

pub(crate) fn global_outline_visibility<Id>(
    item_count: usize,
    headings: &[OutlineHeading<Id>],
    visibility: GlobalVisibility,
) -> (Vec<usize>, HashSet<Id>)
where
    Id: Copy + Eq + Hash,
{
    if visibility == GlobalVisibility::All {
        return ((0..item_count).collect(), HashSet::new());
    }

    let top_level = headings.iter().map(|heading| heading.level).min();
    let visible = headings
        .iter()
        .filter(|heading| {
            visibility == GlobalVisibility::Contents || Some(heading.level) == top_level
        })
        .map(|heading| heading.position)
        .collect();
    let markers = headings
        .iter()
        .enumerate()
        .filter(|(_, heading)| {
            visibility == GlobalVisibility::Contents || Some(heading.level) == top_level
        })
        .filter_map(|(index, heading)| {
            let subtree_end = outline_subtree_end(item_count, headings, index);
            let has_hidden_items = subtree_end > heading.position + 1;
            let shows_child = visibility == GlobalVisibility::Contents
                && headings
                    .get(index + 1)
                    .is_some_and(|next| next.level > heading.level);
            (has_hidden_items && !shows_child).then_some(heading.id)
        })
        .collect();
    (visible, markers)
}

pub(crate) fn cycle_outline_visibility<Id>(
    item_count: usize,
    headings: &[OutlineHeading<Id>],
    current_visible: &[usize],
    current_markers: &HashSet<Id>,
    id: Id,
    continue_from_children: bool,
) -> Option<OutlineCycleProjection<Id>>
where
    Id: Copy + Eq + Hash,
{
    let heading_index = headings.iter().position(|heading| heading.id == id)?;
    let heading = headings[heading_index];
    let subtree_end = outline_subtree_end(item_count, headings, heading_index);
    if subtree_end == heading.position + 1 {
        return Some(OutlineCycleProjection {
            visibility: LocalVisibility::Empty,
            visible_positions: current_visible.to_vec(),
            fold_markers: current_markers.clone(),
        });
    }

    let direct_children = direct_child_headings(headings, heading_index, subtree_end);
    let subtree_is_folded = current_visible
        .iter()
        .filter(|position| **position >= heading.position && **position < subtree_end)
        .copied()
        .eq(std::iter::once(heading.position));
    let current = if subtree_is_folded {
        Some(LocalVisibility::Folded)
    } else if continue_from_children {
        Some(LocalVisibility::Children)
    } else {
        Some(LocalVisibility::Subtree)
    };
    let visibility = next_local_visibility(current, !direct_children.is_empty());

    let replacement = match visibility {
        LocalVisibility::Folded => vec![heading.position],
        LocalVisibility::Children => {
            let entry_end = headings
                .get(heading_index + 1)
                .map_or(subtree_end, |next| next.position);
            (heading.position..entry_end)
                .chain(direct_children.iter().map(|(_, child)| child.position))
                .collect()
        }
        LocalVisibility::Subtree => (heading.position..subtree_end).collect(),
        LocalVisibility::Empty => unreachable!(),
    };
    let mut visible_positions = Vec::with_capacity(
        current_visible.len() - current_visible.partition_point(|position| *position < subtree_end)
            + current_visible.partition_point(|position| *position < heading.position)
            + replacement.len(),
    );
    visible_positions.extend(
        current_visible
            .iter()
            .copied()
            .take_while(|position| *position < heading.position),
    );
    visible_positions.extend(replacement);
    visible_positions.extend(
        current_visible
            .iter()
            .copied()
            .skip_while(|position| *position < subtree_end),
    );

    let mut fold_markers = current_markers.clone();
    for nested in headings
        .iter()
        .skip(heading_index)
        .take_while(|nested| nested.position < subtree_end)
    {
        fold_markers.remove(&nested.id);
    }
    match visibility {
        LocalVisibility::Folded => {
            fold_markers.insert(id);
        }
        LocalVisibility::Children => {
            for (child_index, child) in direct_children {
                if outline_subtree_end(item_count, headings, child_index) > child.position + 1 {
                    fold_markers.insert(child.id);
                }
            }
        }
        LocalVisibility::Subtree | LocalVisibility::Empty => {}
    }

    Some(OutlineCycleProjection {
        visibility,
        visible_positions,
        fold_markers,
    })
}

pub(crate) fn outline_subtree_end<Id>(
    item_count: usize,
    headings: &[OutlineHeading<Id>],
    index: usize,
) -> usize
where
    Id: Copy,
{
    let heading = headings[index];
    headings
        .iter()
        .skip(index + 1)
        .find(|next| next.level <= heading.level)
        .map_or(item_count, |next| next.position)
}

pub(crate) fn direct_child_headings<Id>(
    headings: &[OutlineHeading<Id>],
    index: usize,
    subtree_end: usize,
) -> Vec<(usize, OutlineHeading<Id>)>
where
    Id: Copy + Eq,
{
    let parent = headings[index];
    let mut stack = vec![parent];
    let mut children = Vec::new();
    for (heading_index, heading) in headings
        .iter()
        .copied()
        .enumerate()
        .skip(index + 1)
        .take_while(|(_, heading)| heading.position < subtree_end)
    {
        while stack
            .last()
            .is_some_and(|ancestor| ancestor.level >= heading.level)
        {
            stack.pop();
        }
        if stack
            .last()
            .is_some_and(|ancestor| ancestor.id == parent.id)
        {
            children.push((heading_index, heading));
        }
        stack.push(heading);
    }
    children
}
