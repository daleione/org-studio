//! Lossless editing of individual Org timestamps. Agenda's aggregate timestamp
//! deliberately flattens ranges; an editor must retain each endpoint and cookie.
use std::ops::Range;

use jiff::civil::{Date, Time};

use super::{
    OrgTimestamp, Repeater, RepeaterMode, TimeUnit, TimestampKind, WarningPeriod, timestamp,
};
use crate::document::ByteRange;

#[derive(Clone, Debug)]
pub(crate) struct TimestampEndpoint {
    source: String,
    original: OrgTimestamp,
    pub(crate) value: OrgTimestamp,
    weekday: Option<Range<usize>>,
    time: Option<Range<usize>>,
    repeat: Option<Range<usize>>,
    warning: Option<Range<usize>>,
    repeat_bound: String,
}

impl TimestampEndpoint {
    fn parse(source: &str, kind: TimestampKind) -> Option<Self> {
        let active = source.starts_with('<');
        if !(active || source.starts_with('[')) || !source.ends_with(if active { '>' } else { ']' })
        {
            return None;
        }
        let inner = source.get(1..source.len().checked_sub(1)?)?;
        // Guard byte slicing in the semantic parser for malformed Unicode dates.
        if inner.len() < 10
            || !inner.as_bytes()[..10].is_ascii()
            || inner
                .as_bytes()
                .get(10)
                .is_some_and(|b| !b.is_ascii_whitespace())
        {
            return None;
        }
        let mut value =
            timestamp::parse_single(inner, active, kind, ByteRange::new(0, source.len() as u64))?;
        let mut weekday = None;
        let mut time = None;
        let mut repeat = None;
        let mut warning = None;
        let mut repeat_bound = String::new();
        let mut offset = 11;
        for token in inner[10..].split_whitespace() {
            let start = offset + source[offset..].find(token)?;
            let range = start..start + token.len();
            offset = range.end;
            let repeat_body = token.split('/').next().unwrap_or(token);
            if timestamp::parse_time(token).is_some()
                || token.split_once('-').is_some_and(|(a, b)| {
                    timestamp::parse_time(a).is_some() && timestamp::parse_time(b).is_some()
                })
            {
                if time.replace(range).is_some() {
                    return None;
                }
            } else if let Some(repeater) = timestamp::parse_repeater(repeat_body) {
                if repeat.replace(range).is_some() {
                    return None;
                }
                value.repeater = Some(repeater);
                repeat_bound = token[repeat_body.len()..].to_owned();
            } else if timestamp::parse_warning(token).is_some() {
                if warning.replace(range).is_some() {
                    return None;
                }
            } else if weekday.is_none() && is_weekday(token) {
                weekday = Some(range);
            } else if token.starts_with(['+', '-'])
                || token.starts_with(".+")
                || (token.as_bytes()[0].is_ascii_digit() && token.contains(':'))
            {
                return None;
            }
            // Unrecognized extensions are left untouched, including habit cookies.
        }
        Some(Self {
            source: source.to_owned(),
            original: value.clone(),
            value,
            weekday,
            time,
            repeat,
            warning,
            repeat_bound,
        })
    }

    pub(crate) fn serialize(&self) -> String {
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut additions = std::collections::BTreeMap::<usize, Vec<String>>::new();
        let close = self.source.len() - 1;
        if self.value.start_date != self.original.start_date {
            edits.push((1..11, self.value.start_date.to_string()));
            if let Some(range) = &self.weekday {
                edits.push((range.clone(), weekday(self.value.start_date).to_owned()));
            } else {
                edits.push((11..11, format!(" {}", weekday(self.value.start_date))));
            }
        }
        if self.value.active != self.original.active {
            edits.push((0..1, if self.value.active { "<" } else { "[" }.into()));
            edits.push((
                self.source.len() - 1..self.source.len(),
                if self.value.active { ">" } else { "]" }.into(),
            ));
        }
        let mut field = |span: &Option<Range<usize>>, content: String, insertion: usize| {
            if let Some(span) = span {
                edits.push((span.clone(), content));
            } else if !content.is_empty() {
                additions.entry(insertion).or_default().push(content);
            }
        };
        if (self.value.start_time, self.value.end_time)
            != (self.original.start_time, self.original.end_time)
        {
            let mut text = self.value.start_time.map(time_text).unwrap_or_default();
            if let Some(end) = self
                .value
                .end_time
                .filter(|_| self.value.start_time.is_some())
            {
                text.push('-');
                text.push_str(&time_text(end));
            }
            field(
                &self.time,
                text,
                self.repeat
                    .iter()
                    .chain(self.warning.iter())
                    .map(|r| r.start)
                    .min()
                    .unwrap_or(close),
            );
        }
        if self.value.repeater != self.original.repeater {
            field(
                &self.repeat,
                self.value
                    .repeater
                    .map(|r| format!("{}{}", repeater_text(r), self.repeat_bound))
                    .unwrap_or_default(),
                self.warning.as_ref().map_or(close, |r| r.start),
            );
        }
        if self.value.warning != self.original.warning {
            field(
                &self.warning,
                self.value.warning.map(warning_text).unwrap_or_default(),
                close,
            );
        }
        for (position, fields) in additions {
            let text = if position == close {
                format!(" {}", fields.join(" "))
            } else {
                format!("{} ", fields.join(" "))
            };
            edits.push((position..position, text));
        }
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        let mut result = self.source.clone();
        for (range, text) in edits.into_iter().rev() {
            result.replace_range(range, &text);
        }
        result
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TimestampDraft {
    pub(crate) start: TimestampEndpoint,
    pub(crate) end: Option<TimestampEndpoint>,
}

impl TimestampDraft {
    pub(crate) fn parse(source: &str, kind: TimestampKind) -> Option<Self> {
        let close = source.find(if source.starts_with('<') { '>' } else { ']' })?;
        let start = TimestampEndpoint::parse(&source[..=close], kind)?;
        let rest = &source[close + 1..];
        let end = if rest.is_empty() {
            None
        } else {
            Some(TimestampEndpoint::parse(rest.strip_prefix("--")?, kind)?)
        };
        Some(Self { start, end })
    }

    pub(crate) fn serialize(&self) -> String {
        let mut result = self.start.serialize();
        if let Some(end) = &self.end {
            result.push_str("--");
            result.push_str(&end.serialize());
        }
        result
    }

    pub(crate) fn set_range(&mut self, enabled: bool) {
        if enabled && self.end.is_none() {
            let value = &self.start.value;
            let source = format!(
                "{}{} {}{}",
                if value.active { '<' } else { '[' },
                value.start_date,
                weekday(value.start_date),
                if value.active { '>' } else { ']' }
            );
            self.end = TimestampEndpoint::parse(&source, value.kind);
        } else if !enabled {
            self.end = None;
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.end
            .as_ref()
            .is_none_or(|end| end.value.start_date >= self.start.value.start_date)
            && [&self.start]
                .into_iter()
                .chain(self.end.as_ref())
                .all(|endpoint| {
                    endpoint.value.end_time.is_none() || endpoint.value.start_time.is_some()
                })
    }
}

/// Only scans a single visible source line. A range is one hit, with both endpoints
/// retained. Diary expressions are surfaced to the source editor, never evaluated.
pub(crate) fn timestamp_at(text: &str, offset: usize) -> Option<(Range<usize>, TimestampKind)> {
    for (start, ch) in text.char_indices().filter(|(_, c)| matches!(c, '<' | '[')) {
        let Some(close_relative) = text[start..].find(if ch == '<' { '>' } else { ']' }) else {
            continue;
        };
        let close = start + close_relative + 1;
        let mut end = close;
        if text[close..].starts_with("--<") || text[close..].starts_with("--[") {
            let next = close + 2;
            if let Some(length) = text[next..].find(if text.as_bytes()[next] == b'<' {
                '>'
            } else {
                ']'
            }) {
                end = next + length + 1;
            }
        }
        if offset < start || offset >= end {
            continue;
        }
        let kind = match text[..start].split_whitespace().last() {
            Some("SCHEDULED:") => TimestampKind::Scheduled,
            Some("DEADLINE:") => TimestampKind::Deadline,
            Some("CLOSED:") => TimestampKind::Closed,
            _ => TimestampKind::Plain,
        };
        let source = &text[start..end];
        if TimestampDraft::parse(source, kind).is_some() || is_diary(source) {
            return Some((start..end, kind));
        }
    }
    None
}

pub(crate) fn is_diary(source: &str) -> bool {
    source.starts_with("<%%(") && source.ends_with('>') && !source.contains(['\n', '\r'])
}
fn is_weekday(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "mon"
            | "tue"
            | "wed"
            | "thu"
            | "fri"
            | "sat"
            | "sun"
            | "monday"
            | "tuesday"
            | "wednesday"
            | "thursday"
            | "friday"
            | "saturday"
            | "sunday"
            | "一"
            | "二"
            | "三"
            | "四"
            | "五"
            | "六"
            | "日"
            | "周一"
            | "周二"
            | "周三"
            | "周四"
            | "周五"
            | "周六"
            | "周日"
            | "星期一"
            | "星期二"
            | "星期三"
            | "星期四"
            | "星期五"
            | "星期六"
            | "星期日"
    )
}
pub(crate) fn time_text(time: Time) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}
pub(crate) fn weekday(date: Date) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        [date.weekday().to_monday_zero_offset() as usize]
}
fn unit_code(unit: TimeUnit) -> char {
    match unit {
        TimeUnit::Hour => 'h',
        TimeUnit::Day => 'd',
        TimeUnit::Week => 'w',
        TimeUnit::Month => 'm',
        TimeUnit::Year => 'y',
    }
}
fn repeater_text(r: Repeater) -> String {
    format!(
        "{}{}{}",
        match r.mode {
            RepeaterMode::Cumulative => "+",
            RepeaterMode::CatchUp => "++",
            RepeaterMode::Restart => ".+",
        },
        r.value,
        unit_code(r.unit)
    )
}
fn warning_text(w: WarningPeriod) -> String {
    format!(
        "{}{}{}",
        if w.delayed { "--" } else { "-" },
        w.value,
        unit_code(w.unit)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adding_time_places_it_before_existing_cookies() {
        let mut d =
            TimestampDraft::parse("<2026-09-15 Tue ++1w -3d>", TimestampKind::Deadline).unwrap();
        d.start.value.start_time = Some(Time::new(14, 0, 0, 0).unwrap());
        assert_eq!(d.serialize(), "<2026-09-15 Tue 14:00 ++1w -3d>");
        d.start.value.repeater = None;
        assert_eq!(
            TimestampDraft::parse(&d.serialize(), TimestampKind::Deadline)
                .unwrap()
                .start
                .value
                .start_time,
            d.start.value.start_time
        );
    }
    #[test]
    fn malformed_tokens_do_not_panic_or_become_editable_dates() {
        for source in [
            "<2026-09-15 99:00>",
            "<2026-09-15 +中文>",
            "<2026-09-15 -é>",
            "<2026-09-150>",
            "<2026-02-30 Tue>",
            "<2026-09-15 12:00 13:00>",
        ] {
            assert!(
                TimestampDraft::parse(source, TimestampKind::Plain).is_none(),
                "{source}"
            );
        }
        assert!(
            TimestampDraft::parse("<2026-09-15 Tue 9:00 -0d>", TimestampKind::Deadline).is_some()
        );
        assert!(timestamp_at("[ incomplete <2026-09-15 Tue>", 20).is_some());
    }
    #[test]
    fn lossless_roundtrip_including_independent_range_cookies() {
        for source in [
            "<2026-09-15 Tue 14:00>",
            "[2026-09-15]",
            "<2026-09-15 二  14:00-15:30 ++2w -3d>",
            "<2026-09-15 Tue .+1d/3d --2d>",
            "<2026-09-15 Tue 09:00-10:00 +1w>--<2026-09-18 Fri 11:00-12:00 ++2m -3d>",
        ] {
            assert_eq!(
                TimestampDraft::parse(source, TimestampKind::Plain)
                    .unwrap()
                    .serialize(),
                source
            );
        }
    }
    #[test]
    fn date_edit_preserves_every_other_field() {
        let mut draft = TimestampDraft::parse(
            "<2026-09-15 Tue 09:00-10:00 .+1d/3d -2d>--<2026-09-18 Fri 11:00-12:00 ++2m --3d>",
            TimestampKind::Scheduled,
        )
        .unwrap();
        draft.start.value.start_date = Date::new(2026, 9, 16).unwrap();
        assert_eq!(
            draft.serialize(),
            "<2026-09-16 Wed 09:00-10:00 .+1d/3d -2d>--<2026-09-18 Fri 11:00-12:00 ++2m --3d>"
        );
    }
    #[test]
    fn adding_repeat_keeps_warning_last_and_brackets_independent() {
        let mut draft = TimestampDraft::parse("[2026-09-15 -3d]", TimestampKind::Deadline).unwrap();
        draft.start.value.repeater = Some(Repeater {
            mode: RepeaterMode::CatchUp,
            value: 1,
            unit: TimeUnit::Week,
        });
        draft.start.value.active = true;
        assert_eq!(draft.serialize(), "<2026-09-15 ++1w -3d>");
    }
    #[test]
    fn hits_unicode_ranges_and_diary_without_matching_links() {
        let text = "中文 DEADLINE: <2026-09-15 Tue>--<2026-09-18 Fri>";
        let (range, kind) = timestamp_at(text, text.find("18").unwrap()).unwrap();
        assert_eq!(&text[range], "<2026-09-15 Tue>--<2026-09-18 Fri>");
        assert_eq!(kind, TimestampKind::Deadline);
        assert!(timestamp_at("[[file:test.org]]", 4).is_none());
        assert!(timestamp_at("<%%(diary-float t 4 2) 22:00-23:00>", 5).is_some());
        assert!(TimestampDraft::parse("<中文中文中文>", TimestampKind::Plain).is_none());
    }
}
