use super::*;
use crate::{
    command::TableScope,
    document::{Selection, TextEdit},
};

#[derive(Debug)]
pub(crate) struct TableAlignmentPlan {
    pub edits: Vec<TextEdit>,
    pub tables: usize,
    pub changed: usize,
    pub skipped: usize,
    format: DocumentFormat,
    rows: Vec<(ByteRange, String, String)>,
}

impl TableAlignmentPlan {
    pub(crate) fn map_offset(&self, offset: ByteOffset) -> ByteOffset {
        let mut shift = 0i128;
        for (range, old, new) in &self.rows {
            if offset < range.start {
                break;
            }
            if offset <= range.end {
                let local = (offset.0 - range.start.0) as usize;
                let mapped = if local == 0 {
                    0
                } else {
                    let (column, character) = table_position_at(old, local, self.format);
                    let cells = source_table::parse_line(new, self.format);
                    cells.cells.get(column).map_or(new.len(), |cell| {
                        cell.text_range.start
                            + new[cell.text_range.clone()]
                                .char_indices()
                                .nth(character)
                                .map_or(cell.text_range.len(), |(byte, _)| byte)
                    })
                };
                return ByteOffset((range.start.0 as i128 + shift + mapped as i128) as u64);
            }
            shift += new.len() as i128 - old.len() as i128;
        }
        ByteOffset((offset.0 as i128 + shift).max(0) as u64)
    }

    pub(crate) fn map_selection(&self, selection: Selection) -> Selection {
        Selection::new(
            self.map_offset(selection.anchor()),
            self.map_offset(selection.head()),
        )
    }
}

/// Parse once on the worker. Only actual syntax tables participate, including folded tables.
pub(crate) fn plan_table_alignment(
    path: &Path,
    snapshot: &DocumentSnapshot,
    scope: TableScope,
    selection: Selection,
) -> Option<TableAlignmentPlan> {
    let format = DocumentFormat::detect(path)?;
    let mut tables = Vec::<Range<u64>>::new();
    let mut nested_tables = std::collections::HashSet::new();
    match format {
        DocumentFormat::Org => {
            for node in parse(snapshot).nodes() {
                if !matches!(node.kind, BlockKind::TableRow) {
                    continue;
                }
                let line = snapshot.line_index_at(node.source.start).ok()?.0;
                if let Some(previous) = tables.last_mut().filter(|range| range.end == line) {
                    previous.end += 1;
                } else {
                    tables.push(line..line + 1);
                }
            }
        }
        DocumentFormat::Markdown => {
            let text = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
            let parser =
                pulldown_cmark::Parser::new_ext(&text, pulldown_cmark::Options::ENABLE_TABLES);
            for (event, range) in parser.into_offset_iter() {
                if !matches!(
                    event,
                    pulldown_cmark::Event::Start(pulldown_cmark::Tag::Table(_))
                ) {
                    continue;
                }
                let start = snapshot
                    .line_index_at(ByteOffset(range.start as u64))
                    .ok()?
                    .0;
                // Table event ranges include the final newline. Its preceding byte is sufficient
                // for line lookup even with CRLF; it is an ASCII character boundary.
                let end = snapshot
                    .line_index_at(ByteOffset(
                        text[..range.end].char_indices().next_back()?.0 as u64,
                    ))
                    .ok()?
                    .0
                    + 1;
                let line_start =
                    snapshot.line_content_range(LineIndex(start)).ok()?.start.0 as usize;
                if !text[line_start..range.start].trim().is_empty() {
                    nested_tables.insert(start);
                }
                tables.push(start..end);
            }
        }
    }
    let mut plan = TableAlignmentPlan {
        edits: vec![],
        tables: 0,
        changed: 0,
        skipped: 0,
        format,
        rows: vec![],
    };
    for lines in tables {
        let start = snapshot
            .line_content_range(LineIndex(lines.start))
            .ok()?
            .start;
        let end = snapshot
            .line_content_range(LineIndex(lines.end - 1))
            .ok()?
            .end;
        let included = match scope {
            TableScope::Document => true,
            TableScope::Current => selection.head() >= start && selection.head() <= end,
            TableScope::Selection => {
                !selection.is_empty()
                    && selection.range().start < end
                    && selection.range().end > start
            }
        };
        if !included {
            continue;
        }
        plan.tables += 1;
        let rows = lines
            .clone()
            .map(|line| {
                let range = snapshot.line_content_range(LineIndex(line)).ok()?;
                Some((range, snapshot.copy_range(range)))
            })
            .collect::<Option<Vec<_>>>()?;
        // Nested Markdown quotes/list markers are not cell data. Skip structures the existing
        // formatter cannot preserve instead of silently replacing their container prefixes.
        if nested_tables.contains(&lines.start)
            || rows.iter().any(|(_, text)| {
                text.trim_start().starts_with('>')
                    || text.trim_start().starts_with("- ")
                    || text.trim_start().starts_with("* ")
            })
        {
            plan.skipped += 1;
            continue;
        }
        let parsed = rows
            .iter()
            .map(|(_, text)| parse_source_row(text, format))
            .collect::<Vec<_>>();
        let columns = parsed.iter().map(|row| row.cells.len()).max().unwrap_or(0);
        if columns == 0 {
            plan.skipped += 1;
            continue;
        }
        let widths = table_widths(&parsed, columns, format);
        let before = plan.edits.len();
        for ((range, old), parsed) in rows.into_iter().zip(parsed) {
            let (new, _) = aligned_row(&parsed, &widths, None, format);
            if old == new {
                continue;
            }
            // Keep unchanged line prefixes/suffixes out of the edit so viewport anchors map
            // through the transaction. Newlines are never rewritten, including mixed CRLF/LF.
            let mut prefix = old
                .bytes()
                .zip(new.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
                prefix -= 1;
            }
            let mut suffix = old[prefix..]
                .bytes()
                .rev()
                .zip(new[prefix..].bytes().rev())
                .take_while(|(a, b)| a == b)
                .count();
            while !old.is_char_boundary(old.len() - suffix)
                || !new.is_char_boundary(new.len() - suffix)
            {
                suffix -= 1;
            }
            plan.edits.push(TextEdit::new(
                ByteRange::new(range.start.0 + prefix as u64, range.end.0 - suffix as u64),
                &new[prefix..new.len() - suffix],
            ));
            plan.rows.push((range, old, new));
        }
        plan.changed += usize::from(plan.edits.len() > before);
    }
    Some(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocumentBuffer, EditTransaction};

    fn align(
        text: &str,
        path: &str,
        scope: TableScope,
        selection: Selection,
    ) -> (String, TableAlignmentPlan) {
        let mut buffer = DocumentBuffer::from_utf8(text.as_bytes().to_vec()).unwrap();
        let snapshot = buffer.snapshot();
        let plan = plan_table_alignment(Path::new(path), &snapshot, scope, selection).unwrap();
        if !plan.edits.is_empty() {
            buffer
                .commit(EditTransaction::new(
                    snapshot.revision(),
                    plan.edits.clone(),
                ))
                .unwrap();
        }
        let result = buffer.snapshot();
        (
            result.copy_range(ByteRange::new(0, result.len_bytes())),
            plan,
        )
    }

    #[test]
    fn org_batch_formats_multiple_tables_and_preserves_blocks_and_cell_content() {
        let text = "* Folded\n|名字|value|\n|---+---|\n|中文|a\\|b|\n\n#+begin_src text\n|not|a table|\n#+end_src\n\n#+begin_example\n|not|a table either|\n#+end_example\n\n* Another\n|x|`a\tb`|\n|long name|z|\n";
        let (formatted, plan) = align(
            text,
            "notes.org",
            TableScope::Document,
            Selection::default(),
        );
        assert_eq!((plan.tables, plan.changed, plan.skipped), (2, 2, 0));
        assert!(
            formatted.contains("| 名字 | value |\n|------+-------|\n| 中文 | a\\|b  |"),
            "{formatted}"
        );
        assert!(formatted.contains("#+begin_src text\n|not|a table|\n#+end_src"));
        assert!(formatted.contains("#+begin_example\n|not|a table either|\n#+end_example"));
        assert!(formatted.contains("`a\tb`"));
        let (again, next) = align(
            &formatted,
            "notes.org",
            TableScope::Document,
            Selection::default(),
        );
        assert!(next.edits.is_empty());
        assert_eq!(formatted, again);
    }

    #[test]
    fn markdown_batch_preserves_crlf_alignment_and_ignores_code() {
        let text = "# Tables\r\n\r\nname | value\r\n:--- | ---:\r\n中文 | a\\|b\r\n\r\n```md\r\n|x|y|\r\n|---|---|\r\n```\r\n\r\n    |x|y|\r\n    |---|---|\r\n\r\nordinary | paragraph\r\n";
        let (formatted, plan) = align(text, "notes.md", TableScope::Document, Selection::default());
        assert_eq!((plan.tables, plan.changed), (1, 1));
        assert!(formatted.contains("| :--- | ----: |\r\n"), "{formatted}");
        assert!(formatted.contains("| 中文 | a\\|b  |\r\n"));
        assert!(formatted.ends_with("```md\r\n|x|y|\r\n|---|---|\r\n```\r\n\r\n    |x|y|\r\n    |---|---|\r\n\r\nordinary | paragraph\r\n"));
        assert_eq!(
            formatted.matches('\r').count(),
            formatted.matches('\n').count()
        );
        assert!(
            align(
                &formatted,
                "notes.md",
                TableScope::Document,
                Selection::default()
            )
            .1
            .edits
            .is_empty()
        );
    }

    #[test]
    fn nested_markdown_tables_are_reported_without_rewriting_containers() {
        let text = "> |a|b|\n> |---|---|\n> |x|long|\n\n1. |a|b|\n   |---|---|\n   |x|long|\n";
        let (formatted, plan) = align(text, "notes.md", TableScope::Document, Selection::default());
        assert_eq!(formatted, text);
        assert_eq!((plan.tables, plan.skipped), (2, 2));
    }

    #[test]
    fn scope_and_caret_mapping_follow_cells_and_trailing_text() {
        let text = "|a|中文|\n|length|z|\n\n|second|b|\n|x|longer|\n\n尾部";
        let caret = ByteOffset(text.find('文').unwrap() as u64);
        let second = text.find("second").unwrap() as u64;
        let selection = Selection::new(caret, ByteOffset(second + 2));
        let (formatted, plan) = align(
            text,
            "notes.org",
            TableScope::Current,
            Selection::caret(caret),
        );
        assert_eq!(plan.changed, 1);
        assert!(formatted.contains("|second|b|\n|x|longer|"));
        assert_eq!(&formatted[plan.map_offset(caret).0 as usize..][..3], "文");
        let mapped = plan.map_selection(selection);
        assert_eq!(&formatted[mapped.head().0 as usize..][..4], "cond");
        assert_eq!(
            plan.map_offset(ByteOffset(text.len() as u64)).0,
            formatted.len() as u64
        );
        assert_eq!(
            align(text, "notes.org", TableScope::Selection, selection)
                .1
                .changed,
            2
        );
        assert!(
            align(
                text,
                "notes.org",
                TableScope::Selection,
                Selection::default()
            )
            .1
            .edits
            .is_empty()
        );
    }
}
