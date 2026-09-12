//! Snapshot-based literal search. Every scan starts at the scope boundary, including backwards
//! navigation, so overlapping candidates never change the canonical non-overlapping set.
use crate::document::{
    ByteOffset, ByteRange, DocumentSnapshot, EditTransaction, TextEdit, TextSnapshot,
};
use regex_syntax::hir::{ClassUnicode, ClassUnicodeRange};
use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicBool, Ordering},
};

pub const MATCH_LIMIT: usize = 100_000;
const PLAN_LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CaseMode {
    #[default]
    Auto,
    Sensitive,
    Insensitive,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Query {
    pub pattern: String,
    pub case: CaseMode,
    pub scope: ByteRange,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Completion {
    Complete,
    Partial,
    Truncated,
    Cancelled,
}
#[derive(Clone, Debug)]
pub struct Results {
    pub matches: Vec<ByteRange>,
    pub completion: Completion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplacementError {
    PlanTooLarge,
    InvalidRange,
    StaleMatch,
    DocumentTooLarge,
}

/// Streaming KMP over Unicode scalars; offsets always refer to original UTF-8 bytes.
pub fn scan(
    snapshot: &DocumentSnapshot,
    query: &Query,
    cancel: &AtomicBool,
    limit: usize,
) -> Results {
    scan_progress(snapshot, query, cancel, limit, |_| {})
}

pub fn scan_progress(
    snapshot: &DocumentSnapshot,
    query: &Query,
    cancel: &AtomicBool,
    limit: usize,
    mut publish: impl FnMut(Results),
) -> Results {
    let mut last_publish = std::time::Instant::now();
    let mut result = Results {
        matches: Vec::new(),
        completion: Completion::Complete,
    };
    if query.pattern.is_empty() {
        return result;
    }
    let insensitive = query.case == CaseMode::Insensitive
        || (query.case == CaseMode::Auto && !query.pattern.chars().any(char::is_uppercase));
    let mut folds = HashMap::new();
    if insensitive {
        for c in query.pattern.chars() {
            if cancel.load(Ordering::Relaxed) {
                return Results {
                    matches: vec![],
                    completion: Completion::Cancelled,
                };
            }
            if folds.contains_key(&c) {
                continue;
            }
            let mut class = ClassUnicode::new([ClassUnicodeRange::new(c, c)]);
            class.case_fold_simple();
            let canonical = class.iter().next().unwrap().start();
            for range in class.iter() {
                for ch in range.start()..=range.end() {
                    folds.insert(ch, canonical);
                }
            }
        }
    }
    let ascii: [char; 128] = std::array::from_fn(|n| {
        folds
            .get(&(n as u8 as char))
            .copied()
            .unwrap_or(n as u8 as char)
    });
    let fold = |c: char| {
        if c.is_ascii() {
            ascii[c as usize]
        } else {
            folds.get(&c).copied().unwrap_or(c)
        }
    };
    let pattern: Vec<char> = query.pattern.chars().map(fold).collect();
    let mut prefix = vec![0; pattern.len()];
    for i in 1..pattern.len() {
        let mut j = prefix[i - 1];
        while j > 0 && pattern[i] != pattern[j] {
            j = prefix[j - 1];
        }
        if pattern[i] == pattern[j] {
            j += 1;
        }
        prefix[i] = j;
    }
    let mut matched = 0;
    let mut starts = VecDeque::with_capacity(pattern.len());
    let mut offset = query.scope.start.0;
    let end = query.scope.end.0.min(snapshot.len_bytes());
    while offset < end {
        if cancel.load(Ordering::Relaxed) {
            result.completion = Completion::Cancelled;
            return result;
        }
        let Some(chunk) = snapshot.chunk_at(ByteOffset(offset)) else {
            break;
        };
        let local = (offset - chunk.start.0) as usize;
        if !chunk.text.is_char_boundary(local) {
            break;
        }
        for (index, c) in chunk.text[local..].char_indices() {
            let start = offset + index as u64;
            let stop = start + c.len_utf8() as u64;
            if stop > end {
                return result;
            }
            if index % 1024 == 0 && cancel.load(Ordering::Relaxed) {
                result.completion = Completion::Cancelled;
                return result;
            }
            starts.push_back(start);
            if starts.len() > pattern.len() {
                starts.pop_front();
            }
            let c = fold(c);
            while matched > 0 && c != pattern[matched] {
                matched = prefix[matched - 1];
            }
            if c == pattern[matched] {
                matched += 1;
            }
            if matched == pattern.len() {
                if result.matches.len() == limit {
                    result.completion = Completion::Truncated;
                    return result;
                }
                result
                    .matches
                    .push(ByteRange::new(*starts.front().unwrap(), stop));
                matched = 0;
            }
        }
        offset = chunk.start.0 + chunk.text.len() as u64;
        if last_publish.elapsed() >= std::time::Duration::from_millis(50) {
            publish(Results {
                matches: result.matches.clone(),
                completion: Completion::Partial,
            });
            last_publish = std::time::Instant::now();
        }
    }
    result
}

/// Reject stale/partial results and bound both replacement allocation and resulting document size.
pub fn replacement_plan(
    snapshot: &DocumentSnapshot,
    query: &Query,
    ranges: &[ByteRange],
    replacement: &str,
) -> Result<EditTransaction, ReplacementError> {
    let bytes = replacement
        .len()
        .checked_mul(ranges.len())
        .ok_or(ReplacementError::PlanTooLarge)?;
    if bytes > PLAN_LIMIT {
        return Err(ReplacementError::PlanTooLarge);
    }
    let mut output = snapshot.len_bytes();
    let mut previous = query.scope.start;
    for range in ranges {
        if range.start < previous || range.end > query.scope.end || range.is_empty() {
            return Err(ReplacementError::InvalidRange);
        }
        let verify = Query {
            scope: *range,
            ..query.clone()
        };
        if scan(snapshot, &verify, &AtomicBool::new(false), 1).matches != [*range] {
            return Err(ReplacementError::StaleMatch);
        }
        output = output
            .checked_sub(range.len())
            .and_then(|n| n.checked_add(replacement.len() as u64))
            .ok_or(ReplacementError::PlanTooLarge)?;
        previous = range.end;
    }
    if output > PLAN_LIMIT as u64 {
        return Err(ReplacementError::DocumentTooLarge);
    }
    Ok(EditTransaction::new(
        snapshot.revision(),
        ranges
            .iter()
            .map(|r| TextEdit::new(*r, replacement))
            .collect(),
    ))
}

/// Map a fixed-content scope across edits outside it. Interior changes require explicit recapture.
pub fn map_scope(scope: ByteRange, delta: &crate::document::RevisionDelta) -> Option<ByteRange> {
    let mut shift = 0i128;
    for edit in delta.edits.iter() {
        let insertion = edit.old.is_empty();
        if edit.old.end <= scope.start {
            shift += edit.new_len as i128 - edit.old.len() as i128;
        } else if edit.old.start >= scope.end {
            // Insertion at the end belongs outside the scope.
        } else if insertion || edit.old.start < scope.end {
            return None;
        }
    }
    Some(ByteRange::new(
        u64::try_from(scope.start.0 as i128 + shift).ok()?,
        u64::try_from(scope.end.0 as i128 + shift).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn find(text: &str, pattern: &str, case: CaseMode, limit: usize) -> Results {
        let s = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        scan(
            &s,
            &Query {
                pattern: pattern.into(),
                case,
                scope: ByteRange::new(0, s.len_bytes()),
            },
            &AtomicBool::new(false),
            limit,
        )
    }
    #[test]
    fn canonical_nonoverlap_and_limits() {
        assert_eq!(
            find("aaaaa", "aa", CaseMode::Sensitive, 10).matches,
            [ByteRange::new(0, 2), ByteRange::new(2, 4)]
        );
        assert_eq!(
            find("aa", "a", CaseMode::Auto, 2).completion,
            Completion::Complete
        );
        assert_eq!(
            find("aaa", "a", CaseMode::Auto, 2).completion,
            Completion::Truncated
        );
        assert!(find("abc", "", CaseMode::Auto, 10).matches.is_empty());
    }
    #[test]
    fn unicode_simple_folding_preserves_bytes() {
        assert_eq!(
            find("Σςσ KkK ßSS", "σ", CaseMode::Auto, 10).matches.len(),
            3
        );
        assert_eq!(
            find("KkK", "k", CaseMode::Auto, 10).matches,
            [
                ByteRange::new(0, 3),
                ByteRange::new(3, 4),
                ByteRange::new(4, 5)
            ]
        );
        assert!(find("ß", "ss", CaseMode::Auto, 10).matches.is_empty());
        assert_eq!(find("Aa", "A", CaseMode::Auto, 10).matches.len(), 1);
    }
    #[test]
    fn cross_chunk_multiline_and_long_pattern() {
        let p = format!("{}中文\r\n🙂", "x".repeat(2500));
        let text = format!("{}{}尾", " ".repeat(1023), p);
        assert_eq!(
            find(&text, &p, CaseMode::Sensitive, 10).matches,
            [ByteRange::new(1023, (1023 + p.len()) as u64)]
        );
        assert!(
            find("a\r\nb", "a\nb", CaseMode::Auto, 10)
                .matches
                .is_empty()
        );
    }
    #[test]
    fn cancelled_and_plan_validation() {
        let s = DocumentSnapshot::from_utf8(b"a a a".to_vec()).unwrap();
        let q = Query {
            pattern: "a".into(),
            case: CaseMode::Auto,
            scope: ByteRange::new(0, 5),
        };
        assert_eq!(
            scan(&s, &q, &AtomicBool::new(true), 10).completion,
            Completion::Cancelled
        );
        let plan =
            replacement_plan(&s, &q, &[ByteRange::new(2, 3), ByteRange::new(4, 5)], "aa").unwrap();
        assert_eq!(plan.edits.len(), 2);
        assert!(replacement_plan(&s, &q, &[ByteRange::new(1, 2)], "b").is_err());
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use crate::document::{Revision, RevisionDelta, TextEditSummary};
    fn mapped(scope: (u64, u64), edit: (u64, u64), len: u64) -> Option<ByteRange> {
        map_scope(
            ByteRange::new(scope.0, scope.1),
            &RevisionDelta::new(
                Revision(0),
                Revision(1),
                vec![TextEditSummary::new(ByteRange::new(edit.0, edit.1), len)],
            )
            .unwrap(),
        )
    }
    #[test]
    fn search_fixed_scope_endpoint_affinities() {
        assert_eq!(mapped((2, 5), (2, 2), 3), Some(ByteRange::new(5, 8)));
        assert_eq!(mapped((2, 5), (5, 5), 3), Some(ByteRange::new(2, 5)));
        assert_eq!(mapped((2, 2), (2, 2), 3), Some(ByteRange::new(5, 5)));
        assert_eq!(mapped((2, 5), (3, 3), 3), None);
        assert_eq!(mapped((2, 5), (1, 3), 0), None);
        assert_eq!(mapped((2, 5), (0, 2), 0), Some(ByteRange::new(0, 3)));
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    #[ignore = "explicit search performance report, no machine-dependent threshold"]
    fn search_performance_report() {
        for mib in [1, 10, 100] {
            let unit = "中文 abcdefghijklmnopqrstuvwxyz 0123456789\n";
            let mut text = unit.repeat(mib * 1024 * 1024 / unit.len());
            text.push_str(&" ".repeat(mib * 1024 * 1024 - text.len()));
            let snapshot = DocumentSnapshot::from_utf8(text.into_bytes()).unwrap();
            for pattern in ["不存在", "中文", "a"] {
                let query = Query {
                    pattern: pattern.into(),
                    case: CaseMode::Auto,
                    scope: ByteRange::new(0, snapshot.len_bytes()),
                };
                let mut timings = Vec::new();
                for _ in 0..3 {
                    let start = std::time::Instant::now();
                    let result = scan(&snapshot, &query, &AtomicBool::new(false), MATCH_LIMIT);
                    timings.push(start.elapsed().as_secs_f64() * 1000.);
                    assert!(result.matches.len() <= MATCH_LIMIT);
                }
                println!(
                    "search bytes={} pattern={pattern:?} scan_ms={timings:?} build={}",
                    snapshot.len_bytes(),
                    if cfg!(debug_assertions) {
                        "debug"
                    } else {
                        "release"
                    }
                );
            }
        }
    }
}
