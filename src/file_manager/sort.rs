use std::{cmp::Ordering, sync::LazyLock};

use icu_collator::{
    Collator, CollatorBorrowed, CollatorPreferences,
    options::{CollatorOptions, Strength},
    preferences::CollationNumericOrdering,
};
use serde::{Deserialize, Serialize};

use super::{EntryKind, FileEntry};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    Modified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SortSpec {
    pub direction: SortDirection,
    pub directories_first: bool,
    #[serde(default)]
    pub key: SortKey,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            direction: SortDirection::Ascending,
            directories_first: true,
            key: SortKey::Name,
        }
    }
}

fn collator() -> &'static CollatorBorrowed<'static> {
    static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
        let mut preferences = sys_locale::get_locale()
            .and_then(|locale| {
                locale
                    .replace('_', "-")
                    .parse::<icu_locale_core::Locale>()
                    .ok()
            })
            .map(CollatorPreferences::from)
            .unwrap_or_default();
        preferences.numeric_ordering = Some(CollationNumericOrdering::True);
        let mut options = CollatorOptions::default();
        options.strength = Some(Strength::Secondary);
        Collator::try_new(preferences, options).expect("compiled ICU collation data")
    });
    &COLLATOR
}

pub(super) fn compare_entries(left: &FileEntry, right: &FileEntry, spec: SortSpec) -> Ordering {
    // Navigation and grouping do not reverse when the value sort reverses.
    let parent_order = (right.kind == EntryKind::Parent).cmp(&(left.kind == EntryKind::Parent));
    let group_order = if spec.directories_first {
        right.is_directory().cmp(&left.is_directory())
    } else {
        Ordering::Equal
    };
    let name_order = || {
        collator()
            .compare(&left.display_name, &right.display_name)
            .then_with(|| left.os_name.cmp(&right.os_name))
    };
    let value_order = match spec.key {
        SortKey::Name => name_order(),
        SortKey::Modified => left
            .metadata
            .modified
            .cmp(&right.metadata.modified)
            .then_with(name_order),
    };
    parent_order.then(group_order).then(match spec.direction {
        SortDirection::Ascending => value_order,
        SortDirection::Descending => value_order.reverse(),
    })
}

pub(super) fn sort_entries(entries: &mut [FileEntry], spec: SortSpec) {
    entries.sort_by(|left, right| compare_entries(left, right, spec));
}
