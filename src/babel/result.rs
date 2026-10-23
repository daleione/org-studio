use super::{
    headers::Headers,
    value::{Data, Value},
};
use crate::document::{DocumentFormat, table::text_width};

pub(super) fn payload(data: &Data, headers: &Headers, newline: &str) -> Result<String, String> {
    let scalar = headers.values(":results").iter().any(|v| v == "scalar");
    if let Value::List(values) = &data.value
        && !scalar
    {
        let mut rows = Vec::new();
        for value in values {
            match value {
                Value::List(cells) => rows.push(cells.clone()),
                Value::Hline => rows.push(Vec::new()),
                value => rows.push(vec![value.clone()]),
            }
        }
        if rows.iter().flatten().any(|v| matches!(v, Value::List(_))) {
            return Err("Babel results support scalars and one- or two-dimensional lists".into());
        }
        if !data.row_names.is_empty() && headers.get(":rownames") == Some("yes") {
            for (row, name) in rows
                .iter_mut()
                .filter(|r| !r.is_empty())
                .zip(&data.row_names)
            {
                row.insert(0, name.clone());
            }
        }
        let columns = if headers.get(":colnames") == Some("no") {
            None
        } else {
            data.columns.as_ref()
        };
        let width = rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(columns.map_or(0, Vec::len));
        if rows.iter().any(|row| !row.is_empty() && row.len() != width) {
            return Err("Babel table results must be rectangular".into());
        }
        let numeric = (0..width)
            .map(|i| {
                rows.iter()
                    .filter(|r| !r.is_empty())
                    .all(|row| row[i].number() || row[i] == Value::Nil)
            })
            .collect::<Vec<_>>();
        let rows = rows
            .iter()
            .map(|r| r.iter().map(|v| Some(v.display())).collect())
            .collect::<Vec<_>>();
        let columns = columns
            .map(|r| r.iter().map(Value::display).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![String::new(); width]);
        return serialize_table(
            &columns,
            &rows,
            &numeric,
            data.columns.is_some() && headers.get(":colnames") != Some("no"),
            newline,
            1024 * 1024,
        );
    }
    if matches!(data.value, Value::List(_)) {
        return Err(":results scalar requires a scalar return value".into());
    }
    Ok(fixed_width(&data.value.display(), newline))
}

pub(super) fn fixed_width(text: &str, newline: &str) -> String {
    let mut output = String::new();
    for line in text.lines() {
        output.push(':');
        if !line.is_empty() {
            output.push(' ');
            output.push_str(line);
        }
        output.push_str(newline);
    }
    if output.is_empty() {
        output.push(':');
        output.push_str(newline);
    }
    output
}

pub(super) fn serialize_table(
    columns: &[String],
    rows: &[Vec<Option<String>>],
    numeric_columns: &[bool],
    names: bool,
    newline: &str,
    max_bytes: usize,
) -> Result<String, String> {
    let rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|v| escape_cell(v.as_deref().unwrap_or("NULL")))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let columns = columns.iter().map(|s| escape_cell(s)).collect::<Vec<_>>();
    let mut widths = vec![1; columns.len()];
    for row in rows.iter().chain(names.then_some(&columns)) {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(text_width(cell, DocumentFormat::Org));
            }
        }
    }
    let mut output = String::new();
    let mut emit = |cells: &[String], rule: bool, header: bool| -> Result<(), String> {
        output.push('|');
        for (index, width) in widths.iter().enumerate() {
            if rule {
                output.push_str(&"-".repeat(width + 2));
                output.push(if index + 1 == widths.len() { '|' } else { '+' });
            } else {
                let cell = cells.get(index).map(String::as_str).unwrap_or("");
                output.push(' ');
                let numeric = !header && numeric_columns.get(index).copied().unwrap_or(false);
                let pad = width.saturating_sub(text_width(cell, DocumentFormat::Org));
                if numeric {
                    output.push_str(&" ".repeat(pad));
                }
                output.push_str(cell);
                if !numeric {
                    output.push_str(&" ".repeat(pad));
                }
                output.push_str(" |");
            }
        }
        output.push_str(newline);
        if output.len() > max_bytes {
            return Err("Babel table exceeds the serialized result size limit; previous results were preserved".into());
        }
        Ok(())
    };
    if names && !columns.is_empty() {
        emit(&columns, false, true)?;
        emit(&[], true, true)?;
    }
    for row in &rows {
        emit(row, row.is_empty(), false)?;
    }
    Ok(output)
}
pub(super) fn escape_cell(text: &str) -> String {
    // Escape table structure and actual inline syntax, not every punctuation
    // character. The shared Org parser decides which ranges need protection.
    let parsed = crate::org_syntax::inline::parse(text);
    let mut syntax = parsed
        .spans
        .iter()
        .filter(|span| {
            span.kind != crate::org_syntax::inline::InlineKind::Entity
                || text[span.source.clone()] != parsed.text[span.range.clone()]
        })
        .map(|span| span.source.clone())
        .collect::<Vec<_>>();
    syntax.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    let mut syntax_index = 0;
    let mut out = String::new();
    let trimmed_start = text.len() - text.trim_start_matches(' ').len();
    let trimmed_end = text.trim_end_matches(' ').len();
    for (index, c) in text.char_indices() {
        while syntax
            .get(syntax_index)
            .is_some_and(|range| range.end <= index)
        {
            syntax_index += 1;
        }
        let inline = syntax
            .get(syntax_index)
            .is_some_and(|range| range.contains(&index));
        match c {
            '{' if text[index..].starts_with("{{{") => {
                out.push(c);
                out.push_str("\\zwnj{}");
            }
            '\\' | '[' | ']' | '{' | '}' if inline => {
                out.push(c);
                out.push_str("\\zwnj{}");
            }
            ' ' if index < trimmed_start || index >= trimmed_end => out.push_str("\\nbsp{}"),
            '|' => out.push_str("\\vert{}"),
            '\n' => out.push('↵'),
            '\r' => out.push('␍'),
            '\t' => out.push('⇥'),
            '*' if inline => out.push_str("\\ast{}"),
            '_' if inline => out.push_str("\\under{}"),
            '/' if inline => out.push_str("\\slash{}"),
            '=' if inline => out.push_str("\\equal{}"),
            '~' if inline => out.push_str("\\tilde{}"),
            '+' if inline => out.push_str("\\plus{}"),
            '<' if inline => out.push_str("\\lt{}"),
            '>' if inline => out.push_str("\\gt{}"),
            '^' if inline => out.push_str("\\asciicirc{}"),
            // Org can recognize math starting at the second dollar in $$.
            // Escape every dollar so protecting one fragment cannot expose another.
            '$' => out.push_str("\\dollar{}"),
            c if c.is_control() => out.push('�'),
            c => out.push(c),
        }
    }
    out
}
