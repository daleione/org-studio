//! Build plot data and source anchors without accessing UI state.
use super::PlotEntry;
use crate::{
    document::{RevisionDelta, SharedTextSnapshot},
    org_syntax::BlockArena,
    plot::{PlotCandidate, PlotId, PlotPreparation},
};
use std::{collections::HashMap, sync::Arc};

pub(super) struct IndexedPlot {
    pub(super) source: PlotCandidate,
    pub(super) raw: Arc<str>,
    pub(super) prepared: PlotPreparation,
}

pub(super) struct PlotIndex {
    pub(super) snapshot: SharedTextSnapshot,
    pub(super) arena: Arc<BlockArena>,
    pub(super) candidates: Vec<IndexedPlot>,
    pub(super) mapped_anchors: HashMap<u64, PlotId>,
}

pub(super) fn build(
    snapshot: SharedTextSnapshot,
    old: Arc<[Arc<PlotEntry>]>,
    previous_snapshot: Option<SharedTextSnapshot>,
    arena: Option<Arc<BlockArena>>,
    deltas: Vec<RevisionDelta>,
) -> PlotIndex {
    let mapped_anchors = map_anchors(
        &old,
        &deltas,
        previous_snapshot.as_deref(),
        snapshot.as_ref(),
    );
    if !crate::plot::may_have_plots(snapshot.as_ref()) {
        return PlotIndex {
            snapshot,
            arena: Arc::new(BlockArena::default()),
            candidates: Vec::new(),
            mapped_anchors,
        };
    }
    let arena = arena
        .as_ref()
        .and_then(|arena| {
            crate::org_syntax::parse_incremental(snapshot.as_ref(), arena, &deltas)
                .map(|(arena, _)| arena)
        })
        .unwrap_or_else(|| crate::org_syntax::parse(snapshot.as_ref()));
    let previous = old
        .iter()
        .map(|entry| (entry.raw.clone(), entry.prepared.clone()))
        .collect::<HashMap<_, _>>();
    let candidates = crate::plot::collect_plots(snapshot.as_ref(), &arena)
        .into_iter()
        .map(|source| {
            let raw: Arc<str> = format!(
                "{}\n{}",
                source.options.join("\n"),
                snapshot.copy_range(source.table)
            )
            .into();
            let prepared = previous
                .get(&raw)
                .cloned()
                .unwrap_or_else(|| crate::plot::prepare_plot(snapshot.as_ref(), &source));
            IndexedPlot {
                source,
                raw,
                prepared,
            }
        })
        .collect();
    PlotIndex {
        snapshot,
        arena: Arc::new(arena),
        candidates,
        mapped_anchors,
    }
}

fn map_anchors(
    old: &[Arc<PlotEntry>],
    deltas: &[RevisionDelta],
    previous: Option<&dyn crate::document::TextSnapshot>,
    next: &dyn crate::document::TextSnapshot,
) -> HashMap<u64, PlotId> {
    if old.is_empty() {
        return HashMap::new();
    }
    let Some(previous) = previous else {
        return HashMap::new();
    };
    if !deltas.is_empty() {
        return old
            .iter()
            .filter_map(|entry| {
                let mut range = crate::document::RevisionRange::new(
                    previous.revision(),
                    crate::document::ByteRange::new(
                        entry.source.declaration.start.0,
                        entry.source.declaration.start.0 + 2,
                    ),
                );
                for delta in deltas {
                    range = delta.map_range(range).ok()?;
                }
                (range.revision == next.revision()).then_some((range.range.start.0, entry.id))
            })
            .collect();
    }
    // Reloads and Reading snapshots can skip edit events. Map only unchanged
    // prefix/suffix anchors; never guess ownership inside replaced source.
    let before = previous.copy_range(crate::document::ByteRange::new(0, previous.len_bytes()));
    let after = next.copy_range(crate::document::ByteRange::new(0, next.len_bytes()));
    let prefix = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before.as_bytes()[prefix..]
        .iter()
        .rev()
        .zip(after.as_bytes()[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let end = before.len() - suffix;
    old.iter()
        .filter_map(|entry| {
            let start = entry.source.declaration.start.0 as usize;
            if start + 2 <= prefix {
                Some((start as u64, entry.id))
            } else if start >= end {
                Some((
                    (start as i128 + after.len() as i128 - before.len() as i128) as u64,
                    entry.id,
                ))
            } else {
                None
            }
        })
        .collect()
}
