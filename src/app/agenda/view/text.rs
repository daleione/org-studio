use crate::{
    agenda::{AgendaDateKind, AgendaResultSnapshot, format_agenda_text},
    i18n::Language,
};
use gpui::{FontWeight, HighlightStyle, rgb};
use jiff::civil::Date;

pub(crate) struct AgendaTextBuffer {
    pub(crate) text: String,
    pub(crate) highlights: Vec<crate::editor::LineHighlights>,
    pub(crate) targets: Vec<Option<crate::agenda::TaskKey>>,
}

pub(crate) fn agenda_text(
    result: &AgendaResultSnapshot,
    language: Language,
    window: Option<(Date, Date)>,
) -> AgendaTextBuffer {
    let today = jiff::Zoned::now().date();
    let lines = format_agenda_text(
        result,
        language.text("agenda.scheduled"),
        language.text("agenda.deadline"),
    );
    let mut groups = std::collections::BTreeMap::<Option<Date>, Vec<usize>>::new();
    for (index, row) in result.rows.iter().enumerate() {
        let date = row
            .date
            .map(|date| window.map_or(date, |(start, _)| date.max(start)));
        groups.entry(date).or_default().push(index);
    }
    if let Some((start, end)) = window {
        let mut date = start;
        while date <= end {
            groups.entry(Some(date)).or_default();
            let Ok(next) = date.checked_add(jiff::Span::new().days(1)) else {
                break;
            };
            date = next;
        }
    }
    let mut content = AgendaTextBuffer {
        text: String::new(),
        highlights: Vec::new(),
        targets: Vec::new(),
    };
    for (day_index, (date, rows)) in groups.into_iter().enumerate() {
        let header = date.map_or_else(
            || language.text("agenda.unscheduled").to_owned(),
            |date| {
                date_heading(
                    date,
                    language,
                    day_index == 0 || date.weekday().to_monday_zero_offset() == 0,
                )
            },
        );
        let weekend = date.is_some_and(|date| date.weekday().to_monday_zero_offset() >= 5);
        content.highlights.push(vec![(
            0..header.len(),
            HighlightStyle {
                color: Some(
                    rgb(if date == Some(today) {
                        0xd291d8
                    } else if weekend {
                        0x732b79
                    } else {
                        0xb54cbd
                    })
                    .into(),
                ),
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
        )]);
        content.text.push_str(&header);
        content.text.push('\n');
        content.targets.push(None);
        for index in rows {
            let row = &result.rows[index];
            let line = &lines[index];
            let deadline = row.date_kind == Some(AgendaDateKind::Deadline);
            let overdue = deadline && row.date.is_some_and(|date| date < today);
            let mut highlights = vec![
                (
                    0..line.text.len(),
                    HighlightStyle {
                        color: Some(
                            rgb(if overdue {
                                0xff4a40
                            } else if deadline {
                                0x986801
                            } else {
                                0x383a42
                            })
                            .into(),
                        ),
                        font_weight: Some(FontWeight::NORMAL),
                        ..Default::default()
                    },
                ),
                (
                    line.todo.clone(),
                    HighlightStyle {
                        color: Some(rgb(0x50a14f).into()),
                        font_weight: Some(FontWeight::BOLD),
                        ..Default::default()
                    },
                ),
            ];
            if let Some(range) = &line.priority {
                highlights.push((
                    range.clone(),
                    HighlightStyle {
                        color: Some(
                            rgb(match row.priority {
                                Some('A') => 0xff4a40,
                                Some('B') => 0x986801,
                                _ => 0x50a14f,
                            })
                            .into(),
                        ),
                        ..Default::default()
                    },
                ));
            }
            if let Some(range) = &line.tags {
                highlights.push((
                    range.clone(),
                    HighlightStyle {
                        color: Some(rgb(0x986801).into()),
                        ..Default::default()
                    },
                ));
            }
            content.text.push_str(&line.text);
            content.text.push('\n');
            content.highlights.push(highlights);
            content.targets.push(Some(row.task));
        }
    }
    content
}

fn date_heading(date: Date, language: Language, week: bool) -> String {
    let mut heading = match language {
        Language::English => format!(
            "{:<12} {}",
            date.strftime("%A").to_string(),
            date.strftime("%e %B %Y")
        ),
        Language::Chinese => format!(
            "{}    {}",
            super::calendar::weekday(language, date),
            date.strftime("%Y年%-m月%-d日")
        ),
    };
    if week {
        heading.push_str(&format!(" W{}", date.strftime("%V")));
    }
    heading
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_date_heading_uses_real_iso_week_and_localized_dates() {
        let monday = Date::new(2020, 8, 3).unwrap();
        assert_eq!(
            date_heading(monday, Language::English, true),
            "Monday        3 August 2020 W32"
        );
        assert_eq!(
            date_heading(monday, Language::Chinese, true),
            "周一    2020年8月3日 W32"
        );
        assert!(!date_heading(monday, Language::English, false).contains("W32"));
    }
}
