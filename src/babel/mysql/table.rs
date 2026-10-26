use std::time::Duration;

#[cfg(test)]
use super::super::result::escape_cell;

#[derive(Debug)]
pub(crate) struct QueryTable {
    pub(crate) columns: Vec<String>,
    pub(super) column_types: Vec<String>,
    pub(super) numeric_columns: Vec<bool>,
    pub(crate) rows: Vec<Vec<Option<String>>>,
    pub(crate) elapsed: Duration,
    pub(super) raw_bytes: usize,
}
impl QueryTable {
    pub(super) fn new(columns: Vec<String>) -> Self {
        Self {
            column_types: vec![String::new(); columns.len()],
            numeric_columns: vec![false; columns.len()],
            columns,
            rows: Vec::new(),
            elapsed: Duration::ZERO,
            raw_bytes: 0,
        }
    }
    pub(in crate::babel) fn data(&self) -> super::super::value::Data {
        use super::super::value::{Data, Value};
        let rows = self
            .rows
            .iter()
            .map(|row| {
                Value::List(
                    row.iter()
                        .enumerate()
                        .map(|(i, cell)| {
                            let Some(text) = cell else {
                                return Value::Nil;
                            };
                            let kind = self
                                .column_types
                                .get(i)
                                .map(String::as_str)
                                .unwrap_or("")
                                .split_whitespace()
                                .next()
                                .unwrap_or("");
                            match kind {
                                "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => {
                                    Value::Integer(text.clone())
                                }
                                "DECIMAL" => Value::Decimal(text.clone()),
                                "FLOAT" | "DOUBLE" => text
                                    .parse()
                                    .map(Value::Float)
                                    .unwrap_or_else(|_| Value::Text(text.clone())),
                                _ => Value::Text(text.clone()),
                            }
                        })
                        .collect(),
                )
            })
            .collect();
        Data {
            value: Value::List(rows),
            columns: Some(self.columns.iter().cloned().map(Value::Text).collect()),
            row_names: Vec::new(),
        }
    }
    pub(crate) fn serialize(
        &self,
        names: bool,
        newline: &str,
        max_bytes: usize,
    ) -> Result<String, String> {
        super::super::result::serialize_table(
            &self.columns,
            &self.rows,
            &self.numeric_columns,
            names,
            newline,
            max_bytes,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;
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
            "《示例：A12》 ",
            "用于测试列宽的较长中文示例文本",
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
        let width = result.lines().next().unwrap().width();
        for row in result.lines() {
            assert_eq!(
                crate::org_syntax::inline::parse(row).text.width(),
                width,
                "serialized table must align as displayed: {row}"
            );
        }
        for (row, sample) in result.lines().skip(2).zip(samples) {
            let parsed =
                crate::document::table::parse_line(row, crate::document::DocumentFormat::Org);
            assert_eq!(parsed.cells.len(), 1);
            let raw = &row[parsed.cells[0].text_range.clone()];
            let visible = crate::org_syntax::inline::parse(raw);
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
