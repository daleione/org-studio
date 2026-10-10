//! Script-specific fonts shared by text measurement and rendering.
use std::ops::Range;

use gpui::{FontFallbacks, HighlightStyle, SharedString, StyledText, TextRun};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy)]
pub(crate) enum FontRole {
    Editor,
    Reading,
    Code,
    Ui,
}

impl FontRole {
    pub(crate) fn cjk_family(self) -> Option<&'static str> {
        let fonts = super::fonts();
        match self {
            Self::Editor => fonts.editor_cjk_family.as_deref(),
            Self::Reading => fonts.reading_cjk_family.as_deref(),
            Self::Code => fonts.code_cjk_family.as_deref(),
            Self::Ui => fonts.ui_cjk_family.as_deref(),
        }
    }
}

type FamilyOverride = (Range<usize>, SharedString);
type RoleOverride = (Range<usize>, FontRole, SharedString);

/// Inline overrides select the role and base family before applying its Chinese font.
pub(crate) fn font_overrides(
    text: &str,
    role: FontRole,
    overrides: &[RoleOverride],
) -> Vec<FamilyOverride> {
    if text.is_ascii()
        || role.cjk_family().is_none()
            && overrides
                .iter()
                .all(|(_, role, _)| role.cjk_family().is_none())
    {
        return overrides
            .iter()
            .map(|(range, _, family)| (range.clone(), family.clone()))
            .collect();
    }
    let chinese = cjk_ranges(text);
    let mut boundaries = vec![0, text.len()];
    for (range, _, _) in overrides {
        boundaries.extend([range.start, range.end]);
    }
    for range in &chinese {
        boundaries.extend([range.start, range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut result: Vec<FamilyOverride> = Vec::new();
    let mut chinese_index = 0;
    for pair in boundaries.windows(2) {
        let range = pair[0]..pair[1];
        while chinese
            .get(chinese_index)
            .is_some_and(|r| r.end <= range.start)
        {
            chinese_index += 1;
        }
        let inline = overrides.iter().find(|(r, _, _)| r.contains(&range.start));
        let selected_role = inline.map_or(role, |(_, role, _)| *role);
        let family = chinese
            .get(chinese_index)
            .filter(|r| r.contains(&range.start))
            .and_then(|_| selected_role.cjk_family())
            .map(SharedString::from)
            .or_else(|| inline.map(|(_, _, family)| family.clone()));
        if let Some(family) = family {
            if let Some((previous, previous_family)) = result.last_mut()
                && previous.end == range.start
                && *previous_family == family
            {
                previous.end = range.end;
            } else {
                result.push((range, family));
            }
        }
    }
    result
}

pub(crate) fn cjk_runs(text: &str, runs: Vec<TextRun>, role: FontRole) -> Vec<TextRun> {
    if role.cjk_family().is_none() || text.is_ascii() {
        return runs;
    }
    apply_font_runs(runs, &font_overrides(text, role, &[]))
}

/// Split existing runs without losing syntax colors, decorations, weight, or input underlines.
pub(crate) fn apply_font_runs(runs: Vec<TextRun>, overrides: &[FamilyOverride]) -> Vec<TextRun> {
    if overrides.is_empty() {
        return runs;
    }
    let mut result = Vec::with_capacity(runs.len() + overrides.len() * 2);
    let mut offset = 0;
    let mut override_index = 0;
    for run in runs {
        let end = offset + run.len;
        while offset < end {
            while overrides
                .get(override_index)
                .is_some_and(|(r, _)| r.end <= offset)
            {
                override_index += 1;
            }
            let current = overrides.get(override_index);
            let boundary = current.map_or(end, |(range, _)| {
                end.min(if offset < range.start {
                    range.start
                } else {
                    range.end
                })
            });
            let mut segment = run.clone();
            segment.len = boundary - offset;
            if let Some((range, family)) = current
                && range.contains(&offset)
                && segment.font.family != *family
            {
                let mut fallbacks = vec![segment.font.family.to_string()];
                if let Some(existing) = &segment.font.fallbacks {
                    for name in existing.0.iter() {
                        super::append_fallback(&mut fallbacks, name);
                    }
                }
                segment.font.fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
                segment.font.family = family.clone();
            }
            result.push(segment);
            offset = boundary;
        }
    }
    result
}

pub(crate) fn role_text(text: impl Into<SharedString>, role: FontRole) -> StyledText {
    let text = text.into();
    if text.is_ascii() || role.cjk_family().is_none() {
        return StyledText::new(text);
    }
    styled_text(text, role, [])
}

pub(crate) fn styled_text(
    text: SharedString,
    role: FontRole,
    highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
) -> StyledText {
    let overrides = font_overrides(&text, role, &[]);
    styled_text_with_overrides(text, highlights, overrides)
}

pub(crate) fn styled_text_with_overrides(
    text: SharedString,
    highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    overrides: Vec<FamilyOverride>,
) -> StyledText {
    let highlights = highlights.into_iter().collect::<Vec<_>>();
    if overrides.is_empty() {
        return StyledText::new(text).with_highlights(highlights);
    }
    // GPUI applies family overrides to whole runs, so split highlights at every font boundary.
    let mut boundaries = vec![0, text.len()];
    for range in highlights
        .iter()
        .map(|(range, _)| range)
        .chain(overrides.iter().map(|(range, _)| range))
    {
        boundaries.extend([range.start, range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut highlight_index = 0;
    let highlights = boundaries
        .windows(2)
        .map(|pair| {
            let range = pair[0]..pair[1];
            while highlights
                .get(highlight_index)
                .is_some_and(|(r, _)| r.end <= range.start)
            {
                highlight_index += 1;
            }
            let highlight = highlights
                .get(highlight_index)
                .filter(|(r, _)| r.contains(&range.start))
                .map_or_else(HighlightStyle::default, |(_, highlight)| *highlight);
            (range, highlight)
        })
        .collect::<Vec<_>>();
    StyledText::new(text)
        .with_highlights(highlights)
        .with_font_family_overrides(overrides)
}

/// Keep grapheme clusters intact, including variation selectors and emoji sequences.
pub(crate) fn cjk_ranges(text: &str) -> Vec<Range<usize>> {
    if text.is_ascii() {
        return Vec::new();
    }
    let graphemes = text.grapheme_indices(true).collect::<Vec<_>>();
    let context = graphemes
        .iter()
        .map(|(_, grapheme)| {
            if grapheme.chars().any(is_cjk) {
                Some(true)
            } else if grapheme.chars().any(char::is_alphanumeric) {
                Some(false)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut next = None;
    let mut following = vec![None; context.len()];
    for index in (0..context.len()).rev() {
        following[index] = next;
        next = if graphemes[index].1.contains('\n') {
            None
        } else {
            context[index].or(next)
        };
    }
    let mut previous = None;
    let mut result: Vec<Range<usize>> = Vec::new();
    for (index, &(start, grapheme)) in graphemes.iter().enumerate() {
        let chinese = if grapheme.chars().any(is_shared_punctuation) {
            let neighbor = if grapheme.starts_with(['‘', '“']) {
                following[index].or(previous)
            } else {
                previous.or(following[index])
            };
            neighbor.unwrap_or(false)
        } else {
            context[index] == Some(true)
        };
        previous = if grapheme.contains('\n') {
            None
        } else {
            context[index].or(previous)
        };
        if chinese {
            let end = start + grapheme.len();
            if let Some(range) = result.last_mut()
                && range.end == start
            {
                range.end = end;
            } else {
                result.push(start..end);
            }
        }
    }
    result
}

fn is_cjk(character: char) -> bool {
    matches!(
        character.script(),
        Script::Han | Script::Hiragana | Script::Katakana | Script::Hangul | Script::Bopomofo
    ) || matches!(character as u32, 0x2e80..=0x2fff | 0x3000..=0x303f | 0xfe10..=0xfe1f | 0xfe30..=0xfe4f | 0xff01..=0xff60 | 0xffe0..=0xffe6)
}

fn is_shared_punctuation(character: char) -> bool {
    matches!(character, '·' | '‐'..='―' | '‘'..='‟' | '…')
}
