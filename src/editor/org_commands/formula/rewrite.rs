use super::{Axis, Parser, Target, parse_formula, split_top_level, top_level_separator};

/// Coordinates are one-based data rows (hlines excluded) or columns.
#[derive(Clone, Copy)]
pub(in crate::editor::org_commands) enum AxisEdit {
    Insert(usize),
    Delete(usize),
    Swap(usize, usize),
}

impl AxisEdit {
    fn map(self, value: usize) -> Option<usize> {
        match self {
            Self::Insert(at) if value >= at => value.checked_add(1),
            Self::Insert(_) => Some(value),
            Self::Delete(at) if value == at => None,
            Self::Delete(at) => Some(value - usize::from(value > at)),
            Self::Swap(first, second) if value == first => Some(second),
            Self::Swap(first, second) if value == second => Some(first),
            Self::Swap(..) => Some(value),
        }
    }

    fn deletes(self) -> bool {
        matches!(self, Self::Delete(_))
    }
}

pub(in crate::editor::org_commands) fn adapt_formula_lines(
    lines: &[String],
    row: Option<AxisEdit>,
    column: Option<AxisEdit>,
) -> Option<Vec<String>> {
    lines
        .iter()
        .map(|line| adapt_line(line, row, column))
        .collect::<Option<Vec<_>>>()
        .map(|lines| lines.into_iter().flatten().collect())
}

fn target_deleted(
    target: &Target,
    row: Option<AxisEdit>,
    column: Option<AxisEdit>,
) -> Option<bool> {
    let removed = |axis: Axis, edit: Option<AxisEdit>| {
        if let (Axis::Absolute(value), Some(edit)) = (axis, edit) {
            edit.map(value).is_none()
        } else {
            false
        }
    };
    match target {
        Target::Column(axis) => Some(removed(*axis, column)),
        Target::Row(axis) => Some(removed(*axis, row)),
        Target::Cell(r, c) => Some(removed(*r, row) || removed(*c, column)),
        Target::Range(first, last) => (!removed(first.row, row)
            && !removed(first.column, column)
            && !removed(last.row, row)
            && !removed(last.column, column))
        .then_some(false),
        // The location of a named field depends on special rows; do not guess on deletion.
        Target::Named(_)
            if row.is_some_and(AxisEdit::deletes) || column.is_some_and(AxisEdit::deletes) =>
        {
            None
        }
        Target::Named(_) => Some(false),
    }
}

fn adapt_line(
    line: &str,
    row: Option<AxisEdit>,
    column: Option<AxisEdit>,
) -> Option<Option<String>> {
    let indent = line.len() - line.trim_start().len();
    let prefix_end = indent + 8;
    if !line
        .get(indent..prefix_end)?
        .eq_ignore_ascii_case("#+TBLFM:")
    {
        return None;
    }
    let mut formulas = Vec::new();
    for part in split_top_level(&line[prefix_end..], "::") {
        if part.trim().is_empty() {
            continue;
        }
        let formula = parse_formula(part.trim()).ok()?;
        if target_deleted(&formula.target, row, column)? {
            continue;
        }
        // Lisp formulas have a different reference grammar.
        if formula.expression.trim_start().starts_with("'(") {
            return None;
        }
        formulas.push(rewrite_refs(part, row, column)?);
    }
    if formulas.is_empty() {
        Some(None)
    } else {
        let body = &line[prefix_end..];
        let spacing = &body[..body.len() - body.trim_start().len()];
        let joined = formulas.join("::");
        Some(Some(format!(
            "{}{}{}",
            &line[..prefix_end],
            if joined.starts_with(char::is_whitespace) {
                ""
            } else {
                spacing
            },
            joined
        )))
    }
}

fn rewrite_refs(source: &str, row: Option<AxisEdit>, column: Option<AxisEdit>) -> Option<String> {
    let mut result = String::with_capacity(source.len());
    let mut at = 0;
    while at < source.len() {
        let rest = &source[at..];
        let byte = rest.as_bytes()[0];
        if byte == b'"' {
            let end = quoted_end(rest)?;
            let quoted = &rest[1..end - 1];
            let mut reference = Parser::new(quoted, None);
            if reference.parse_reference().is_ok() && reference.finish().is_ok() {
                result.push('"');
                result.push_str(&rewrite_refs(quoted, row, column)?);
                result.push('"');
            } else {
                result.push_str(&rest[..end]);
            }
            at += end;
            continue;
        }
        if byte == b'$' || byte == b'@' {
            let digits = rest[1..].bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 {
                let value = rest[1..1 + digits].parse::<usize>().ok()?;
                if value > 0 {
                    let mapped = if byte == b'$' { column } else { row }
                        .map_or(Some(value), |edit| edit.map(value))?;
                    result.push(byte as char);
                    result.push_str(&mapped.to_string());
                    at += 1 + digits;
                    continue;
                }
            }
            if byte == b'$' && rest.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic) {
                let len = rest[1..]
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                result.push_str(&rest[..1 + len]);
                at += 1 + len;
                continue;
            }
        }
        if byte.is_ascii_alphabetic() {
            let len = rest
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                .count();
            let name = &rest[..len];
            if name.eq_ignore_ascii_case("remote") {
                let spaces = rest[len..]
                    .bytes()
                    .take_while(u8::is_ascii_whitespace)
                    .count();
                if rest.as_bytes().get(len + spaces) == Some(&b'(') {
                    let open = len + spaces;
                    let close = closing_paren(rest, open)?;
                    let inner = &rest[open + 1..close];
                    let comma = top_level_separator(inner, ",")?;
                    let name_arg = &inner[..comma];
                    result.push_str(&rest[..open + 1]);
                    if name_arg.trim_start().starts_with(['$', '@']) {
                        result.push_str(&rewrite_refs(name_arg, row, column)?);
                    } else {
                        result.push_str(name_arg);
                    }
                    result.push_str(&rest[open + 1 + comma..=close]);
                    at += close + 1;
                    continue;
                }
            }
            let letters = name.bytes().take_while(u8::is_ascii_alphabetic).count();
            if letters > 0
                && letters < len
                && name[letters..].bytes().all(|byte| byte.is_ascii_digit())
                && source[..at]
                    .bytes()
                    .next_back()
                    .is_none_or(|previous| !previous.is_ascii_alphanumeric() && previous != b'_')
                && !rest[len..].trim_start().starts_with('(')
            {
                let original_column = name[..letters].bytes().try_fold(0_usize, |n, byte| {
                    n.checked_mul(26)?
                        .checked_add(usize::from(byte.to_ascii_uppercase() - b'A' + 1))
                })?;
                let original_row = name[letters..].parse::<usize>().ok()?;
                if original_row > 0 {
                    let next_column =
                        column.map_or(Some(original_column), |edit| edit.map(original_column))?;
                    let next_row = row.map_or(Some(original_row), |edit| edit.map(original_row))?;
                    result.push_str(&column_letters(
                        next_column,
                        name.as_bytes()[0].is_ascii_lowercase(),
                    ));
                    result.push_str(&next_row.to_string());
                    at += len;
                    continue;
                }
            }
            result.push_str(name);
            at += len;
            continue;
        }
        let ch = rest.chars().next()?;
        result.push(ch);
        at += ch.len_utf8();
    }
    Some(result)
}

fn column_letters(mut value: usize, lower: bool) -> String {
    let mut letters = Vec::new();
    while value > 0 {
        value -= 1;
        let byte = if lower { b'a' } else { b'A' };
        letters.push((byte + (value % 26) as u8) as char);
        value /= 26;
    }
    letters.into_iter().rev().collect()
}

fn quoted_end(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut at = 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return Some(at + 1),
            _ => at += 1,
        }
    }
    None
}

fn closing_paren(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0;
    let mut at = open;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => at += quoted_end(&source[at..])?,
            b'(' => {
                depth += 1;
                at += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(
        source: &str,
        row: Option<AxisEdit>,
        column: Option<AxisEdit>,
    ) -> Option<Vec<String>> {
        adapt_formula_lines(&[source.to_owned()], row, column)
    }

    #[test]
    fn rewrites_only_local_references_outside_strings_and_remote_targets() {
        let source =
            "#+TBLFM: $4=if(\"$2 A1\" == \"$2 A1\",$2+remote(base1,@2$3)+remote($1,@1$2),0)";
        assert_eq!(
            rewrite(source, None, Some(AxisEdit::Insert(2))),
            Some(vec![
                "#+TBLFM: $5=if(\"$2 A1\" == \"$2 A1\",$3+remote(base1,@2$3)+remote($1,@1$2),0)"
                    .into()
            ])
        );
        assert_eq!(
            rewrite(
                "#+TBLFM: $3=if(\"$2\" == \"A1\",1,0)",
                None,
                Some(AxisEdit::Insert(2))
            ),
            Some(vec!["#+TBLFM: $4=if(\"$3\" == \"A1\",1,0)".into()])
        );
        assert_eq!(
            rewrite(
                "#+TBLFM: $3=remote($1,@1$2)+remote(base1,@1$2)",
                None,
                Some(AxisEdit::Insert(1))
            ),
            Some(vec!["#+TBLFM: $4=remote($2,@1$2)+remote(base1,@1$2)".into()])
        );
    }

    #[test]
    fn deletion_drops_target_formulas_but_rejects_live_sources_and_named_targets() {
        assert_eq!(
            rewrite(
                "#+TBLFM: $3=$1+$2::$4=$1*2",
                None,
                Some(AxisEdit::Delete(3))
            ),
            Some(vec!["#+TBLFM: $3=$1*2".into()])
        );
        assert_eq!(
            rewrite("#+TBLFM: $3=$2*2", None, Some(AxisEdit::Delete(2))),
            None
        );
        assert_eq!(
            rewrite("#+TBLFM: $sum=$1+$2", None, Some(AxisEdit::Delete(2))),
            None
        );
        assert_eq!(
            rewrite("#+TBLFM: $3=$1", None, Some(AxisEdit::Delete(3))),
            Some(vec![])
        );
    }
}
