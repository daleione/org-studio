//! Numeric `#+TBLFM:` evaluation. Unsupported Calc syntax fails before any edit is committed.
use super::*;

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

fn checked_number(value: f64) -> Result<f64, String> {
    if !value.is_finite() || value.abs() > MAX_SAFE_INTEGER {
        return Err("TBLFM number exceeds safe floating-point precision".into());
    }
    Ok(value)
}

#[derive(Clone, Copy)]
enum Axis {
    Current,
    Absolute(usize),
    Relative(isize),
    First,
    Last,
}

#[derive(Clone, Copy)]
struct CellRef {
    row: Axis,
    column: Axis,
}

enum Target {
    Column(Axis),
    Row(Axis),
    Cell(Axis, Axis),
}

struct Formula {
    target: Target,
    expression: String,
    numeric_empty: bool,
    decimals: Option<usize>,
}

struct Table {
    rows: Vec<ParsedRow>,
    data_rows: Vec<usize>,
    columns: usize,
    first_hline: Option<usize>,
}

impl Table {
    fn resolve(
        &self,
        reference: CellRef,
        row: usize,
        column: usize,
    ) -> Result<(usize, usize), String> {
        let data_row = resolve_axis(reference.row, row, self.data_rows.len())?;
        let column = resolve_axis(reference.column, column, self.columns)?;
        Ok((self.data_rows[data_row - 1], column - 1))
    }

    fn number(
        &self,
        row: usize,
        column: usize,
        numeric_empty: bool,
    ) -> Result<Option<f64>, String> {
        let value = self.rows[row].cells[column].trim();
        if value.is_empty() {
            return Ok(numeric_empty.then_some(0.0));
        }
        match value.parse::<f64>() {
            Ok(number) => Ok(Some(checked_number(number)?)),
            Err(_) if numeric_empty => Ok(Some(0.0)),
            Err(_) => Err(format!(
                "Cell @{}${} is not numeric: {value}",
                self.data_rows
                    .iter()
                    .position(|&index| index == row)
                    .unwrap_or(0)
                    + 1,
                column + 1
            )),
        }
    }
}

fn resolve_axis(axis: Axis, current: usize, last: usize) -> Result<usize, String> {
    let value = match axis {
        Axis::Current => current as isize,
        Axis::Absolute(value) => value as isize,
        Axis::Relative(delta) => current as isize + delta,
        Axis::First => 1,
        Axis::Last => last as isize,
    };
    if value < 1 || value > last as isize {
        return Err(format!("Table reference {value} is outside 1..{last}"));
    }
    Ok(value as usize)
}

pub(in crate::editor) fn recalculate(
    snapshot: &DocumentSnapshot,
    caret: ByteOffset,
    newline: &str,
    all_rows: bool,
) -> Result<Option<TableAlignment>, String> {
    let arena = parse(snapshot);
    let mut table_lines = std::collections::HashSet::new();
    let mut keyword_lines = std::collections::HashSet::new();
    for node in arena.nodes() {
        match node.kind {
            BlockKind::TableRow => {
                table_lines.insert(node.source.start.0);
            }
            BlockKind::Keyword => {
                keyword_lines.insert(node.source.start.0);
            }
            _ => {}
        }
    }
    let caret_line = snapshot
        .line_index_at(caret)
        .map_err(|error| error.to_string())?
        .0;
    let mut table_line = caret_line;
    while table_line > 0 && formula_line(snapshot, table_line, &keyword_lines).is_some() {
        table_line -= 1;
    }
    if table_line_text(snapshot, table_line, &table_lines).is_none() {
        return Ok(None);
    }
    let mut start = table_line;
    while start > 0 && table_line_text(snapshot, start - 1, &table_lines).is_some() {
        start -= 1;
    }
    let mut end = start;
    let mut rows = Vec::new();
    while end < snapshot.len_lines() {
        let Some(text) = table_line_text(snapshot, end, &table_lines) else {
            break;
        };
        rows.push(parse_source_row(&text, DocumentFormat::Org));
        end += 1;
    }
    let mut formulas = Vec::new();
    let mut formula_end = end;
    while let Some(source) = formula_line(snapshot, formula_end, &keyword_lines) {
        for part in source.split("::").filter(|part| !part.trim().is_empty()) {
            formulas.push(parse_formula(part.trim())?);
        }
        formula_end += 1;
    }
    if formulas.is_empty() {
        return Ok(None);
    }
    let columns = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
    if columns == 0 {
        return Err("Table has no cells".into());
    }
    for row in &mut rows {
        row.cells.resize(columns, String::new());
        row.separator_alignments.resize(columns, (false, false));
    }
    let data_rows = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| (!row.separator).then_some(index))
        .collect::<Vec<_>>();
    let first_hline = rows.iter().position(|row| row.separator);
    let mut table = Table {
        rows,
        data_rows,
        columns,
        first_hline,
    };
    let mut field_targets = std::collections::HashSet::new();
    for formula in &formulas {
        if let Target::Cell(row, column) = formula.target {
            let data_row = resolve_axis(row, 1, table.data_rows.len())?;
            let column = resolve_axis(column, 1, table.columns)?;
            field_targets.insert((data_row, column));
        } else if let Target::Row(row) = formula.target {
            let data_row = resolve_axis(row, 1, table.data_rows.len())?;
            for column in 1..=table.columns {
                field_targets.insert((data_row, column));
            }
        }
    }
    let mut columns = formulas
        .iter()
        .filter_map(|formula| match formula.target {
            Target::Column(axis) => Some((resolve_axis(axis, 1, table.columns), formula)),
            Target::Row(_) | Target::Cell(_, _) => None,
        })
        .map(|(column, formula)| Ok((column?, formula)))
        .collect::<Result<Vec<_>, String>>()?;
    columns.sort_by_key(|(column, _)| *column);
    for data_row in 1..=table.data_rows.len() {
        let physical = table.data_rows[data_row - 1];
        if !all_rows && caret_line < end && caret_line != start + physical as u64 {
            continue;
        }
        if all_rows && table.first_hline.is_some_and(|hline| physical < hline) {
            continue;
        }
        if table.first_hline.is_none_or(|hline| physical > hline) {
            for &(column, formula) in &columns {
                if !field_targets.contains(&(data_row, column)) {
                    apply_formula(&mut table, physical, data_row, column, formula)?;
                }
            }
        }
        for formula in &formulas {
            if let Target::Row(row) = formula.target
                && resolve_axis(row, 1, table.data_rows.len())? == data_row
            {
                for column in 1..=table.columns {
                    apply_formula(&mut table, physical, data_row, column, formula)?;
                }
            }
            if let Target::Cell(row, column) = formula.target
                && resolve_axis(row, 1, table.data_rows.len())? == data_row
            {
                let column = resolve_axis(column, 1, table.columns)?;
                apply_formula(&mut table, physical, data_row, column, formula)?;
            }
        }
    }
    let range = ByteRange::new(
        snapshot
            .line_content_range(LineIndex(start))
            .map_err(|error| error.to_string())?
            .start
            .0,
        snapshot
            .line_content_range(LineIndex(end - 1))
            .map_err(|error| error.to_string())?
            .end
            .0,
    );
    let widths = table_widths(&table.rows, table.columns, DocumentFormat::Org);
    let mut replacement = String::new();
    let mut caret_after = caret;
    for (index, row) in table.rows.iter().enumerate() {
        if index > 0 {
            replacement.push_str(newline);
        }
        let caret_position = (caret_line == start + index as u64).then(|| {
            let source = table_line_text(snapshot, caret_line, &table_lines).unwrap_or_default();
            table_position_at(
                &source,
                caret.0.saturating_sub(
                    snapshot
                        .line_content_range(LineIndex(caret_line))
                        .unwrap()
                        .start
                        .0,
                ) as usize,
                DocumentFormat::Org,
            )
        });
        let (text, local) = aligned_row(row, &widths, caret_position, DocumentFormat::Org);
        if let Some(local) = local {
            caret_after = ByteOffset(range.start.0 + (replacement.len() + local) as u64);
        }
        replacement.push_str(&text);
    }
    if caret_line >= end {
        let old_len = range.end.0 - range.start.0;
        caret_after = ByteOffset(
            caret
                .0
                .saturating_add(replacement.len() as u64)
                .saturating_sub(old_len),
        );
    }
    Ok(Some(TableAlignment {
        range,
        replacement,
        caret: caret_after,
    }))
}

fn structural_line_text(
    snapshot: &DocumentSnapshot,
    line: u64,
    starts: &std::collections::HashSet<u64>,
) -> Option<String> {
    let range = snapshot.line_content_range(LineIndex(line)).ok()?;
    if !starts.contains(&range.start.0) {
        return None;
    }
    Some(snapshot.copy_range(range))
}

fn table_line_text(
    snapshot: &DocumentSnapshot,
    line: u64,
    starts: &std::collections::HashSet<u64>,
) -> Option<String> {
    let text = structural_line_text(snapshot, line, starts)?;
    source_table::is_table_row(&text, DocumentFormat::Org).then_some(text)
}

fn formula_line(
    snapshot: &DocumentSnapshot,
    line: u64,
    starts: &std::collections::HashSet<u64>,
) -> Option<String> {
    let text = structural_line_text(snapshot, line, starts)?;
    let text = text.trim_start();
    text.get(..8)
        .filter(|prefix| prefix.eq_ignore_ascii_case("#+TBLFM:"))
        .map(|_| text[8..].trim().to_owned())
}

fn parse_formula(source: &str) -> Result<Formula, String> {
    let (target_source, rhs) = source
        .split_once('=')
        .ok_or_else(|| format!("Invalid TBLFM formula: {source}"))?;
    let mut parser = Parser::new(target_source.trim(), None);
    let reference = parser.parse_reference()?;
    parser.finish()?;
    let target = if target_source.trim().starts_with('@') {
        if target_source.contains('$') {
            Target::Cell(reference.row, reference.column)
        } else {
            Target::Row(reference.row)
        }
    } else {
        Target::Column(reference.column)
    };
    let valid = |axis| matches!(axis, Axis::Absolute(_) | Axis::First | Axis::Last);
    if !match target {
        Target::Column(column) | Target::Row(column) => valid(column),
        Target::Cell(row, column) => valid(row) && valid(column),
    } {
        return Err(format!(
            "Unsupported TBLFM target: {}",
            target_source.trim()
        ));
    }
    let (expression, mode) = rhs
        .split_once(';')
        .map_or((rhs, ""), |(expr, mode)| (expr, mode));
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("Empty TBLFM expression".into());
    }
    let mode = mode.trim();
    let numeric_empty = mode.contains('N');
    if mode.contains('E') && !numeric_empty {
        return Err("TBLFM ;E mode requires Calc nan handling and is not supported".into());
    }
    let remainder = mode.replace(['N', 'E'], "");
    let decimals = if remainder.trim().is_empty() {
        None
    } else {
        let value = remainder
            .trim()
            .strip_prefix("%.")
            .and_then(|value| value.strip_suffix('f'))
            .ok_or_else(|| format!("Unsupported TBLFM mode: {mode}"))?;
        let decimals = value
            .parse::<usize>()
            .map_err(|_| format!("Invalid TBLFM precision: {mode}"))?;
        if decimals > 12 {
            return Err("TBLFM precision must be at most 12".into());
        }
        Some(decimals)
    };
    Ok(Formula {
        target,
        expression: expression.into(),
        numeric_empty,
        decimals,
    })
}

fn apply_formula(
    table: &mut Table,
    physical: usize,
    row: usize,
    column: usize,
    formula: &Formula,
) -> Result<(), String> {
    let value = Parser::new(
        &formula.expression,
        Some((table, row, column, formula.numeric_empty)),
    )
    .parse()?;
    let value = checked_number(value)?;
    let output = if let Some(decimals) = formula.decimals {
        format!("{value:.decimals$}")
    } else if value.fract() == 0.0 && value.abs() < i64::MAX as f64 {
        format!("{value:.0}")
    } else if value != 0.0 && !(1e-6..1e12).contains(&value.abs()) {
        format!("{value:.8e}")
    } else {
        format!("{value:.8}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    };
    table.rows[physical].cells[column - 1] = output;
    Ok(())
}

#[derive(Clone)]
enum Value {
    Number(f64),
    Vector(Vec<f64>),
}

impl Value {
    fn scalar(value: f64) -> Result<Self, String> {
        Ok(Self::Number(checked_number(value)?))
    }

    fn number(self) -> Result<f64, String> {
        match self {
            Self::Number(value) => Ok(value),
            Self::Vector(_) => Err("Range must be used inside a vector function".into()),
        }
    }
}

struct Parser<'a> {
    source: &'a str,
    position: usize,
    context: Option<(&'a Table, usize, usize, bool)>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, context: Option<(&'a Table, usize, usize, bool)>) -> Self {
        Self {
            source,
            position: 0,
            context,
        }
    }
    fn rest(&self) -> &'a str {
        &self.source[self.position..]
    }
    fn space(&mut self) {
        self.position += self.rest().len() - self.rest().trim_start().len();
    }
    fn eat(&mut self, text: &str) -> bool {
        self.space();
        if self.rest().starts_with(text) {
            self.position += text.len();
            true
        } else {
            false
        }
    }
    fn finish(&mut self) -> Result<(), String> {
        self.space();
        if self.rest().is_empty() {
            Ok(())
        } else {
            Err(format!("Unsupported TBLFM syntax near: {}", self.rest()))
        }
    }
    fn parse(&mut self) -> Result<f64, String> {
        let value = self.add()?.number()?;
        self.finish()?;
        Ok(value)
    }
    fn add(&mut self) -> Result<Value, String> {
        let mut value = self.divide()?;
        loop {
            if self.eat("+") {
                value = Value::scalar(value.number()? + self.divide()?.number()?)?;
            } else if self.eat("-") {
                value = Value::scalar(value.number()? - self.divide()?.number()?)?;
            } else {
                break;
            }
        }
        Ok(value)
    }
    // Calc gives multiplication higher precedence than division.
    fn divide(&mut self) -> Result<Value, String> {
        let mut value = self.multiply()?;
        while self.eat("/") {
            let right = self.multiply()?.number()?;
            if right == 0.0 {
                return Err("Division by zero in TBLFM".into());
            }
            value = Value::scalar(value.number()? / right)?;
        }
        Ok(value)
    }
    fn multiply(&mut self) -> Result<Value, String> {
        let mut value = self.unary()?;
        while self.eat("*") {
            value = Value::scalar(value.number()? * self.unary()?.number()?)?;
        }
        Ok(value)
    }
    fn unary(&mut self) -> Result<Value, String> {
        if self.eat("-") {
            return Value::scalar(-self.unary()?.number()?);
        }
        if self.eat("+") {
            return self.unary();
        }
        self.power()
    }
    fn power(&mut self) -> Result<Value, String> {
        let mut value = self.atom()?;
        if self.eat("^") {
            value = Value::scalar(value.number()?.powf(self.unary()?.number()?))?;
        }
        Ok(value)
    }
    fn atom(&mut self) -> Result<Value, String> {
        self.space();
        if self.rest().starts_with("@#") && !self.rest()[2..].starts_with('$') {
            self.position += 2;
            return Value::scalar(self.context.ok_or("Reference outside table")?.1 as f64);
        }
        if self.rest().starts_with("$#") {
            self.position += 2;
            return Value::scalar(self.context.ok_or("Reference outside table")?.2 as f64);
        }
        if self.eat("(") {
            let value = self.add()?;
            if !self.eat(")") {
                return Err("Missing ')' in TBLFM".into());
            }
            return Ok(value);
        }
        if self.rest().starts_with(['$', '@']) {
            let first = self.parse_reference()?;
            let context = self.context.ok_or("Reference outside table")?;
            if self.eat("..") {
                let last = self.parse_reference()?;
                let (first_row, first_col) = context.0.resolve(first, context.1, context.2)?;
                let (last_row, last_col) = context.0.resolve(last, context.1, context.2)?;
                let mut numbers = Vec::new();
                let data_start = context
                    .0
                    .data_rows
                    .iter()
                    .position(|&index| index == first_row)
                    .unwrap();
                let data_end = context
                    .0
                    .data_rows
                    .iter()
                    .position(|&index| index == last_row)
                    .unwrap();
                for data_row in data_start.min(data_end)..=data_start.max(data_end) {
                    for column in first_col.min(last_col)..=first_col.max(last_col) {
                        if let Some(value) =
                            context
                                .0
                                .number(context.0.data_rows[data_row], column, context.3)?
                        {
                            numbers.push(value);
                        }
                    }
                }
                return Ok(Value::Vector(numbers));
            }
            let (row, column) = context.0.resolve(first, context.1, context.2)?;
            return Value::scalar(
                context
                    .0
                    .number(row, column, context.3)?
                    .ok_or("Empty field in TBLFM reference")?,
            );
        }
        let len = self
            .rest()
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            .count();
        if len > 0 && self.rest().as_bytes()[0].is_ascii_alphabetic() {
            let name = self.rest()[..len].to_ascii_lowercase();
            self.position += len;
            if !self.eat("(") {
                return Err(format!("Unknown TBLFM name: {name}"));
            }
            let argument = self.add()?;
            if !self.eat(")") {
                return Err("Missing ')' in TBLFM function".into());
            }
            let value = match (name.as_str(), argument) {
                ("vsum", Value::Vector(values)) => values
                    .iter()
                    .try_fold(0.0, |sum, value| checked_number(sum + value))?,
                ("vmean", Value::Vector(values)) if !values.is_empty() => {
                    values
                        .iter()
                        .try_fold(0.0, |sum, value| checked_number(sum + value))?
                        / values.len() as f64
                }
                ("vmin", Value::Vector(values)) if !values.is_empty() => {
                    values.into_iter().fold(f64::INFINITY, f64::min)
                }
                ("vmax", Value::Vector(values)) if !values.is_empty() => {
                    values.into_iter().fold(f64::NEG_INFINITY, f64::max)
                }
                ("abs", Value::Number(value)) => value.abs(),
                ("sqrt", Value::Number(value)) => value.sqrt(),
                _ => return Err(format!("Unsupported TBLFM function or empty range: {name}")),
            };
            return Value::scalar(value);
        }
        let bytes = self.rest().as_bytes();
        let mut len = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if bytes.get(len) == Some(&b'.') && bytes.get(len + 1).is_some_and(u8::is_ascii_digit) {
            len += 1;
            len += bytes[len..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
        }
        if len > 0 && matches!(bytes.get(len), Some(b'e' | b'E')) {
            let exponent = len + usize::from(matches!(bytes.get(len + 1), Some(b'+' | b'-'))) + 1;
            let digits = bytes[exponent..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if digits > 0 {
                len = exponent + digits;
            }
        }
        if len == 0 {
            return Err(format!(
                "Expected number or reference near: {}",
                self.rest()
            ));
        }
        let number = self.rest()[..len]
            .parse::<f64>()
            .map_err(|_| "Invalid TBLFM number")?;
        self.position += len;
        Value::scalar(number)
    }
    fn parse_reference(&mut self) -> Result<CellRef, String> {
        self.space();
        let row = if self.eat("@") {
            self.axis()?
        } else {
            Axis::Current
        };
        let column = if self.eat("$") {
            self.axis()?
        } else {
            Axis::Current
        };
        Ok(CellRef { row, column })
    }
    fn axis(&mut self) -> Result<Axis, String> {
        if self.eat("#") || self.eat("0") {
            return Ok(Axis::Current);
        }
        if self.eat("<") {
            return Ok(Axis::First);
        }
        if self.eat(">") {
            return Ok(Axis::Last);
        }
        let relative = if self.eat("+") { true } else { self.eat("-") };
        let negative = relative && self.source.as_bytes().get(self.position - 1) == Some(&b'-');
        let len = self.rest().bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 {
            return Err(format!("Invalid TBLFM reference near: {}", self.rest()));
        }
        let value = self.rest()[..len]
            .parse::<usize>()
            .map_err(|_| "Invalid TBLFM coordinate")?;
        self.position += len;
        if relative {
            Ok(Axis::Relative(if negative {
                -(value as isize)
            } else {
                value as isize
            }))
        } else {
            Ok(Axis::Absolute(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentSession;
    use gpui::AppContext;

    fn calculated(source: &str) -> Result<String, String> {
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let caret = ByteOffset(source.find("#+TBLFM:").unwrap() as u64);
        let change = recalculate(&snapshot, caret, "\n", true)?.unwrap();
        let mut result = source.to_owned();
        result.replace_range(
            change.range.start.0 as usize..change.range.end.0 as usize,
            &change.replacement,
        );
        Ok(result)
    }

    fn cell(source: &str, line: usize, column: usize) -> String {
        let text = source.lines().nth(line).unwrap();
        let parsed = source_table::parse_line(text, DocumentFormat::Org);
        text[parsed.cells[column].text_range.clone()].to_owned()
    }

    #[test]
    fn column_formulas_skip_headers_and_field_formulas_override_them() {
        let source = "| Item | Qty | Unit | Total |\n|------+-----+------+-------|\n| A | 2 | 3 | 0 |\n| B | 4 | 5 | 0 |\n#+TBLFM: $4=$2*$3::@3$4=vsum(@2$2..@3$2)\n";
        let result = calculated(source).unwrap();
        assert_eq!(cell(&result, 0, 3), "Total");
        assert_eq!(cell(&result, 2, 3), "6");
        assert_eq!(cell(&result, 3, 3), "6");
        assert!(result.contains("#+TBLFM: $4=$2*$3::@3$4=vsum(@2$2..@3$2)"));
    }

    #[test]
    fn range_aggregation_and_calc_division_order() {
        let source = "| n | x | total |\n|---+---+-------|\n| A | 2 | 0 |\n| B | 4 | 0 |\n| Mean | 0 | 0 |\n#+TBLFM: $3=$2/2*2::@4$3=vmean(@2$2..@3$2);%.1f\n";
        let result = calculated(source).unwrap();
        assert_eq!(cell(&result, 4, 2), "3.0");
        assert_eq!(cell(&result, 2, 2), "0.5");
        assert_eq!(cell(&result, 3, 2), "1");
    }

    #[test]
    fn current_row_recalculation_uses_relative_columns() {
        let source = "| 2 | 3 | 0 |\n| 4 | 5 | 0 |\n#+TBLFM: $3=$-2+$-1\n";
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let change = recalculate(&snapshot, ByteOffset(2), "\n", false)
            .unwrap()
            .unwrap();
        let mut result = source.to_owned();
        result.replace_range(
            change.range.start.0 as usize..change.range.end.0 as usize,
            &change.replacement,
        );
        assert_eq!(cell(&result, 0, 2), "5");
        assert_eq!(cell(&result, 1, 2), "0");
    }

    #[test]
    fn full_recalculation_preserves_headers_and_coordinates_are_numbers() {
        let source = "| 0 | Heading | 0 |\n|---+---------+---|\n| 2 | A | 0 |\n| 4 | B | 0 |\n#+TBLFM: $3=$1*2::@1$1=10::@3$3=@#+$#\n";
        let result = calculated(source).unwrap();
        assert_eq!(cell(&result, 0, 0), "0");
        assert_eq!(cell(&result, 0, 2), "0");
        assert_eq!(cell(&result, 2, 2), "4");
        assert_eq!(cell(&result, 3, 2), "6");
    }

    #[test]
    fn current_header_row_can_apply_its_field_formula() {
        let source = "| 0 | Label |\n|---+-------|\n| 1 | Body |\n#+TBLFM: @1$1=10\n";
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let change = recalculate(&snapshot, ByteOffset(2), "\n", false)
            .unwrap()
            .unwrap();
        let mut result = source.to_owned();
        result.replace_range(
            change.range.start.0 as usize..change.range.end.0 as usize,
            &change.replacement,
        );
        assert_eq!(cell(&result, 0, 0), "10");
    }

    #[test]
    fn formula_text_inside_source_block_is_not_a_table() {
        let source =
            "#+begin_src python\ntext = '''\n| 1 | 0 |\n#+TBLFM: $2=$1*2\n'''\n#+end_src\n";
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        for marker in ["| 1 | 0 |", "#+TBLFM:"] {
            let caret = ByteOffset(source.find(marker).unwrap() as u64);
            assert!(recalculate(&snapshot, caret, "\n", true).unwrap().is_none());
        }
    }

    #[test]
    fn unsafe_integer_inputs_and_results_fail_without_writing() {
        for source in [
            "| 9007199254740992 | 0 |\n#+TBLFM: $2=$1+1\n",
            "| 1 | 0 |\n#+TBLFM: $2=9007199254740992+1\n",
            "| 9007199254740991 | 0 |\n#+TBLFM: $2=$1+1\n",
        ] {
            assert!(calculated(source).unwrap_err().contains("precision"));
        }
    }

    #[test]
    fn last_row_target_and_numeric_empty_mode() {
        let source = "| 2 |   | 0 |\n| 4 | 3 | 0 |\n| 0 | 0 | 0 |\n#+TBLFM: $3=$1+$2;N::@>$3=vsum(@1$1..@2$1)\n";
        let result = calculated(source).unwrap();
        assert_eq!(cell(&result, 0, 2), "2");
        assert_eq!(cell(&result, 1, 2), "7");
        assert_eq!(cell(&result, 2, 2), "6");
    }

    #[test]
    fn unsupported_formula_fails_without_changing_source() {
        let source = "| 1 | 2 |\n| 3 | 4 |\n#+TBLFM: $2=unknown($1)\n";
        assert!(calculated(source).unwrap_err().contains("Unsupported"));
    }

    #[gpui::test]
    fn editor_recalculates_in_one_undo_step(cx: &mut gpui::TestAppContext) {
        cx.update(crate::editor::init);
        let source = "| A | 2 | 3 | 0 |\n| B | 4 | 5 | 0 |\n#+TBLFM: $4=$2*$3\n";
        let session = cx.new(|_| {
            DocumentSession::from_utf8("table.org".into(), source.as_bytes().to_vec()).unwrap()
        });
        let (editor, view) =
            cx.add_window_view(|_, cx| crate::editor::SemanticEditor::new(session.clone(), cx));
        view.run_until_parked();
        editor.update(view, |editor, cx| {
            editor.set_selection(
                crate::document::Selection::caret(ByteOffset(
                    source.find("#+TBLFM:").unwrap() as u64
                )),
                cx,
            );
            assert!(editor.recalculate_table_at_selection(false, cx).unwrap());
        });
        view.read(|cx| {
            let snapshot = session.read(cx).snapshot();
            let text = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
            assert_eq!(cell(&text, 0, 3), "6");
            assert_eq!(cell(&text, 1, 3), "20");
        });
        session.update(view, |session, cx| {
            session.undo(cx).unwrap();
        });
        view.read(|cx| {
            let snapshot = session.read(cx).snapshot();
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                source
            );
        });
    }
}
