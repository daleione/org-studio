use super::*;
use crate::{command::TableEdit, document::Selection};

/// One parsed table is used for availability checks and the eventual edit.
pub(in crate::editor) struct EditableTable {
    rows: Vec<ParsedRow>,
    columns: usize,
    row: usize,
    column: usize,
    character: usize,
    start_line: u64,
    range: ByteRange,
    format: DocumentFormat,
    formula_lines: Vec<String>,
}

impl EditableTable {
    pub(in crate::editor) fn at(
        snapshot: &DocumentSnapshot,
        context: &EditorCommandContext,
    ) -> Option<Self> {
        let EditorCommandKind::TableCell { column, character } = context.kind else {
            return None;
        };
        let format = context.format?;
        let start = table_start(snapshot, context.line.0, format)?;
        let mut end = start;
        let mut rows = Vec::new();
        while end < snapshot.len_lines() {
            let Some(text) = table_line(snapshot, end, format) else {
                break;
            };
            rows.push(parse_source_row(&text, format));
            end += 1;
        }
        let columns = rows.iter().map(|row| row.cells.len()).max()?;
        if columns == 0 {
            return None;
        }
        for row in &mut rows {
            row.cells.resize(columns, String::new());
            row.separator_alignments.resize(columns, (false, false));
        }
        let mut formula_lines = Vec::new();
        let mut formula_end = end;
        if format == DocumentFormat::Org {
            while let Ok(range) = snapshot.line_content_range(LineIndex(formula_end)) {
                let line = snapshot.copy_range(range);
                if !line
                    .trim_start()
                    .get(..8)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+TBLFM:"))
                {
                    break;
                }
                formula_lines.push(line);
                formula_end += 1;
            }
        }
        Some(Self {
            rows,
            columns,
            row: (context.line.0 - start) as usize,
            column: column.min(columns - 1),
            character,
            start_line: start,
            range: ByteRange::new(
                snapshot.line_content_range(LineIndex(start)).ok()?.start.0,
                snapshot
                    .line_content_range(LineIndex(formula_end.saturating_sub(1)))
                    .ok()?
                    .end
                    .0,
            ),
            format,
            formula_lines,
        })
    }

    pub(in crate::editor) fn available(&self, edit: TableEdit) -> bool {
        let markdown = self.format == DocumentFormat::Markdown;
        let position_valid = match edit {
            TableEdit::InsertRow => !markdown || self.row > 1,
            TableEdit::InsertRowBelow => !markdown || self.row > 0,
            TableEdit::KillRow => !markdown || self.row > 1,
            TableEdit::MoveRowUp => self.row > if markdown { 2 } else { 0 },
            TableEdit::MoveRowDown => self.row + 1 < self.rows.len() && (!markdown || self.row > 1),
            TableEdit::DeleteColumn => self.columns > 1,
            TableEdit::MoveColumnLeft => self.column > 0,
            TableEdit::MoveColumnRight => self.column + 1 < self.columns,
            TableEdit::InsertHline | TableEdit::InsertHlineAbove | TableEdit::HlineAndMove => {
                !markdown
            }
            TableEdit::Sort { .. } => !self.rows[self.row].separator && (!markdown || self.row > 1),
            TableEdit::InsertColumn => true,
        };
        position_valid && (self.formula_lines.is_empty() || self.rewritten_formulas(edit).is_some())
    }

    fn data_row_at(&self, index: usize) -> usize {
        self.rows[..index]
            .iter()
            .filter(|row| !row.separator)
            .count()
            + 1
    }

    fn row_marker(&self, index: usize) -> Option<&str> {
        self.rows.get(index)?.cells.first().map(|cell| cell.trim())
    }

    fn rewritten_formulas(&self, edit: TableEdit) -> Option<Vec<String>> {
        use super::formula::{AxisEdit, adapt_formula_lines};

        let insert_at = match edit {
            TableEdit::InsertRow => Some(self.row),
            TableEdit::InsertRowBelow => Some(self.row + 1),
            _ => None,
        };
        if insert_at.is_some_and(|index| {
            self.row_marker(index) == Some("^")
                || index > 0 && self.row_marker(index - 1) == Some("_")
        }) {
            return None;
        }
        if edit == TableEdit::KillRow
            && (matches!(self.row_marker(self.row), Some("!" | "^" | "_" | "$"))
                || self.row_marker(self.row + 1) == Some("^")
                || self.row > 0 && self.row_marker(self.row - 1) == Some("_"))
        {
            return None;
        }
        let data_row = self.data_row_at(self.row);
        let (row, column) = match edit {
            TableEdit::InsertRow => (Some(AxisEdit::Insert(data_row)), None),
            TableEdit::InsertRowBelow => {
                (Some(AxisEdit::Insert(self.data_row_at(self.row + 1))), None)
            }
            TableEdit::KillRow
                if self.rows.iter().filter(|row| !row.separator).count() > 1
                    && !self.rows[self.row].separator =>
            {
                (Some(AxisEdit::Delete(data_row)), None)
            }
            TableEdit::MoveRowUp | TableEdit::MoveRowDown => {
                let next = if edit == TableEdit::MoveRowUp {
                    self.row - 1
                } else {
                    self.row + 1
                };
                if self.rows[self.row].separator || self.rows[next].separator {
                    return None;
                }
                if matches!(self.row_marker(self.row), Some("!" | "^" | "_" | "$"))
                    || matches!(self.row_marker(next), Some("!" | "^" | "_" | "$"))
                    || self.row_marker(self.row.max(next) + 1) == Some("^")
                    || self.row.min(next) > 0
                        && self.row_marker(self.row.min(next) - 1) == Some("_")
                {
                    return None;
                }
                (Some(AxisEdit::Swap(data_row, self.data_row_at(next))), None)
            }
            TableEdit::InsertColumn => (None, Some(AxisEdit::Insert(self.column + 1))),
            TableEdit::DeleteColumn => (None, Some(AxisEdit::Delete(self.column + 1))),
            TableEdit::MoveColumnLeft | TableEdit::MoveColumnRight => {
                let next = if edit == TableEdit::MoveColumnLeft {
                    self.column - 1
                } else {
                    self.column + 1
                };
                (None, Some(AxisEdit::Swap(self.column + 1, next + 1)))
            }
            TableEdit::InsertHline
            | TableEdit::InsertHlineAbove
            | TableEdit::HlineAndMove
            | TableEdit::Sort { .. }
            | TableEdit::KillRow => return None,
        };
        // The first column holds Org's #, *, !, ^, _, and $ row markers.
        let changes_first_column =
            matches!(edit, TableEdit::InsertColumn | TableEdit::DeleteColumn) && self.column == 0
                || edit == TableEdit::MoveColumnRight && self.column == 0
                || edit == TableEdit::MoveColumnLeft && self.column == 1;
        if changes_first_column
            && self.rows.iter().any(|row| {
                row.cells.first().is_some_and(|cell| {
                    matches!(cell.trim(), "#" | "*" | "!" | "^" | "_" | "$" | "/")
                })
            })
        {
            return None;
        }
        if edit == TableEdit::DeleteColumn
            && self.rows.iter().any(|row| {
                matches!(
                    row.cells.first().map(|cell| cell.trim()),
                    Some("!" | "^" | "_" | "$")
                ) && !row.cells[self.column].trim().is_empty()
            })
        {
            return None;
        }
        adapt_formula_lines(&self.formula_lines, row, column)
    }

    fn blank_row(&self, separator: bool) -> ParsedRow {
        ParsedRow {
            indent: self.rows[self.row].indent.clone(),
            cells: vec![String::new(); self.columns],
            separator,
            separator_alignments: vec![(false, false); self.columns],
        }
    }

    pub(in crate::editor) fn edit(
        mut self,
        edit: TableEdit,
        snapshot: &DocumentSnapshot,
        selection: Selection,
        newline: &str,
    ) -> Result<TableAlignment, &'static str> {
        if !self.available(edit) {
            return Err("当前表格位置不支持此操作 / Operation unavailable at this table position");
        }
        let formulas = if self.formula_lines.is_empty() {
            Vec::new()
        } else {
            self.rewritten_formulas(edit)
                .ok_or("Cannot safely update table formula references")?
        };
        match edit {
            TableEdit::InsertRow | TableEdit::InsertRowBelow => {
                let target = self.row + usize::from(edit == TableEdit::InsertRowBelow);
                self.rows.insert(target, self.blank_row(false));
                self.row = target;
                self.character = 0;
            }
            TableEdit::KillRow => {
                self.rows.remove(self.row);
                self.row = self.row.min(self.rows.len().saturating_sub(1));
                self.character = 0;
                // Deleting the only row also consumes its newline, without joining neighbours.
                if self.rows.is_empty() {
                    self.range = snapshot
                        .line_range(LineIndex(self.start_line))
                        .map_err(|_| "Invalid table range")?;
                }
            }
            TableEdit::MoveRowUp | TableEdit::MoveRowDown => {
                let target = if edit == TableEdit::MoveRowUp {
                    self.row - 1
                } else {
                    self.row + 1
                };
                self.rows.swap(self.row, target);
                self.row = target;
            }
            TableEdit::InsertColumn => {
                for row in &mut self.rows {
                    row.cells.insert(self.column, String::new());
                    row.separator_alignments.insert(self.column, (false, false));
                }
                self.columns += 1;
                self.character = 0;
            }
            TableEdit::DeleteColumn => {
                for row in &mut self.rows {
                    row.cells.remove(self.column);
                    row.separator_alignments.remove(self.column);
                }
                self.columns -= 1;
                self.column = self.column.min(self.columns - 1);
                self.character = 0;
            }
            TableEdit::MoveColumnLeft | TableEdit::MoveColumnRight => {
                let target = if edit == TableEdit::MoveColumnLeft {
                    self.column - 1
                } else {
                    self.column + 1
                };
                for row in &mut self.rows {
                    row.cells.swap(self.column, target);
                    row.separator_alignments.swap(self.column, target);
                }
                self.column = target;
            }
            TableEdit::InsertHline | TableEdit::InsertHlineAbove | TableEdit::HlineAndMove => {
                let separator = self.row + usize::from(edit != TableEdit::InsertHlineAbove);
                self.rows.insert(separator, self.blank_row(true));
                if edit == TableEdit::InsertHlineAbove {
                    self.row += 1;
                }
                if edit == TableEdit::HlineAndMove {
                    let next = separator + 1;
                    if self.rows.get(next).is_none_or(|row| row.separator) {
                        self.rows.insert(next, self.blank_row(false));
                    }
                    self.row = next;
                    self.character = 0;
                }
            }
            TableEdit::Sort { numeric, reverse } => {
                self.sort(snapshot, selection, numeric, reverse)?
            }
        }
        let widths = table_widths(&self.rows, self.columns, self.format);
        let mut replacement = String::new();
        let mut caret = self.range.start;
        for (index, row) in self.rows.iter().enumerate() {
            if index > 0 {
                replacement.push_str(newline);
            }
            let (text, local) = aligned_row(
                row,
                &widths,
                (index == self.row).then_some((self.column, self.character)),
                self.format,
            );
            if let Some(local) = local {
                caret = ByteOffset(self.range.start.0 + (replacement.len() + local) as u64);
            }
            replacement.push_str(&text);
        }
        for formula in formulas {
            replacement.push_str(newline);
            replacement.push_str(&formula);
        }
        Ok(TableAlignment {
            range: self.range,
            replacement,
            caret,
        })
    }

    fn sort(
        &mut self,
        snapshot: &DocumentSnapshot,
        selection: Selection,
        numeric: bool,
        reverse: bool,
    ) -> Result<(), &'static str> {
        let (start, end, column) = if selection.is_empty() {
            let start = self.rows[..self.row]
                .iter()
                .rposition(|row| row.separator)
                .map_or(0, |i| i + 1);
            let end = self.rows[self.row..]
                .iter()
                .position(|row| row.separator)
                .map_or(self.rows.len(), |i| self.row + i);
            (start, end, self.column)
        } else {
            let range = selection.range();
            let first = snapshot
                .line_index_at(range.start)
                .map_err(|_| "Invalid selection")?
                .0;
            let mut last = snapshot
                .line_index_at(range.end)
                .map_err(|_| "Invalid selection")?
                .0;
            if snapshot
                .line_content_range(LineIndex(last))
                .is_ok_and(|line| line.start == range.end)
            {
                last = last.saturating_sub(1);
            }
            if first < self.start_line || last >= self.start_line + self.rows.len() as u64 {
                return Err("排序选区必须位于同一表格内 / Select rows within one table");
            }
            let mark = selection.anchor();
            let mark_line = snapshot
                .line_index_at(mark)
                .map_err(|_| "Invalid selection")?;
            let mark_range = snapshot
                .line_content_range(mark_line)
                .map_err(|_| "Invalid selection")?;
            let column = table_position_at(
                &snapshot.copy_range(mark_range),
                (mark.0 - mark_range.start.0) as usize,
                self.format,
            )
            .0
            .min(self.columns - 1);
            (
                (first - self.start_line) as usize,
                (last - self.start_line + 1) as usize,
                column,
            )
        };
        if (self.format == DocumentFormat::Markdown && start < 2)
            || self.rows[start..end].iter().any(|row| row.separator)
        {
            return Err("排序选区不能跨越表头或分隔线 / Select data rows within one section");
        }
        let mut order = (start..end)
            .map(|index| {
                let text = &self.rows[index].cells[column];
                let number = if numeric {
                    text.parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .ok_or("数值排序列包含非数值单元格 / Numeric sort requires numbers in every selected row")?
                } else {
                    0.0
                };
                let text = if numeric { String::new() } else { text.to_lowercase() };
                Ok((index, text, number))
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        order.sort_by(|a, b| {
            let order = if numeric {
                a.2.partial_cmp(&b.2).expect("numeric keys are finite")
            } else {
                a.1.cmp(&b.1)
            };
            if reverse { order.reverse() } else { order }
        });
        let mut old = self.rows.drain(start..end).map(Some).collect::<Vec<_>>();
        let current = self.row;
        let sorted = order
            .into_iter()
            .enumerate()
            .map(|(position, (index, _, _))| {
                if index == current {
                    self.row = start + position;
                }
                old[index - start]
                    .take()
                    .expect("each row occurs once in the sort order")
            })
            .collect::<Vec<_>>();
        self.rows.splice(start..start, sorted);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(
        text: &str,
        needle: &str,
        extension: &str,
    ) -> (DocumentSnapshot, EditableTable, Selection) {
        let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
        let selection = Selection::caret(ByteOffset(text.find(needle).unwrap() as u64));
        let context =
            EditorCommandContext::at(Path::new(extension), &snapshot, selection.head()).unwrap();
        let table = EditableTable::at(&snapshot, &context).unwrap();
        (snapshot, table, selection)
    }

    fn apply(text: &str, needle: &str, extension: &str, edit: TableEdit) -> (String, usize) {
        let (snapshot, table, selection) = table(text, needle, extension);
        let change = table
            .edit(
                edit,
                &snapshot,
                selection,
                if text.contains("\r\n") { "\r\n" } else { "\n" },
            )
            .unwrap();
        let mut output = text.to_owned();
        output.replace_range(
            change.range.start.0 as usize..change.range.end.0 as usize,
            &change.replacement,
        );
        (output, change.caret.0 as usize)
    }

    fn cells(text: &str, format: DocumentFormat) -> Vec<Vec<String>> {
        text.lines()
            .filter(|line| source_table::is_table_row(line, format))
            .map(|line| parse_source_row(line, format).cells)
            .collect()
    }

    #[test]
    fn table_columns_preserve_unicode_literals_and_markdown_alignment() {
        for (extension, text, literal) in [
            (
                "shelf.org",
                "| 名称 | 标记 | 数量 |\n|------+------ +------|\n| 松木🌲 | =a|b= | 12 |\n",
                "=a|b=",
            ),
            (
                "shelf.md",
                "| 名称 | 标记 | 数量 |\n| :--- | :---: | ---: |\n| 松木🌲 | `a|b` | 12 |\n",
                "`a|b`",
            ),
        ] {
            let format = DocumentFormat::detect(Path::new(extension)).unwrap();
            let (output, caret) = apply(text, literal, extension, TableEdit::MoveColumnRight);
            assert!(output[caret..].starts_with(literal));
            let rows = cells(&output, format);
            assert_eq!(rows[0], ["名称", "数量", "标记"]);
            assert_eq!(rows[2], ["松木🌲", "12", literal]);
            if format == DocumentFormat::Markdown {
                assert_eq!(
                    parse_source_row(output.lines().nth(1).unwrap(), format).separator_alignments,
                    [(true, false), (false, true), (true, true)]
                );
            }
            let (output, caret) = apply(&output, literal, extension, TableEdit::InsertColumn);
            assert!(output[caret..].starts_with(' '));
            assert_eq!(cells(&output, format)[2], ["松木🌲", "12", "", literal]);
            let (output, _) = apply(&output, literal, extension, TableEdit::DeleteColumn);
            assert_eq!(cells(&output, format)[2], ["松木🌲", "12", ""]);
        }
    }

    #[test]
    fn table_rows_keep_crlf_indentation_and_eof() {
        let text = "intro\r\n  | Cedar | 12 |\r\n  | Birch | 3 |";
        let (output, caret) = apply(text, "Cedar", "a.org", TableEdit::MoveRowDown);
        assert_eq!(output, "intro\r\n  | Birch | 3  |\r\n  | Cedar | 12 |");
        assert!(output[caret..].starts_with("Cedar"));
        let (output, _) = apply(&output, "Birch", "a.org", TableEdit::InsertRow);
        assert_eq!(
            cells(&output, DocumentFormat::Org),
            [["", ""], ["Birch", "3"], ["Cedar", "12"]]
        );
        assert!(!output.ends_with('\n'));
        assert!(
            output
                .split("\r\n")
                .skip(1)
                .all(|row| row.starts_with("  |"))
        );
        let (output, _) = apply(
            "before\n| only |\nafter\n",
            "only",
            "a.org",
            TableEdit::KillRow,
        );
        assert_eq!(output, "before\nafter\n");
        let (output, caret) = apply("| only |", "only", "a.org", TableEdit::KillRow);
        assert!(output.is_empty());
        assert_eq!(caret, 0);
    }

    #[test]
    fn table_hline_navigation_and_row_crossing() {
        let (output, caret) = apply("| Cedar | 12 |", "Cedar", "a.org", TableEdit::HlineAndMove);
        let rows = cells(&output, DocumentFormat::Org);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2], ["", ""]);
        assert_eq!(output[..caret].lines().count(), 3);
        let (output, _) = apply(
            "| Cedar |\n|-------|\n| Birch |",
            "Cedar",
            "a.org",
            TableEdit::MoveRowDown,
        );
        assert!(output.lines().next().unwrap().starts_with("|---"));
        let (output, _) = apply(&output, "---", "a.org", TableEdit::KillRow);
        assert_eq!(cells(&output, DocumentFormat::Org), [["Cedar"], ["Birch"]]);
    }

    #[test]
    fn table_sort_uses_section_and_keeps_equal_keys_stable() {
        let text = "| Shelf | Count |\n|-------+-------|\n| Cedar | 12 |\n| Birch | 3 |\n| Maple | 3 |\n|-------+-------|\n| Willow | 1 |";
        let (output, caret) = apply(
            text,
            "12",
            "a.org",
            TableEdit::Sort {
                numeric: true,
                reverse: false,
            },
        );
        let rows = cells(&output, DocumentFormat::Org);
        assert_eq!(rows[0], ["Shelf", "Count"]);
        assert_eq!(rows[2], ["Birch", "3"]);
        assert_eq!(rows[3], ["Maple", "3"]);
        assert_eq!(rows[4], ["Cedar", "12"]);
        assert_eq!(rows[6], ["Willow", "1"]);
        assert!(output[caret..].starts_with("12"));
        let (output, _) = apply(
            &output,
            "12",
            "a.org",
            TableEdit::Sort {
                numeric: true,
                reverse: true,
            },
        );
        assert_eq!(
            cells(&output, DocumentFormat::Org)[2..5],
            [["Cedar", "12"], ["Birch", "3"], ["Maple", "3"]]
        );
        let (output, _) = apply(
            "| cedar |\n| Birch |\n| birch |",
            "cedar",
            "a.org",
            TableEdit::Sort {
                numeric: false,
                reverse: false,
            },
        );
        assert_eq!(
            cells(&output, DocumentFormat::Org),
            [["Birch"], ["birch"], ["cedar"]]
        );
    }

    #[test]
    fn table_sort_selection_uses_mark_column_and_rejects_invalid_numbers() {
        let text = "| Cedar | 12 |\n| Birch | 3 |\n| Maple | 1 |";
        for reversed in [false, true] {
            let (snapshot, table, _) = table(text, "Cedar", "a.org");
            let start = ByteOffset(text.find("12").unwrap() as u64);
            let end = ByteOffset(text.find("| Maple").unwrap() as u64);
            let selection = if reversed {
                Selection::new(ByteOffset(text.find("3").unwrap() as u64), ByteOffset(0))
            } else {
                Selection::new(start, end)
            };
            let change = table
                .edit(
                    TableEdit::Sort {
                        numeric: true,
                        reverse: false,
                    },
                    &snapshot,
                    selection,
                    "\n",
                )
                .unwrap();
            assert_eq!(
                cells(&change.replacement, DocumentFormat::Org),
                [["Birch", "3"], ["Cedar", "12"], ["Maple", "1"]]
            );
        }
        let (snapshot, table, selection) = table("| Cedar |\n| 3 |", "Cedar", "a.org");
        assert!(
            table
                .edit(
                    TableEdit::Sort {
                        numeric: true,
                        reverse: false
                    },
                    &snapshot,
                    selection,
                    "\n"
                )
                .is_err()
        );
    }

    #[test]
    fn table_availability_protects_markdown_structure_and_formula_references() {
        let text = "| Shelf | Count |\n| :--- | ---: |\n| Cedar | 12 |\n| Birch | 3 |";
        for needle in ["Shelf", ":---"] {
            let (_, table, _) = table(text, needle, "a.md");
            for edit in [
                TableEdit::KillRow,
                TableEdit::MoveRowUp,
                TableEdit::MoveRowDown,
                TableEdit::InsertRow,
                TableEdit::InsertHline,
            ] {
                assert!(!table.available(edit), "{needle}: {edit:?}");
            }
            assert!(table.available(TableEdit::InsertColumn));
        }
        let (_, first, _) = table(text, "Cedar", "a.md");
        assert!(!first.available(TableEdit::MoveRowUp));
        let (_, formula, _) = table("| 1 | 2 |\n#+TBLFM: $2=$1*2", "1", "a.org");
        assert!(formula.available(TableEdit::InsertColumn));
        assert!(formula.available(TableEdit::InsertRowBelow));
        assert!(!formula.available(TableEdit::DeleteColumn));
        assert!(!formula.available(TableEdit::InsertHline));
        for (extension, text) in [
            ("a.org", "#+begin_example\n| literal |\n#+end_example"),
            ("a.md", "```\n| literal |\n```"),
            ("a.md", "a | paragraph"),
        ] {
            let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
            let context = EditorCommandContext::at(
                Path::new(extension),
                &snapshot,
                ByteOffset(text.find('|').unwrap() as u64),
            )
            .unwrap();
            assert!(EditableTable::at(&snapshot, &context).is_none());
        }
    }

    #[test]
    fn formula_table_column_edits_keep_absolute_references_on_their_cells() {
        let source = "| 2 | 3 | 0 |\n#+TBLFM: $3=$1+$2::@1$3=A1+B1+log10(100)\n";
        let (inserted, _) = apply(source, "3 |", "a.org", TableEdit::InsertColumn);
        assert!(inserted.contains("#+TBLFM: $4=$1+$3::@1$4=A1+C1+log10(100)"));
        assert_eq!(
            cells(&inserted, DocumentFormat::Org)[0],
            ["2", "", "3", "0"]
        );
        let snapshot = DocumentSnapshot::from_utf8(inserted.as_bytes().to_vec()).unwrap();
        let caret = ByteOffset(inserted.find("#+TBLFM:").unwrap() as u64);
        let calculated = super::recalculate_table(&snapshot, caret, "\n", true)
            .unwrap()
            .unwrap();
        assert_eq!(
            cells(&calculated.replacement, DocumentFormat::Org)[0][3],
            "7"
        );

        let (moved, _) = apply(source, "3 |", "a.org", TableEdit::MoveColumnRight);
        assert!(moved.contains("#+TBLFM: $2=$1+$3::@1$2=A1+C1+log10(100)"));

        let (deleted, _) = apply(
            "| 2 | unused | 0 |\n#+TBLFM: $3=$1*2\n",
            "unused",
            "a.org",
            TableEdit::DeleteColumn,
        );
        assert!(deleted.contains("#+TBLFM: $2=$1*2"));
        let (snapshot, table, selection) = table(source, "3 |", "a.org");
        assert!(!table.available(TableEdit::DeleteColumn));
        assert!(
            table
                .edit(TableEdit::DeleteColumn, &snapshot, selection, "\n")
                .is_err()
        );
    }

    #[test]
    fn formula_table_row_edits_count_data_rows_and_preserve_multiple_formula_lines() {
        let source = "| heading | 0 |\n|---------+---|\n| A | 1 |\n| B | 2 |\n#+TBLFM: @3$2=@2$2+1\n#+TBLFM: @3$2=A3+1\n";
        let (inserted, _) = apply(source, "B |", "a.org", TableEdit::InsertRow);
        assert!(inserted.contains("#+TBLFM: @4$2=@2$2+1\n#+TBLFM: @4$2=A4+1"));
        let (moved, _) = apply(source, "B |", "a.org", TableEdit::MoveRowUp);
        assert!(moved.contains("#+TBLFM: @2$2=@3$2+1\n#+TBLFM: @2$2=A2+1"));
        assert_eq!(cells(&moved, DocumentFormat::Org)[2][0], "B");

        let (_, formula_table, _) = table(source, "A |", "a.org");
        assert!(!formula_table.available(TableEdit::KillRow));
        let (snapshot, table, selection) = table(source, "B |", "a.org");
        assert!(table.available(TableEdit::KillRow));
        let change = table
            .edit(TableEdit::KillRow, &snapshot, selection, "\n")
            .unwrap();
        assert!(!change.replacement.contains("#+TBLFM:"));
    }

    #[test]
    fn formula_table_keeps_crlf_and_special_first_column_markers() {
        let source = "| # | 2 | 0 |\r\n#+TBLFM: $3=$2*2\r\n";
        let (_, table, _) = table(source, "#", "a.org");
        assert!(!table.available(TableEdit::InsertColumn));
        assert!(!table.available(TableEdit::MoveColumnRight));
        assert!(table.available(TableEdit::InsertRowBelow));
        let (output, _) = apply(source, "2 |", "a.org", TableEdit::InsertColumn);
        assert!(output.contains("\r\n#+TBLFM: $4=$3*2\r\n"));
        assert_eq!(output.matches("\r\n#+TBLFM:").count(), 1);
    }

    #[test]
    fn formula_table_does_not_break_named_fields_with_structural_edits() {
        let named_column = "| ! | value | result |\n| # | 2 | 0 |\n#+TBLFM: $3=$value*2\n";
        let (_, named_table, _) = table(named_column, "value", "a.org");
        assert!(!named_table.available(TableEdit::DeleteColumn));
        let named_field = "| 2 | 0 |\n| ^ | input | |\n| 4 | 0 |\n#+TBLFM: @3$2=$input*2\n";
        let (_, table, _) = table(named_field, "2 |", "a.org");
        assert!(!table.available(TableEdit::KillRow));
        assert!(!table.available(TableEdit::InsertRowBelow));
    }
}
