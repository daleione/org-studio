use unicode_width::UnicodeWidthStr;

use super::AgendaResultSnapshot;

pub(crate) fn format_agenda_text(result: &AgendaResultSnapshot) -> String {
    let category_width = result
        .rows
        .iter()
        .filter_map(|row| row.category.as_deref())
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0);
    let mut output = String::new();
    for row in result.rows.iter() {
        let category = row.category.as_deref().unwrap_or("");
        let padding = " ".repeat(category_width.saturating_sub(UnicodeWidthStr::width(category)));
        let date = row
            .date
            .map(|date| date.to_string())
            .unwrap_or_else(|| "          ".into());
        let time = row
            .time
            .map(|time| time.strftime("%H:%M").to_string())
            .unwrap_or_else(|| "     ".into());
        output.push_str(&format!(
            "{date} {time} {category}{padding}: {:<8} {}\n",
            row.todo, row.title
        ));
    }
    output
}
