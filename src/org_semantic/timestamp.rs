use jiff::civil::{Date, Time};

use crate::document::ByteRange;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimestampKind {
    Plain,
    Scheduled,
    Deadline,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepeaterMode {
    Cumulative,
    CatchUp,
    Restart,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeUnit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Repeater {
    pub(crate) mode: RepeaterMode,
    pub(crate) value: u32,
    pub(crate) unit: TimeUnit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WarningPeriod {
    pub(crate) delayed: bool,
    pub(crate) value: u32,
    pub(crate) unit: TimeUnit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrgTimestamp {
    pub(crate) kind: TimestampKind,
    pub(crate) active: bool,
    pub(crate) start_date: Date,
    pub(crate) start_time: Option<Time>,
    pub(crate) end_date: Option<Date>,
    pub(crate) end_time: Option<Time>,
    pub(crate) repeater: Option<Repeater>,
    pub(crate) warning: Option<WarningPeriod>,
    pub(crate) source_range: ByteRange,
}

pub(super) fn parse_timestamps(text: &str, source_start: u64) -> Vec<OrgTimestamp> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(relative) = text[cursor..].find(['<', '[']) else {
            break;
        };
        let start = cursor + relative;
        let opener = bytes[start];
        let closer = if opener == b'<' { b'>' } else { b']' };
        let Some(close_relative) = bytes[start + 1..].iter().position(|byte| *byte == closer)
        else {
            break;
        };
        let close = start + 1 + close_relative;
        if opener == b'<' && bytes.get(start + 1..start + 3) == Some(b"%%") {
            cursor = close + 1;
            continue;
        }
        let kind = planning_kind(&text[..start]);
        if let Some(mut timestamp) = parse_single(
            &text[start + 1..close],
            opener == b'<',
            kind,
            ByteRange::new(source_start + start as u64, source_start + close as u64 + 1),
        ) {
            let mut consumed = close + 1;
            if bytes.get(consumed..consumed + 2) == Some(b"--")
                && matches!(bytes.get(consumed + 2), Some(b'<') | Some(b'['))
            {
                let second_start = consumed + 2;
                let second_closer = if bytes[second_start] == b'<' {
                    b'>'
                } else {
                    b']'
                };
                if let Some(second_relative) = bytes[second_start + 1..]
                    .iter()
                    .position(|byte| *byte == second_closer)
                {
                    let second_close = second_start + 1 + second_relative;
                    if let Some(end) = parse_single(
                        &text[second_start + 1..second_close],
                        bytes[second_start] == b'<',
                        kind,
                        ByteRange::new(0, 0),
                    ) {
                        timestamp.end_date = Some(end.start_date);
                        timestamp.end_time = end.start_time;
                        timestamp.source_range.end.0 = source_start + second_close as u64 + 1;
                        consumed = second_close + 1;
                    }
                }
            }
            result.push(timestamp);
            cursor = consumed;
        } else {
            cursor = close + 1;
        }
    }
    result
}

fn parse_single(
    inner: &str,
    active: bool,
    kind: TimestampKind,
    source_range: ByteRange,
) -> Option<OrgTimestamp> {
    let date = parse_date(inner.get(..10)?)?;
    let mut start_time = None;
    let mut end_time = None;
    let mut repeater = None;
    let mut warning = None;
    for token in inner[10..].split_whitespace() {
        if let Some((start, end)) = token.split_once('-')
            && let (Some(start), Some(end)) = (parse_time(start), parse_time(end))
        {
            start_time = Some(start);
            end_time = Some(end);
            continue;
        }
        if let Some(time) = parse_time(token) {
            start_time = Some(time);
        } else if let Some(value) = parse_repeater(token) {
            repeater = Some(value);
        } else if let Some(value) = parse_warning(token) {
            warning = Some(value);
        }
    }
    Some(OrgTimestamp {
        kind,
        active,
        start_date: date,
        start_time,
        end_date: None,
        end_time,
        repeater,
        warning,
        source_range,
    })
}

fn parse_date(text: &str) -> Option<Date> {
    if text.as_bytes().get(4) != Some(&b'-') || text.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    Date::new(
        text[..4].parse().ok()?,
        text[5..7].parse().ok()?,
        text[8..10].parse().ok()?,
    )
    .ok()
}

fn parse_time(text: &str) -> Option<Time> {
    let (hour, minute) = text.split_once(':')?;
    if hour.len() != 2 || minute.len() != 2 {
        return None;
    }
    Time::new(hour.parse().ok()?, minute.parse().ok()?, 0, 0).ok()
}

fn parse_repeater(token: &str) -> Option<Repeater> {
    let (mode, body) = if let Some(body) = token.strip_prefix("++") {
        (RepeaterMode::CatchUp, body)
    } else if let Some(body) = token.strip_prefix(".+") {
        (RepeaterMode::Restart, body)
    } else {
        (RepeaterMode::Cumulative, token.strip_prefix('+')?)
    };
    let (value, unit) = parse_period(body)?;
    Some(Repeater { mode, value, unit })
}

fn parse_warning(token: &str) -> Option<WarningPeriod> {
    let (delayed, body) = token
        .strip_prefix("--")
        .map(|body| (true, body))
        .or_else(|| token.strip_prefix('-').map(|body| (false, body)))?;
    let (value, unit) = parse_period(body)?;
    Some(WarningPeriod {
        delayed,
        value,
        unit,
    })
}

fn parse_period(body: &str) -> Option<(u32, TimeUnit)> {
    let (number, unit) = body.split_at(body.len().checked_sub(1)?);
    let unit = match unit {
        "h" => TimeUnit::Hour,
        "d" => TimeUnit::Day,
        "w" => TimeUnit::Week,
        "m" => TimeUnit::Month,
        "y" => TimeUnit::Year,
        _ => return None,
    };
    let value = number.parse().ok()?;
    (value > 0).then_some((value, unit))
}

fn planning_kind(prefix: &str) -> TimestampKind {
    let tail = prefix.split_whitespace().last().unwrap_or(prefix);
    if tail.ends_with("SCHEDULED:") {
        TimestampKind::Scheduled
    } else if tail.ends_with("DEADLINE:") {
        TimestampKind::Deadline
    } else if tail.ends_with("CLOSED:") {
        TimestampKind::Closed
    } else {
        TimestampKind::Plain
    }
}
