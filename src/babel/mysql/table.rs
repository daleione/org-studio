use std::time::Duration;
use unicode_width::UnicodeWidthStr;

#[derive(Debug)]
pub(crate) struct QueryTable {
    pub(crate) columns: Vec<String>,
    pub(super) numeric_columns: Vec<bool>,
    pub(crate) rows: Vec<Vec<Option<String>>>,
    pub(crate) elapsed: Duration,
    pub(super) raw_bytes: usize,
}
impl QueryTable {
    pub(super) fn new(columns: Vec<String>) -> Self {
        Self {
            numeric_columns: vec![false; columns.len()],
            columns,
            rows: Vec::new(),
            elapsed: Duration::ZERO,
            raw_bytes: 0,
        }
    }
    pub(crate) fn serialize(
        &self,
        names: bool,
        newline: &str,
        max_bytes: usize,
    ) -> Result<String, String> {
        let rows = self
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|v| escape_cell(v.as_deref().unwrap_or("NULL")))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let columns = self
            .columns
            .iter()
            .map(|s| escape_cell(s))
            .collect::<Vec<_>>();
        let mut widths = vec![1; columns.len()];
        for row in rows.iter().chain(names.then_some(&columns)) {
            for (index, cell) in row.iter().enumerate() {
                if let Some(width) = widths.get_mut(index) {
                    *width = (*width).max(cell.width());
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
                    let numeric =
                        !header && self.numeric_columns.get(index).copied().unwrap_or(false);
                    let pad = width.saturating_sub(cell.width());
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
                return Err("MySQL table exceeds the serialized result size limit; previous results were preserved".into());
            }
            Ok(())
        };
        if names && !columns.is_empty() {
            emit(&columns, false, true)?;
            emit(&[], true, true)?;
        }
        for row in &rows {
            emit(row, false, false)?;
        }
        Ok(output)
    }
}
fn escape_cell(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_empty_results_and_exact_values() {
        let mut table = QueryTable::new(vec!["id".into(), "value".into()]);
        assert_eq!(
            table.serialize(true, "\n", 1000).unwrap().lines().count(),
            2
        );
        table.rows.push(vec![
            Some("00123".into()),
            Some("18446744073709551615".into()),
        ]);
        table.rows.push(vec![None, Some("a|b\n中文".into())]);
        let result = table.serialize(true, "\r\n", 1000).unwrap();
        assert!(result.contains("00123"));
        assert!(result.contains("18446744073709551615"));
        assert!(result.contains("a\\vert{}b↵中文"));
        assert_eq!(result.lines().count(), 4);
        assert!(table.serialize(true, "\n", 10).is_err());
    }
    #[test]
    fn plain_identifiers_and_punctuation_are_not_escaped() {
        for text in [
            "org_studio_demo",
            "customer__id",
            "客户_编号",
            r"C:\Users\demo",
            r#"{"message": "中文", "enabled": true}"#,
            "[华东, 华南]",
            "[plain text]",
            "price = 3 + 2 / 5",
            "a_b_c < 10",
        ] {
            assert_eq!(escape_cell(text), text);
        }
    }

    #[test]
    fn result_table_entities_display_original_data_without_extra_columns() {
        let samples = [
            "包含竖线 a|b",
            "[[file:demo][链接形态]] *粗体* _下划线_ =原样= ~代码~ {{{macro}}}",
            " leading and trailing ",
            r"C:\Users\demo",
            r#"{"message":"中文"}"#,
            r"\alpha \vert{}",
        ];
        let mut table = QueryTable::new(vec!["value".into()]);
        table.rows = samples
            .iter()
            .map(|sample| vec![Some((*sample).into())])
            .collect();
        let result = table.serialize(true, "\n", 10_000).unwrap();
        for (row, sample) in result.lines().skip(2).zip(samples) {
            let parsed =
                crate::document::table::parse_line(row, crate::document::DocumentFormat::Org);
            assert_eq!(parsed.cells.len(), 1);
            let raw = &row[parsed.cells[0].text_range.clone()];
            let visible = crate::org_syntax::inline::EntityText::new(raw, None);
            assert_eq!(
                visible.text.replace('\u{200c}', "").replace('\u{a0}', " "),
                sample
            );
        }
    }

    #[test]
    fn combinations_of_inline_syntax_remain_literal_table_data() {
        let parts = [
            "a",
            " ",
            "中",
            "*",
            "_",
            "/",
            "+",
            "=",
            "~",
            "$",
            "\\alpha{}",
            "[[",
            "]]",
            "{{{",
            "}}}",
            "|",
            "{",
            "}",
            "\\",
            "[fn:x]",
            "<2026-10-21 Wed>",
        ];
        for a in parts {
            for b in parts {
                for c in parts {
                    let value = format!("{a}{b}{c}");
                    let encoded = escape_cell(&value);
                    let parsed = crate::org_syntax::inline::parse(&encoded);
                    assert!(
                        parsed
                            .spans
                            .iter()
                            .all(|span| span.kind == crate::org_syntax::inline::InlineKind::Entity),
                        "active syntax in {value:?}: {encoded:?}"
                    );
                    assert_eq!(
                        parsed.text.replace('\u{200c}', "").replace('\u{a0}', " "),
                        value,
                        "{encoded}"
                    );
                }
            }
        }
    }
}
