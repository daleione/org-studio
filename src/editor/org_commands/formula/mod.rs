//! `#+TBLFM:` evaluation. Unsupported Calc syntax fails before any edit is committed.
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
    FromFirst(usize),
    FromLast(usize),
    Hline {
        ordinal: usize,
        direction: i8,
        offset: isize,
    },
}

#[derive(Clone, Copy)]
enum RangeEdge {
    Start,
    End,
    Field,
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
    Range(CellRef, CellRef),
    Named(String),
}

struct Formula {
    target: Target,
    expression: String,
    empty_mode: EmptyMode,
    number_format: NumberFormat,
    duration: Option<DurationFormat>,
    radians: bool,
}

#[derive(Clone, Copy)]
enum NumberFormat {
    Default,
    Fixed(usize),
    Scientific(usize),
    Engineering(usize),
    Normal(usize),
    PrintfFixed(usize),
    PrintfScientific(usize),
}

fn format_number(value: f64, mode: NumberFormat) -> String {
    let trim_fraction = |text: String| {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text
        }
    };
    match mode {
        NumberFormat::Fixed(digits) | NumberFormat::PrintfFixed(digits) => {
            format!("{value:.digits$}")
        }
        NumberFormat::Scientific(digits) | NumberFormat::PrintfScientific(digits) => {
            let places = if matches!(mode, NumberFormat::Scientific(_)) {
                digits.saturating_sub(1)
            } else {
                digits
            };
            format!("{value:.places$e}")
        }
        NumberFormat::Engineering(digits) if value != 0.0 => {
            let scientific = format!("{value:e}");
            let (mantissa, exponent) = scientific.split_once('e').expect("scientific notation");
            let exponent = exponent.parse::<i32>().expect("scientific exponent");
            let engineering_exponent = exponent.div_euclid(3) * 3;
            let scaled = mantissa.parse::<f64>().expect("scientific mantissa")
                * 10_f64.powi(exponent - engineering_exponent);
            format!(
                "{scaled:.places$}e{engineering_exponent}",
                places = digits.saturating_sub(1)
            )
        }
        NumberFormat::Engineering(digits) => {
            format!("{value:.places$}e0", places = digits.saturating_sub(1))
        }
        NumberFormat::Normal(digits) if value != 0.0 && value.fract() != 0.0 => {
            let places =
                (digits as i32 - 1 - value.abs().log10().floor() as i32).clamp(0, 15) as usize;
            trim_fraction(format!("{value:.places$}"))
        }
        NumberFormat::Default | NumberFormat::Normal(_) => {
            if value.fract() == 0.0 && value.abs() < i64::MAX as f64 {
                format!("{value:.0}")
            } else if value != 0.0 && !(1e-6..1e12).contains(&value.abs()) {
                format!("{value:.8e}")
            } else {
                trim_fraction(format!("{value:.8}"))
            }
        }
    }
}

#[derive(Clone, Copy)]
enum EmptyMode {
    Skip,
    Nan,
    Zero,
}

#[derive(Clone, Copy)]
enum DurationFormat {
    HoursMinutesSeconds,
    HoursMinutes,
    DecimalHours,
}

fn parse_duration(value: &str) -> Option<f64> {
    let (negative, value) = value
        .strip_prefix('-')
        .map_or((false, value), |value| (true, value));
    let parts = value.split(':').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let hours = parts[0].parse::<u64>().ok()?;
    let minutes = parts[1].parse::<u64>().ok()?;
    let seconds = if parts.len() == 3 {
        parts[2].parse::<u64>().ok()?
    } else {
        0
    };
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    let total = hours
        .checked_mul(3600)?
        .checked_add(minutes * 60 + seconds)?;
    checked_number(total as f64 * if negative { -1.0 } else { 1.0 }).ok()
}

fn format_duration(value: f64, mode: DurationFormat) -> String {
    if matches!(mode, DurationFormat::DecimalHours) {
        return format!("{:.2}", value / 3600.0)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned();
    }
    let seconds = value.round().abs() as u64;
    let sign = if value < 0.0 { "-" } else { "" };
    let hours = seconds / 3600;
    let minutes = seconds / 60 % 60;
    if matches!(mode, DurationFormat::HoursMinutesSeconds) {
        format!("{sign}{hours:02}:{minutes:02}:{:02}", seconds % 60)
    } else {
        format!("{sign}{hours:02}:{minutes:02}")
    }
}

#[derive(Clone, Copy)]
enum NamedValue {
    Constant(f64),
    Column(usize),
    Cell(usize, usize),
}

struct Table {
    rows: Vec<ParsedRow>,
    data_rows: Vec<usize>,
    columns: usize,
    first_hline: Option<usize>,
    hlines: Vec<usize>,
    names: std::collections::HashMap<String, NamedValue>,
    remotes: std::collections::HashMap<String, std::sync::Arc<Table>>,
}

impl Table {
    fn resolve(
        &self,
        reference: CellRef,
        row: usize,
        column: usize,
        edge: RangeEdge,
    ) -> Result<(usize, usize), String> {
        let data_row = self.resolve_row(reference.row, row, edge)?;
        let column = resolve_axis(reference.column, column, self.columns)?;
        Ok((self.data_rows[data_row - 1], column - 1))
    }

    fn resolve_row(&self, axis: Axis, current: usize, edge: RangeEdge) -> Result<usize, String> {
        let Axis::Hline {
            ordinal,
            direction,
            offset,
        } = axis
        else {
            return resolve_axis(axis, current, self.data_rows.len());
        };
        let current_physical = self.data_rows.get(current - 1).copied().unwrap_or(0);
        let hline = match direction {
            -1 => self
                .hlines
                .iter()
                .rev()
                .filter(|&&line| line < current_physical)
                .nth(ordinal - 1),
            1 => self
                .hlines
                .iter()
                .filter(|&&line| line > current_physical)
                .nth(ordinal - 1),
            _ => self.hlines.get(ordinal - 1),
        }
        .copied()
        .ok_or("TBLFM hline reference is outside the table")?;
        let offset = if offset == 0 {
            match edge {
                RangeEdge::Start => 1,
                RangeEdge::End => -1,
                RangeEdge::Field if direction != 0 => 1,
                RangeEdge::Field => return Err("A hline is not a table field".into()),
            }
        } else {
            offset
        };
        let physical = if offset > 0 {
            self.data_rows
                .iter()
                .copied()
                .filter(|&line| line > hline)
                .nth(offset as usize - 1)
        } else {
            self.data_rows
                .iter()
                .rev()
                .copied()
                .filter(|&line| line < hline)
                .nth(offset.unsigned_abs() - 1)
        }
        .ok_or("TBLFM hline offset is outside the table")?;
        Ok(self.data_rows.partition_point(|&line| line < physical) + 1)
    }

    fn number(
        &self,
        row: usize,
        column: usize,
        empty_mode: EmptyMode,
        duration: bool,
    ) -> Result<Option<f64>, String> {
        let value = self.rows[row].cells[column].trim();
        if value.is_empty() {
            return Ok(match empty_mode {
                EmptyMode::Skip => None,
                EmptyMode::Nan => Some(f64::NAN),
                EmptyMode::Zero => Some(0.0),
            });
        }
        match value
            .parse::<f64>()
            .ok()
            .or_else(|| duration.then(|| parse_duration(value)).flatten())
        {
            Some(number) if number.is_nan() => Ok(Some(number)),
            Some(number) => Ok(Some(checked_number(number)?)),
            None if matches!(empty_mode, EmptyMode::Zero) => Ok(Some(0.0)),
            None => Err(format!(
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

    fn named_number(
        &self,
        name: &str,
        row: usize,
        empty_mode: EmptyMode,
        duration: bool,
    ) -> Result<f64, String> {
        match self.names.get(name) {
            Some(NamedValue::Constant(value)) => Ok(*value),
            Some(NamedValue::Column(column)) => {
                let physical = self
                    .data_rows
                    .get(row.saturating_sub(1))
                    .ok_or_else(|| format!("Named field ${name} row is outside the table"))?;
                self.number(*physical, *column, empty_mode, duration)?
                    .ok_or_else(|| format!("Empty named field: ${name}"))
            }
            Some(NamedValue::Cell(data_row, column)) => {
                let physical = self
                    .data_rows
                    .get(data_row.saturating_sub(1))
                    .ok_or_else(|| format!("Named field ${name} row is outside the table"))?;
                self.number(*physical, *column, empty_mode, duration)?
                    .ok_or_else(|| format!("Empty named field: ${name}"))
            }
            None => Err(format!("Unknown TBLFM name: ${name}")),
        }
    }

    fn named_column(&self, name: &str) -> Result<usize, String> {
        match self.names.get(name) {
            Some(NamedValue::Column(column)) => Ok(column + 1),
            _ => Err(format!("TBLFM ${name} is not a named column")),
        }
    }

    fn field_number(
        &self,
        reference: CellRef,
        row: usize,
        column: usize,
        empty_mode: EmptyMode,
        duration: bool,
    ) -> Result<f64, String> {
        let (physical, column) = self.resolve(reference, row, column, RangeEdge::Field)?;
        self.number(physical, column, empty_mode, duration)?
            .ok_or("Empty field in TBLFM reference".into())
    }

    fn range_numbers(
        &self,
        first: CellRef,
        last: CellRef,
        row: usize,
        column: usize,
        empty_mode: EmptyMode,
        duration: bool,
    ) -> Result<Vec<f64>, String> {
        let (first_row, first_col) = self.resolve(first, row, column, RangeEdge::Start)?;
        let (last_row, last_col) = self.resolve(last, row, column, RangeEdge::End)?;
        let data_start = self.data_rows.partition_point(|&index| index < first_row);
        let data_end = self.data_rows.partition_point(|&index| index < last_row);
        let mut numbers = Vec::new();
        for data_row in data_start.min(data_end)..=data_start.max(data_end) {
            for column in first_col.min(last_col)..=first_col.max(last_col) {
                if let Some(value) =
                    self.number(self.data_rows[data_row], column, empty_mode, duration)?
                {
                    numbers.push(value);
                }
            }
        }
        Ok(numbers)
    }
}

fn resolve_axis(axis: Axis, current: usize, last: usize) -> Result<usize, String> {
    let invalid = || "TBLFM coordinate is too large".to_owned();
    let current = isize::try_from(current).map_err(|_| invalid())?;
    let last = isize::try_from(last).map_err(|_| invalid())?;
    let value = match axis {
        Axis::Current => current,
        Axis::Absolute(value) => isize::try_from(value).map_err(|_| invalid())?,
        Axis::Relative(delta) => current.checked_add(delta).ok_or_else(invalid)?,
        Axis::First => 1,
        Axis::Last => last,
        Axis::FromFirst(offset) => isize::try_from(offset)
            .map_err(|_| invalid())?
            .checked_add(1)
            .ok_or_else(invalid)?,
        Axis::FromLast(offset) => last
            .checked_sub(isize::try_from(offset).map_err(|_| invalid())?)
            .ok_or_else(invalid)?,
        Axis::Hline { .. } => return Err("A hline is not a table column".into()),
    };
    if value < 1 || value > last {
        return Err(format!("Table reference {value} is outside 1..{last}"));
    }
    Ok(value as usize)
}

fn target_coordinate(
    table: &Table,
    reference: CellRef,
    edge: RangeEdge,
) -> Result<(usize, usize), String> {
    Ok((
        table.resolve_row(reference.row, 1, edge)?,
        resolve_axis(reference.column, 1, table.columns)?,
    ))
}

fn a1_reference(source: &str) -> Option<(CellRef, usize)> {
    let bytes = source.as_bytes();
    let letters = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_alphabetic())
        .count();
    if letters == 0 {
        return None;
    }
    let digits = bytes[letters..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let column = bytes[..letters].iter().try_fold(0_usize, |value, byte| {
        value
            .checked_mul(26)?
            .checked_add(usize::from(byte.to_ascii_uppercase() - b'A' + 1))
    })?;
    let row = source[letters..letters + digits].parse().ok()?;
    Some((
        CellRef {
            row: Axis::Absolute(row),
            column: Axis::Absolute(column),
        },
        letters + digits,
    ))
}

fn roman_hline(source: &str) -> Option<(usize, usize)> {
    let len = source
        .bytes()
        .take_while(|byte| matches!(byte, b'I' | b'V' | b'X' | b'L' | b'C' | b'D' | b'M'))
        .count();
    if len == 0 {
        return None;
    }
    let numeral = |byte| match byte {
        b'I' => 1,
        b'V' => 5,
        b'X' => 10,
        b'L' => 50,
        b'C' => 100,
        b'D' => 500,
        b'M' => 1000,
        _ => 0,
    };
    let bytes = &source.as_bytes()[..len];
    let mut result = 0_isize;
    for (index, &byte) in bytes.iter().enumerate() {
        let value = numeral(byte);
        if bytes
            .get(index + 1)
            .is_some_and(|&next| numeral(next) > value)
        {
            result = result.checked_sub(value)?;
        } else {
            result = result.checked_add(value)?;
        }
    }
    (result > 0).then_some((result as usize, len))
}

fn table_names(
    snapshot: &DocumentSnapshot,
    arena: &crate::org_syntax::BlockArena,
    start: u64,
    rows: &[ParsedRow],
    data_rows: &[usize],
) -> std::collections::HashMap<String, NamedValue> {
    let mut names = std::collections::HashMap::new();
    let table_start = snapshot
        .line_content_range(LineIndex(start))
        .map_or(0, |range| range.start.0);
    for node in arena.nodes() {
        if node.source.start.0 >= table_start || !matches!(node.kind, BlockKind::Keyword) {
            continue;
        }
        let text = snapshot.copy_range(node.content);
        let text = text.trim_start();
        if text
            .get(..12)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+CONSTANTS:"))
        {
            for assignment in text[12..].split_whitespace() {
                if let Some((name, value)) = assignment.split_once('=')
                    && let Ok(value) = value.parse::<f64>()
                    && let Ok(value) = checked_number(value)
                {
                    names.insert(name.to_owned(), NamedValue::Constant(value));
                }
            }
        }
    }
    let nodes = arena.nodes();
    let mut headings = std::collections::HashSet::new();
    if let Some((_, table)) = nodes.iter().enumerate().find(|(_, node)| {
        node.source.start.0 == table_start && matches!(node.kind, BlockKind::TableRow)
    }) {
        let mut parent = table.parent;
        while let Some(id) = parent {
            headings.insert(id);
            parent = nodes[id as usize].parent;
        }
    }
    for node in nodes {
        if node.source.start.0 >= table_start
            || !headings.contains(&node.parent.unwrap_or(u32::MAX))
        {
            continue;
        }
        let BlockKind::Drawer { name } = &node.kind else {
            continue;
        };
        if !name.eq_ignore_ascii_case("PROPERTIES") {
            continue;
        }
        for line in snapshot.copy_range(node.content).lines() {
            if let Some((name, value)) = line
                .trim()
                .strip_prefix(':')
                .and_then(|line| line.split_once(':'))
                && let Ok(value) = value.trim().parse::<f64>()
                && let Ok(value) = checked_number(value)
            {
                names.insert(format!("PROP_{name}"), NamedValue::Constant(value));
            }
        }
    }
    for (physical, row) in rows.iter().enumerate() {
        match row.cells.first().map(|cell| cell.trim()) {
            Some("!") => {
                for (column, name) in row.cells.iter().enumerate().skip(1) {
                    let name = name.trim();
                    if !name.is_empty() {
                        names.insert(name.to_owned(), NamedValue::Column(column));
                    }
                }
            }
            Some("$") => {
                for cell in row.cells.iter().skip(1) {
                    if let Some((name, value)) = cell.trim().split_once('=')
                        && let Ok(value) = value.trim().parse::<f64>()
                        && let Ok(value) = checked_number(value)
                    {
                        names.insert(name.trim().to_owned(), NamedValue::Constant(value));
                    }
                }
            }
            Some("^" | "_") => {
                let above = row.cells[0].trim() == "^";
                let target = if above {
                    data_rows
                        .iter()
                        .copied()
                        .rev()
                        .find(|&line| line < physical)
                } else {
                    data_rows.iter().copied().find(|&line| line > physical)
                };
                if let Some(target) = target {
                    let data_row = data_rows.partition_point(|&line| line < target) + 1;
                    for (column, name) in row.cells.iter().enumerate().skip(1) {
                        let name = name.trim();
                        if !name.is_empty() {
                            names.insert(name.to_owned(), NamedValue::Cell(data_row, column));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    names
}

fn make_table(
    snapshot: &DocumentSnapshot,
    arena: &crate::org_syntax::BlockArena,
    start: u64,
    mut rows: Vec<ParsedRow>,
) -> Result<Table, String> {
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
    let hlines = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| row.separator.then_some(index))
        .collect();
    let names = table_names(snapshot, arena, start, &rows, &data_rows);
    Ok(Table {
        rows,
        data_rows,
        columns,
        first_hline,
        hlines,
        names,
        remotes: std::collections::HashMap::new(),
    })
}

fn named_tables(
    snapshot: &DocumentSnapshot,
    arena: &crate::org_syntax::BlockArena,
    table_lines: &std::collections::HashSet<u64>,
    keyword_lines: &std::collections::HashSet<u64>,
) -> Result<std::collections::HashMap<String, std::sync::Arc<Table>>, String> {
    let mut tables = std::collections::HashMap::new();
    let mut seen_entries = std::collections::HashSet::new();
    let mut line = 0;
    while line < snapshot.len_lines() {
        if table_line_text(snapshot, line, table_lines).is_none() {
            line += 1;
            continue;
        }
        let start = line;
        let mut rows = Vec::new();
        while let Some(text) = table_line_text(snapshot, line, table_lines) {
            rows.push(parse_source_row(&text, DocumentFormat::Org));
            line += 1;
        }
        let name = affiliated_table_name(snapshot, start, keyword_lines);
        let entry_id = table_entry_id(snapshot, arena, start);
        if name.is_none() && entry_id.is_none() {
            continue;
        }
        let table = std::sync::Arc::new(make_table(snapshot, arena, start, rows)?);
        if let Some(name) = name {
            tables.insert(name, table.clone());
        }
        if let Some((heading, id)) = entry_id
            && seen_entries.insert(heading)
        {
            tables.insert(id, table);
        }
    }
    Ok(tables)
}

fn affiliated_table_name(
    snapshot: &DocumentSnapshot,
    start: u64,
    keyword_lines: &std::collections::HashSet<u64>,
) -> Option<String> {
    let mut line = start;
    while line > 0 {
        line -= 1;
        let text = structural_line_text(snapshot, line, keyword_lines)?;
        let text = text.trim_start();
        if text
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+NAME:"))
        {
            return Some(text[7..].trim().to_owned());
        }
        let upper = text.to_ascii_uppercase();
        if !upper.starts_with("#+CAPTION:") && !upper.starts_with("#+ATTR_") {
            return None;
        }
    }
    None
}

fn table_entry_id(
    snapshot: &DocumentSnapshot,
    arena: &crate::org_syntax::BlockArena,
    start: u64,
) -> Option<(u32, String)> {
    let table_start = snapshot.line_content_range(LineIndex(start)).ok()?.start.0;
    let nodes = arena.nodes();
    let table = nodes.iter().find(|node| {
        node.source.start.0 == table_start && matches!(node.kind, BlockKind::TableRow)
    })?;
    let mut parent = table.parent;
    let heading = loop {
        let id = parent?;
        let node = &nodes[id as usize];
        if matches!(node.kind, BlockKind::Heading { .. }) {
            break id;
        }
        parent = node.parent;
    };
    for node in nodes {
        if node.parent != Some(heading) || node.source.start.0 >= table_start {
            continue;
        }
        if !matches!(&node.kind, BlockKind::Drawer { name } if name.eq_ignore_ascii_case("PROPERTIES"))
        {
            continue;
        }
        for line in snapshot.copy_range(node.content).lines() {
            if let Some((name, value)) = line
                .trim()
                .strip_prefix(':')
                .and_then(|line| line.split_once(':'))
                && name.eq_ignore_ascii_case("ID")
                && !value.trim().is_empty()
            {
                return Some((heading, value.trim().to_owned()));
            }
        }
    }
    None
}

pub(in crate::editor) fn recalculate(
    snapshot: &DocumentSnapshot,
    caret: ByteOffset,
    newline: &str,
    all_rows: bool,
) -> Result<Option<TableAlignment>, String> {
    recalculate_impl(snapshot, caret, newline, all_rows, false)
}

pub(in crate::editor) fn recalculate_iteratively(
    snapshot: &DocumentSnapshot,
    caret: ByteOffset,
    newline: &str,
) -> Result<Option<TableAlignment>, String> {
    recalculate_impl(snapshot, caret, newline, true, true)
}

pub(in crate::editor) fn recalculate_marked_row_on_navigation(
    snapshot: &DocumentSnapshot,
    context: &EditorCommandContext,
    caret: ByteOffset,
    newline: &str,
    navigation: TableNavigation,
) -> Result<Option<TableAlignment>, String> {
    if navigation == TableNavigation::Stay || context.format != Some(DocumentFormat::Org) {
        return Ok(None);
    }
    let row = parse_source_row(
        &snapshot.copy_range(context.line_range),
        DocumentFormat::Org,
    );
    if row.cells.first().is_none_or(|cell| cell.trim() != "#") {
        return Ok(None);
    }
    let Some(calculated) = recalculate(snapshot, caret, newline, false)? else {
        return Ok(None);
    };
    let mut source = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
    source.replace_range(
        calculated.range.start.0 as usize..calculated.range.end.0 as usize,
        &calculated.replacement,
    );
    let temporary =
        DocumentSnapshot::from_utf8(source.into_bytes()).map_err(|error| error.to_string())?;
    let context =
        EditorCommandContext::at(Path::new("temporary.org"), &temporary, calculated.caret)
            .ok_or("TBLFM row disappeared after recalculation")?;
    let aligned = align_table(&temporary, &context, newline, navigation)
        .ok_or("TBLFM row could not be aligned after recalculation")?;
    if aligned.range.start != calculated.range.start {
        return Err("TBLFM table moved during recalculation".into());
    }
    Ok(Some(TableAlignment {
        range: calculated.range,
        replacement: aligned.replacement,
        caret: aligned.caret,
    }))
}

pub(in crate::editor) fn recalculate_buffer_tables(
    snapshot: &DocumentSnapshot,
    caret: ByteOffset,
    newline: &str,
    iterate: bool,
) -> Result<Option<TableAlignment>, String> {
    let original = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
    let mut source = original.clone();
    let max_passes = if iterate { 100 } else { 1 };
    let mut found_table = false;
    let mut converged = false;
    for _pass in 0..max_passes {
        let before = source.clone();
        let current = DocumentSnapshot::from_utf8(source.as_bytes().to_vec())
            .map_err(|error| error.to_string())?;
        let arena = parse(&current);
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
        let mut changes = Vec::new();
        for line in 1..current.len_lines() {
            if table_line_text(&current, line - 1, &table_lines).is_none()
                || formula_line(&current, line, &keyword_lines).is_none()
            {
                continue;
            }
            found_table = true;
            let caret = current
                .line_content_range(LineIndex(line))
                .map_err(|error| error.to_string())?
                .start;
            if let Some(change) = recalculate(&current, caret, newline, true)? {
                changes.push(change);
            }
        }
        for change in changes.into_iter().rev() {
            source.replace_range(
                change.range.start.0 as usize..change.range.end.0 as usize,
                &change.replacement,
            );
        }
        if !iterate || source == before {
            converged = true;
            break;
        }
    }
    if !found_table {
        return Ok(None);
    }
    if !converged {
        return Err("TBLFM buffer iteration did not converge after 100 passes".into());
    }
    let mut prefix = original
        .bytes()
        .zip(source.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !original.is_char_boundary(prefix) || !source.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = original
        .as_bytes()
        .iter()
        .rev()
        .zip(source.as_bytes().iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(original.len() - prefix)
        .min(source.len() - prefix);
    while !original.is_char_boundary(original.len() - suffix)
        || !source.is_char_boundary(source.len() - suffix)
    {
        suffix -= 1;
    }
    let line = snapshot
        .line_index_at(caret)
        .map_err(|error| error.to_string())?;
    let old_line = snapshot
        .line_content_range(line)
        .map_err(|error| error.to_string())?;
    let new_snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec())
        .map_err(|error| error.to_string())?;
    let new_line = new_snapshot
        .line_content_range(line)
        .map_err(|error| error.to_string())?;
    let mut new_caret =
        (new_line.start.0 + caret.0.saturating_sub(old_line.start.0)).min(new_line.end.0) as usize;
    while !source.is_char_boundary(new_caret) {
        new_caret -= 1;
    }
    let caret = ByteOffset(new_caret as u64);
    Ok(Some(TableAlignment {
        range: ByteRange::new(prefix as u64, (original.len() - suffix) as u64),
        replacement: source[prefix..source.len() - suffix].to_owned(),
        caret,
    }))
}

fn recalculate_impl(
    snapshot: &DocumentSnapshot,
    caret: ByteOffset,
    newline: &str,
    all_rows: bool,
    iterate: bool,
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
    let mut formula_end = end;
    let mut formula_lines = Vec::new();
    while let Some(source) = formula_line(snapshot, formula_end, &keyword_lines) {
        formula_lines.push(source);
        formula_end += 1;
    }
    let selected_line = if (end..formula_end).contains(&caret_line) {
        (caret_line - end) as usize
    } else {
        0
    };
    let mut formulas = formula_lines
        .get(selected_line)
        .into_iter()
        .flat_map(|source| split_top_level(source, "::"))
        .filter(|part| !part.trim().is_empty())
        .map(|part| parse_formula(part.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    if formulas.is_empty() {
        return Ok(None);
    }
    let mut table = make_table(snapshot, &arena, start, rows)?;
    if formulas
        .iter()
        .any(|formula| formula.expression.to_ascii_lowercase().contains("remote"))
    {
        table.remotes = named_tables(snapshot, &arena, &table_lines, &keyword_lines)?;
    }
    for formula in &mut formulas {
        if let Target::Named(name) = &formula.target {
            formula.target = match table.names.get(name) {
                Some(NamedValue::Column(column)) => Target::Column(Axis::Absolute(column + 1)),
                Some(NamedValue::Cell(row, column)) => {
                    Target::Cell(Axis::Absolute(*row), Axis::Absolute(column + 1))
                }
                Some(NamedValue::Constant(_)) => {
                    return Err(format!("Constant ${name} cannot be a TBLFM target"));
                }
                None => return Err(format!("Unknown TBLFM target: ${name}")),
            };
        }
    }
    let mut field_targets = std::collections::HashSet::new();
    for formula in &formulas {
        match formula.target {
            Target::Cell(row, column) => {
                let data_row = table.resolve_row(row, 1, RangeEdge::Field)?;
                let column = resolve_axis(column, 1, table.columns)?;
                field_targets.insert((data_row, column));
            }
            Target::Row(row) => {
                let data_row = table.resolve_row(row, 1, RangeEdge::Field)?;
                for column in 1..=table.columns {
                    field_targets.insert((data_row, column));
                }
            }
            Target::Range(first, last) => {
                let (start_row, start_col) = target_coordinate(&table, first, RangeEdge::Start)?;
                let (end_row, end_col) = target_coordinate(&table, last, RangeEdge::End)?;
                for row in start_row.min(end_row)..=start_row.max(end_row) {
                    for column in start_col.min(end_col)..=start_col.max(end_col) {
                        field_targets.insert((row, column));
                    }
                }
            }
            Target::Column(_) => {}
            Target::Named(_) => unreachable!("named targets are resolved before calculation"),
        }
    }
    let mut columns = formulas
        .iter()
        .filter_map(|formula| match formula.target {
            Target::Column(axis) => Some((resolve_axis(axis, 1, table.columns), formula)),
            Target::Row(_) | Target::Cell(_, _) | Target::Range(_, _) | Target::Named(_) => None,
        })
        .map(|(column, formula)| Ok((column?, formula)))
        .collect::<Result<Vec<_>, String>>()?;
    columns.sort_by_key(|(column, _)| *column);
    let special_rows = table.rows.iter().any(|row| {
        row.cells
            .first()
            .is_some_and(|cell| matches!(cell.trim(), "!" | "^" | "_" | "$" | "#" | "*" | "/"))
    });
    let max_passes = if iterate { 100 } else { 1 };
    let mut converged = false;
    for _pass in 0..max_passes {
        let before = table
            .rows
            .iter()
            .map(|row| row.cells.clone())
            .collect::<Vec<_>>();
        for data_row in 1..=table.data_rows.len() {
            let physical = table.data_rows[data_row - 1];
            if !all_rows && caret_line < end && caret_line != start + physical as u64 {
                continue;
            }
            if all_rows && table.first_hline.is_some_and(|hline| physical < hline) {
                continue;
            }
            let marker = table.rows[physical].cells[0].trim();
            let apply_columns = !all_rows || !special_rows || matches!(marker, "#" | "*");
            if apply_columns && table.first_hline.is_none_or(|hline| physical > hline) {
                for &(column, formula) in &columns {
                    if !field_targets.contains(&(data_row, column)) {
                        apply_formula(&mut table, physical, data_row, column, formula)?;
                    }
                }
            }
            for formula in &formulas {
                if let Target::Row(row) = formula.target
                    && table.resolve_row(row, 1, RangeEdge::Field)? == data_row
                {
                    for column in 1..=table.columns {
                        apply_formula(&mut table, physical, data_row, column, formula)?;
                    }
                }
                if let Target::Cell(row, column) = formula.target
                    && table.resolve_row(row, 1, RangeEdge::Field)? == data_row
                {
                    let column = resolve_axis(column, 1, table.columns)?;
                    apply_formula(&mut table, physical, data_row, column, formula)?;
                }
                if let Target::Range(first, last) = formula.target {
                    let (start_row, start_col) =
                        target_coordinate(&table, first, RangeEdge::Start)?;
                    let (end_row, end_col) = target_coordinate(&table, last, RangeEdge::End)?;
                    if (start_row.min(end_row)..=start_row.max(end_row)).contains(&data_row) {
                        for column in start_col.min(end_col)..=start_col.max(end_col) {
                            apply_formula(&mut table, physical, data_row, column, formula)?;
                        }
                    }
                }
            }
        }
        if !iterate
            || table
                .rows
                .iter()
                .zip(before)
                .all(|(row, previous)| row.cells == previous)
        {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err("TBLFM iteration did not converge after 100 passes".into());
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

fn top_level_separator(source: &str, delimiter: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut quoted = false;
    let mut depth = 0_usize;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if quoted => {
                index = (index + 2).min(bytes.len());
                continue;
            }
            b'"' => quoted = !quoted,
            b'(' if !quoted => depth += 1,
            b')' if !quoted => depth = depth.saturating_sub(1),
            _ => {}
        }
        if !quoted
            && depth == 0
            && source.is_char_boundary(index)
            && source[index..].starts_with(delimiter)
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn split_top_level<'a>(source: &'a str, delimiter: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut rest = source;
    while let Some(index) = top_level_separator(rest, delimiter) {
        parts.push(&rest[..index]);
        rest = &rest[index + delimiter.len()..];
    }
    parts.push(rest);
    parts
}

fn parse_formula(source: &str) -> Result<Formula, String> {
    let (target_source, rhs) = source
        .split_once('=')
        .ok_or_else(|| format!("Invalid TBLFM formula: {source}"))?;
    let target_source = target_source.trim();
    let target = if let Some(name) = target_source.strip_prefix('$')
        && !name.is_empty()
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Target::Named(name.to_owned())
    } else {
        let mut parser = Parser::new(target_source, None);
        let reference = parser.parse_reference()?;
        let last = parser
            .eat("..")
            .then(|| parser.parse_reference())
            .transpose()?;
        parser.finish()?;
        if let Some(last) = last {
            Target::Range(reference, last)
        } else if target_source.starts_with('@') {
            if target_source.contains('$') {
                Target::Cell(reference.row, reference.column)
            } else {
                Target::Row(reference.row)
            }
        } else if target_source.starts_with('$') {
            Target::Column(reference.column)
        } else {
            Target::Cell(reference.row, reference.column)
        }
    };
    let valid_column = |axis| {
        matches!(
            axis,
            Axis::Absolute(_) | Axis::First | Axis::Last | Axis::FromFirst(_) | Axis::FromLast(_)
        )
    };
    let valid_row = |axis| valid_column(axis) || matches!(axis, Axis::Hline { .. });
    if !match target {
        Target::Column(column) => valid_column(column),
        Target::Row(row) => valid_row(row),
        Target::Cell(row, column) => valid_row(row) && valid_column(column),
        Target::Range(first, last) => {
            valid_row(first.row)
                && valid_column(first.column)
                && valid_row(last.row)
                && valid_column(last.column)
        }
        Target::Named(_) => true,
    } {
        return Err(format!(
            "Unsupported TBLFM target: {}",
            target_source.trim()
        ));
    }
    let (expression, mode) =
        top_level_separator(rhs, ";").map_or((rhs, ""), |index| (&rhs[..index], &rhs[index + 1..]));
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("Empty TBLFM expression".into());
    }
    let mode = mode.trim();
    let empty_mode = if mode.contains('N') {
        EmptyMode::Zero
    } else if mode.contains('E') {
        EmptyMode::Nan
    } else {
        EmptyMode::Skip
    };
    let duration = match (mode.contains('T'), mode.contains('U'), mode.contains('t')) {
        (true, false, false) => Some(DurationFormat::HoursMinutesSeconds),
        (false, true, false) => Some(DurationFormat::HoursMinutes),
        (false, false, true) => Some(DurationFormat::DecimalHours),
        (false, false, false) => None,
        _ => return Err("Conflicting TBLFM duration modes".into()),
    };
    if mode.contains('D') && mode.contains('R') {
        return Err("Conflicting TBLFM angle modes".into());
    }
    let radians = mode.contains('R');
    let remainder = mode.replace(['N', 'E', 'T', 'U', 't', 'D', 'R'], "");
    let number_format = if remainder.trim().is_empty() {
        NumberFormat::Default
    } else {
        let spec = remainder.trim();
        let (kind, digits) = if let Some(spec) = spec.strip_prefix("%.") {
            let (digits, kind) = spec.split_at(spec.len().saturating_sub(1));
            (kind, digits)
        } else {
            let (kind, digits) = spec.split_at(1);
            (kind, digits)
        };
        let digits = digits
            .parse::<isize>()
            .map_err(|_| format!("Invalid TBLFM precision: {mode}"))?;
        if digits.unsigned_abs() > 15 {
            return Err("TBLFM floating-point display precision must be at most 15".into());
        }
        let digits = if digits < 0 && kind != "f" {
            12_usize.saturating_sub(digits.unsigned_abs())
        } else {
            digits.unsigned_abs()
        };
        match (spec.starts_with('%'), kind) {
            (true, "f") => NumberFormat::PrintfFixed(digits),
            (true, "e") => NumberFormat::PrintfScientific(digits),
            (false, "f") => NumberFormat::Fixed(digits),
            (false, "s") => NumberFormat::Scientific(digits),
            (false, "e") => NumberFormat::Engineering(digits),
            (false, "n") => NumberFormat::Normal(digits),
            _ => return Err(format!("Unsupported TBLFM mode: {mode}")),
        }
    };
    Ok(Formula {
        target,
        expression: expression.into(),
        empty_mode,
        number_format,
        duration,
        radians,
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
        Some(EvalContext {
            table,
            row,
            column,
            empty_mode: formula.empty_mode,
            duration: formula.duration.is_some(),
            radians: formula.radians,
        }),
    )
    .parse()?;
    let output = match value {
        Value::Text(text) => text,
        Value::Nan => "nan".into(),
        Value::Number(value) => {
            let value = checked_number(value)?;
            if let Some(duration) = formula.duration {
                format_duration(value, duration)
            } else {
                format_number(value, formula.number_format)
            }
        }
        Value::Vector(_) => return Err("Range must be used inside a vector function".into()),
    };
    table.rows[physical].cells[column - 1] = output;
    Ok(())
}

mod calc;
mod rewrite;
use calc::{EvalContext, Parser, Value};
pub(super) use rewrite::{AxisEdit, adapt_formula_lines};

#[cfg(test)]
mod tests;
