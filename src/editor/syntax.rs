use std::{ops::Range, path::Path};

use gpui::{TextRun, UnderlineStyle, px, rgb};

use crate::theme::Theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Language {
    Org,
    Markdown,
}

fn language(path: &Path) -> Language {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("md" | "markdown") => Language::Markdown,
        _ => Language::Org,
    }
}

pub(super) fn cache_key(path: &Path) -> u8 {
    match language(path) {
        Language::Org => 0,
        Language::Markdown => 1,
    }
}

/// Produces paint-only decoration runs. Source/display coordinates never depend on these styles.
pub(super) fn runs(
    path: &Path,
    text: &str,
    base: TextRun,
    marked: Option<Range<usize>>,
    theme: &Theme,
) -> Vec<TextRun> {
    let mut spans = Vec::<(Range<usize>, u32)>::new();
    let trimmed = text.trim_start();
    let indent = text.len() - trimmed.len();
    match language(path) {
        Language::Org => {
            let stars = trimmed.bytes().take_while(|byte| *byte == b'*').count();
            if stars > 0 && trimmed.as_bytes().get(stars) == Some(&b' ') {
                spans.push((indent..text.len(), theme.heading[(stars - 1).min(3)]));
            } else if trimmed.starts_with("#+") {
                spans.push((indent..text.len(), theme.meta));
            } else if trimmed.starts_with('#') {
                spans.push((indent..text.len(), theme.comment));
            } else if trimmed.starts_with(':') && trimmed.ends_with(':') {
                spans.push((indent..text.len(), theme.attribute));
            } else if ["TODO", "DONE", "SCHEDULED:", "DEADLINE:", "CLOSED:"]
                .iter()
                .any(|keyword| trimmed.starts_with(keyword))
            {
                spans.push((indent..text.len(), theme.keyword));
            }
            collect_delimited(text, "[[", "]]", theme.link, &mut spans);
            collect_delimited(text, "<", ">", theme.date, &mut spans);
        }
        Language::Markdown => {
            if trimmed.starts_with('#') {
                spans.push((indent..text.len(), theme.heading[0]));
            } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                spans.push((indent..text.len(), theme.code_boundary));
            } else if trimmed.starts_with('>') {
                spans.push((indent..text.len(), theme.quote));
            } else if trimmed.starts_with("<!--") {
                spans.push((indent..text.len(), theme.comment));
            }
            collect_delimited(text, "[", "]", theme.link, &mut spans);
            collect_delimited(text, "`", "`", theme.inline_code, &mut spans);
        }
    }

    let mut boundaries = vec![0, text.len()];
    for (range, _) in &spans {
        boundaries.extend([range.start, range.end]);
    }
    if let Some(marked) = &marked {
        boundaries.extend([marked.start, marked.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .filter_map(|boundary| {
            let range = boundary[0]..boundary[1];
            if range.is_empty() {
                return None;
            }
            let color = spans
                .iter()
                .rev()
                .find(|(span, _)| span.start <= range.start && range.start < span.end)
                .map_or(base.color, |(_, color)| rgb(*color).into());
            let underline = marked
                .as_ref()
                .filter(|marked| marked.start < range.end && range.start < marked.end)
                .map(|_| UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.0),
                    wavy: false,
                });
            Some(TextRun {
                len: range.len(),
                color,
                underline,
                ..base.clone()
            })
        })
        .collect()
}

fn collect_delimited(
    text: &str,
    open: &str,
    close: &str,
    color: u32,
    spans: &mut Vec<(Range<usize>, u32)>,
) {
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find(open).map(|index| cursor + index) {
        let body = start + open.len();
        let Some(end) = text[body..]
            .find(close)
            .map(|index| body + index + close.len())
        else {
            break;
        };
        spans.push((start..end, color));
        cursor = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::current_theme;

    #[test]
    fn decoration_runs_preserve_text_length() {
        let text = "** Heading [[target]]";
        let base = TextRun {
            len: text.len(),
            font: Default::default(),
            color: rgb(0).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = runs(Path::new("a.org"), text, base, None, current_theme());
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), text.len());
    }
}
