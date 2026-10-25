//! Display-only text transformations with source coordinates retained through composition.

use std::ops::Range;

use super::Bias;

#[derive(Clone, Debug)]
struct Replacement {
    source: Range<usize>,
    display: Range<usize>,
}

#[derive(Clone, Debug)]
struct ProjectionMap {
    source_len: usize,
    display_len: usize,
    replacements: Vec<Replacement>,
}

/// Coordinates always refer to the original UTF-8 source, even after several transformations.
/// Display boundaries can represent both sides of hidden source; callers that need a particular
/// side use the bias-taking methods instead of guessing from the number of displayed bytes.
#[derive(Clone, Debug)]
pub(crate) struct TextProjection {
    pub(crate) text: String,
    source_len: usize,
    maps: Vec<ProjectionMap>,
}

impl TextProjection {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            source_len: text.len(),
            text,
            maps: Vec::new(),
        }
    }

    /// Replacements must be ordered, disjoint UTF-8 ranges in the current display text.
    pub(crate) fn replace(
        mut self,
        edits: impl IntoIterator<Item = (Range<usize>, String)>,
    ) -> Self {
        let mut edits = edits.into_iter().peekable();
        if edits.peek().is_none() {
            return self;
        }
        let mut text = String::with_capacity(self.text.len());
        let mut replacements = Vec::new();
        let mut cursor = 0;
        for (source, replacement) in edits {
            assert!(source.start >= cursor && source.end >= source.start);
            assert!(self.text.is_char_boundary(source.start));
            assert!(self.text.is_char_boundary(source.end));
            text.push_str(&self.text[cursor..source.start]);
            let start = text.len();
            text.push_str(&replacement);
            replacements.push(Replacement {
                source: source.clone(),
                display: start..text.len(),
            });
            cursor = source.end;
        }
        text.push_str(&self.text[cursor..]);
        self.maps.push(ProjectionMap {
            source_len: self.text.len(),
            display_len: text.len(),
            replacements,
        });
        self.text = text;
        self
    }

    pub(crate) fn then(mut self, next: Self) -> Self {
        assert_eq!(self.text.len(), next.source_len);
        self.text = next.text;
        self.maps.extend(next.maps);
        self
    }

    pub(crate) fn has_replacements(&self) -> bool {
        !self.maps.is_empty()
    }

    pub(crate) fn source_to_display(&self, source: usize) -> usize {
        self.maps
            .iter()
            .fold(source.min(self.source_len), |p, map| {
                map.source_to_display(p, None)
            })
    }

    pub(crate) fn source_to_display_with_bias(&self, source: usize, bias: Bias) -> usize {
        self.maps
            .iter()
            .fold(source.min(self.source_len), |p, map| {
                map.source_to_display(p, Some(bias))
            })
    }

    pub(crate) fn display_to_source(&self, display: usize) -> usize {
        self.maps
            .iter()
            .rev()
            .fold(display.min(self.text.len()), |p, map| {
                map.display_to_source(p, None)
            })
    }

    pub(crate) fn display_to_source_with_bias(&self, display: usize, bias: Bias) -> usize {
        self.maps
            .iter()
            .rev()
            .fold(display.min(self.text.len()), |p, map| {
                map.display_to_source(p, Some(bias))
            })
    }

    pub(crate) fn display_range(&self, source: Range<usize>) -> Range<usize> {
        self.source_to_display_with_bias(source.start, Bias::Left)
            ..self.source_to_display_with_bias(source.end, Bias::Right)
    }
}

impl ProjectionMap {
    fn source_to_display(&self, source: usize, bias: Option<Bias>) -> usize {
        let (mut source_end, mut display_end) = (0, 0);
        for replacement in &self.replacements {
            let raw = &replacement.source;
            let display = &replacement.display;
            if source < raw.start {
                break;
            }
            if source < raw.end {
                if source == raw.start {
                    return display.start;
                }
                return match bias.unwrap_or_else(|| nearest_bias(source - raw.start, raw.len())) {
                    Bias::Left => display.start,
                    Bias::Right => display.end,
                };
            }
            if raw.is_empty() && source == raw.start && bias == Some(Bias::Left) {
                return display.start;
            }
            source_end = raw.end;
            display_end = display.end;
        }
        (display_end + source.saturating_sub(source_end)).min(self.display_len)
    }

    fn display_to_source(&self, display: usize, bias: Option<Bias>) -> usize {
        let (mut source_end, mut display_end) = (0, 0);
        let mut boundary: Option<(usize, usize)> = None;
        for replacement in &self.replacements {
            let raw = &replacement.source;
            let visible = &replacement.display;
            if display < visible.start {
                break;
            }
            if visible.start < display && display < visible.end {
                return match bias
                    .unwrap_or_else(|| nearest_bias(display - visible.start, visible.len()))
                {
                    Bias::Left => raw.start,
                    Bias::Right => raw.end,
                };
            }
            if display == visible.start || display == visible.end {
                let (left, right) = if visible.is_empty() {
                    (raw.start, raw.end)
                } else if display == visible.start {
                    (raw.start, raw.start)
                } else {
                    (raw.end, raw.end)
                };
                boundary = Some(boundary.map_or((left, right), |(first, _)| (first, right)));
            }
            source_end = raw.end;
            display_end = visible.end;
        }
        if let Some((left, right)) = boundary {
            return match bias.unwrap_or(Bias::Right) {
                Bias::Left => left,
                Bias::Right => right,
            };
        }
        (source_end + display.saturating_sub(display_end)).min(self.source_len)
    }
}

fn nearest_bias(offset: usize, len: usize) -> Bias {
    if offset <= len / 2 {
        Bias::Left
    } else {
        Bias::Right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_boundaries_keep_both_source_sides() {
        let text = TextProjection::new("[[path][Guide]]")
            .replace([(0..8, String::new()), (13..15, String::new())]);
        assert_eq!(text.text, "Guide");
        assert_eq!(text.display_to_source_with_bias(0, Bias::Left), 0);
        assert_eq!(text.display_to_source_with_bias(0, Bias::Right), 8);
        assert_eq!(text.display_to_source_with_bias(5, Bias::Left), 13);
        assert_eq!(text.display_to_source_with_bias(5, Bias::Right), 15);
        assert_eq!(text.source_to_display(10), 2);
    }

    #[test]
    fn composition_retains_original_coordinates_and_utf8_boundaries() {
        let tabs = TextProjection::new("\t[[ref][目录😀]]").replace([(0..1, "    ".into())]);
        let link_start = 4;
        let label_start = tabs.text.find("目录").unwrap();
        let label_end = tabs.text.find("]]").unwrap();
        let next = TextProjection::new(tabs.text.clone()).replace([
            (link_start..label_start, String::new()),
            (label_end..tabs.text.len(), String::new()),
        ]);
        let text = tabs.then(next);
        assert_eq!(text.text, "    目录😀");
        assert_eq!(text.display_to_source(4), 8);
        assert_eq!(text.display_to_source(10), 14);
        assert_eq!(text.source_to_display(14), 10);
        assert_eq!(text.display_to_source(text.text.len()), text.source_len);
    }

    #[test]
    fn adjacent_hidden_ranges_share_one_boundary() {
        let text =
            TextProjection::new("abtext").replace([(0..1, String::new()), (1..2, String::new())]);
        assert_eq!(text.display_to_source_with_bias(0, Bias::Left), 0);
        assert_eq!(text.display_to_source_with_bias(0, Bias::Right), 2);
        assert_eq!(text.display_range(2..6), 0..4);
    }

    #[test]
    fn virtual_prefix_has_no_source_characters() {
        let text = TextProjection::new("body").replace([(0..0, "    ".into())]);
        assert_eq!(text.display_to_source(2), 0);
        assert_eq!(text.source_to_display_with_bias(0, Bias::Left), 0);
        assert_eq!(text.source_to_display_with_bias(0, Bias::Right), 4);
        assert_eq!(text.display_to_source(8), 4);
    }
}
