use crate::{
    agenda::{AgendaDateKind, AgendaProjectedLine, AgendaResultSnapshot, project_agenda_text},
    i18n::Language,
};
use gpui::{FontWeight, HighlightStyle, rgb};
use jiff::civil::Date;

pub(crate) struct AgendaTextBuffer {
    pub(crate) text: String,
    pub(crate) highlights: Vec<crate::editor::LineHighlights>,
    pub(crate) targets: Vec<Option<crate::agenda::AgendaPlacementRef>>,
}

pub(crate) fn agenda_text(
    result: &AgendaResultSnapshot,
    language: Language,
    window: Option<(Date, Date)>,
) -> AgendaTextBuffer {
    let theme = crate::theme::current_theme();
    let today = result.query.today;
    let lines = project_agenda_text(
        result,
        language.text("agenda.scheduled"),
        language.text("agenda.deadline"),
        language.text("agenda.unscheduled"),
        window,
        |date, week| date_heading(date, language, week),
    );
    let mut content = AgendaTextBuffer {
        text: String::new(),
        highlights: Vec::new(),
        targets: Vec::new(),
    };
    for projected in lines {
        match projected {
            AgendaProjectedLine::Header { date, text: header } => {
                let weekend = date.is_some_and(|date| date.weekday().to_monday_zero_offset() >= 5);
                content.highlights.push(vec![(
                    0..header.len(),
                    HighlightStyle {
                        color: Some(
                            rgb(if date == Some(today) {
                                super::super::style::HEADER_TODAY()
                            } else if weekend {
                                super::super::style::HEADER_WEEKEND()
                            } else {
                                super::super::style::HEADER_DATE()
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
            }
            AgendaProjectedLine::Entry {
                entry: index,
                placement: placement_key,
                line,
            } => {
                let entry = &result.entries[index];
                let row = &entry.row;
                let deadline = entry.occurrence.as_ref().is_some_and(|occurrence| {
                    occurrence.timestamp.kind == AgendaDateKind::Deadline
                });
                let overdue = deadline
                    && entry
                        .occurrence
                        .as_ref()
                        .is_some_and(|occurrence| occurrence.start_date < today);
                let mut highlights = vec![
                    (
                        0..line.text.len(),
                        HighlightStyle {
                            color: Some(
                                rgb(if overdue {
                                    theme.error
                                } else if deadline {
                                    theme.warning
                                } else {
                                    theme.foreground
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
                            color: Some(rgb(theme.success).into()),
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
                                    Some('A') => theme.error,
                                    Some('B') => theme.warning,
                                    _ => theme.success,
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
                            color: Some(rgb(theme.warning).into()),
                            ..Default::default()
                        },
                    ));
                }
                content.text.push_str(&line.text);
                content.text.push('\n');
                content.highlights.push(highlights);
                content.targets.push(result.placement_ref(placement_key));
            }
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
