use super::*;

#[derive(Clone)]
pub(super) enum Value {
    Number(f64),
    Vector(Vec<f64>),
    Text(String),
    Nan,
}

impl Value {
    fn scalar(value: f64) -> Result<Self, String> {
        if value.is_nan() {
            Ok(Self::Nan)
        } else {
            Ok(Self::Number(checked_number(value)?))
        }
    }

    fn number(self) -> Result<f64, String> {
        match self {
            Self::Number(value) => Ok(value),
            Self::Vector(_) => Err("Range must be used inside a vector function".into()),
            Self::Text(_) => Err("Text cannot be used as a number in TBLFM".into()),
            Self::Nan => Ok(f64::NAN),
        }
    }

    fn truthy(self) -> Result<bool, String> {
        match self {
            Self::Number(value) => Ok(value != 0.0),
            Self::Text(value) => Ok(!value.is_empty()),
            Self::Vector(_) => Err("Range cannot be used as a condition".into()),
            Self::Nan => Err("nan cannot be used as a TBLFM condition".into()),
        }
    }
}

fn eval_function(name: &str, args: &[Value], radians: bool) -> Result<Value, String> {
    if let ("typeof", [Value::Nan]) = (name, args) {
        return Value::scalar(12.0);
    }
    if !matches!(name, "string" | "typeof") && args.iter().any(|value| matches!(value, Value::Nan))
    {
        return Ok(Value::Nan);
    }
    if !matches!(name, "vcount" | "vlen")
        && args.iter().any(|value| matches!(value, Value::Vector(values) if values.iter().any(|value| value.is_nan())))
    {
        return Ok(Value::Nan);
    }
    if let ("string", [value]) = (name, args) {
        return Ok(Value::Text(match value {
            Value::Text(text) => text.clone(),
            Value::Number(value) => value.to_string(),
            Value::Vector(_) => return Err("Cannot convert a range to text".into()),
            Value::Nan => "nan".into(),
        }));
    }
    let degrees = |value: f64| if radians { value } else { value.to_radians() };
    let angle = |value: f64| if radians { value } else { value.to_degrees() };
    let result = match (name, args) {
        ("vsum", [Value::Vector(values)]) => values.iter().sum::<f64>(),
        ("vprod", [Value::Vector(values)]) => values
            .iter()
            .try_fold(1.0, |product, value| checked_number(product * value))?,
        ("vcount" | "vlen", [Value::Vector(values)]) => values.len() as f64,
        ("vmean", [Value::Vector(values)]) if !values.is_empty() => {
            values.iter().sum::<f64>() / values.len() as f64
        }
        ("vmin", [Value::Vector(values)]) if !values.is_empty() => {
            values.iter().copied().fold(f64::INFINITY, f64::min)
        }
        ("vmax", [Value::Vector(values)]) if !values.is_empty() => {
            values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        }
        ("vmedian", [Value::Vector(values)]) if !values.is_empty() => {
            let mut sorted = values.clone();
            sorted.sort_by(f64::total_cmp);
            let mid = sorted.len() / 2;
            if sorted.len() % 2 == 0 {
                (sorted[mid - 1] + sorted[mid]) / 2.0
            } else {
                sorted[mid]
            }
        }
        ("vvar" | "vpvar" | "vsdev" | "vpsdev", [Value::Vector(values)])
            if values.len()
                >= if matches!(name, "vvar" | "vsdev") {
                    2
                } else {
                    1
                } =>
        {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let variance = values
                .iter()
                .map(|value| (value - mean).powi(2))
                .sum::<f64>()
                / (values.len() - usize::from(matches!(name, "vvar" | "vsdev"))) as f64;
            if matches!(name, "vsdev" | "vpsdev") {
                variance.sqrt()
            } else {
                variance
            }
        }
        ("abs", [Value::Number(value)]) => value.abs(),
        ("sqrt", [Value::Number(value)]) => value.sqrt(),
        ("sqr", [Value::Number(value)]) => value * value,
        ("exp", [Value::Number(value)]) => value.exp(),
        ("ln", [Value::Number(value)]) => value.ln(),
        ("log10", [Value::Number(value)]) => value.log10(),
        ("log", [Value::Number(value), Value::Number(base)]) => value.log(*base),
        ("sin", [Value::Number(value)]) => degrees(*value).sin(),
        ("cos", [Value::Number(value)]) => degrees(*value).cos(),
        ("tan", [Value::Number(value)]) => degrees(*value).tan(),
        ("arcsin", [Value::Number(value)]) => angle(value.asin()),
        ("arccos", [Value::Number(value)]) => angle(value.acos()),
        ("arctan", [Value::Number(value)]) => angle(value.atan()),
        ("floor", [Value::Number(value)]) => value.floor(),
        ("ceil" | "ceiling", [Value::Number(value)]) => value.ceil(),
        ("round", [Value::Number(value)]) => value.round(),
        ("trunc", [Value::Number(value)]) => value.trunc(),
        ("min", [Value::Number(left), Value::Number(right)]) => left.min(*right),
        ("max", [Value::Number(left), Value::Number(right)]) => left.max(*right),
        _ => return Err(format!("Unsupported TBLFM function or argument: {name}")),
    };
    if result.is_nan() {
        return Err(format!("TBLFM {name} has no real-valued result"));
    }
    Value::scalar(result)
}

#[derive(Clone, Copy)]
pub(super) struct EvalContext<'a> {
    pub(super) table: &'a Table,
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) empty_mode: EmptyMode,
    pub(super) duration: bool,
    pub(super) radians: bool,
}

pub(super) struct Parser<'a> {
    source: &'a str,
    position: usize,
    context: Option<EvalContext<'a>>,
}

impl<'a> Parser<'a> {
    pub(super) fn new(source: &'a str, context: Option<EvalContext<'a>>) -> Self {
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
    pub(super) fn eat(&mut self, text: &str) -> bool {
        self.space();
        if self.rest().starts_with(text) {
            self.position += text.len();
            true
        } else {
            false
        }
    }
    pub(super) fn finish(&mut self) -> Result<(), String> {
        self.space();
        if self.rest().is_empty() {
            Ok(())
        } else {
            Err(format!("Unsupported TBLFM syntax near: {}", self.rest()))
        }
    }
    pub(super) fn parse(&mut self) -> Result<Value, String> {
        let value = self.logical_or()?;
        self.finish()?;
        Ok(value)
    }
    fn logical_or(&mut self) -> Result<Value, String> {
        let mut value = self.logical_and()?;
        while self.eat("||") {
            let right = self.logical_and()?;
            value = Value::scalar(f64::from(value.truthy()? || right.truthy()?))?;
        }
        Ok(value)
    }
    fn logical_and(&mut self) -> Result<Value, String> {
        let mut value = self.compare()?;
        while self.eat("&&") {
            let right = self.compare()?;
            value = Value::scalar(f64::from(value.truthy()? && right.truthy()?))?;
        }
        Ok(value)
    }
    fn compare(&mut self) -> Result<Value, String> {
        let mut value = self.add()?;
        loop {
            let operator = ["<=", ">=", "!=", "==", "=", "<", ">"]
                .into_iter()
                .find(|operator| self.eat(operator));
            let Some(operator) = operator else { break };
            let right = self.add()?;
            let ordering = match (&value, &right) {
                (Value::Number(left), Value::Number(right)) => left.partial_cmp(right),
                (Value::Text(left), Value::Text(right)) => Some(left.cmp(right)),
                _ => return Err("Cannot compare text and numbers in TBLFM".into()),
            };
            let result = match operator {
                "=" | "==" => ordering == Some(std::cmp::Ordering::Equal),
                "!=" => ordering != Some(std::cmp::Ordering::Equal),
                "<" => ordering == Some(std::cmp::Ordering::Less),
                ">" => ordering == Some(std::cmp::Ordering::Greater),
                "<=" => ordering.is_some_and(|order| order != std::cmp::Ordering::Greater),
                ">=" => ordering.is_some_and(|order| order != std::cmp::Ordering::Less),
                _ => unreachable!(),
            };
            value = Value::scalar(f64::from(result))?;
        }
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
        loop {
            if self.eat("/") {
                let right = self.multiply()?.number()?;
                if right == 0.0 {
                    return Err("Division by zero in TBLFM".into());
                }
                value = Value::scalar(value.number()? / right)?;
            } else if self.eat("%") {
                let right = self.multiply()?.number()?;
                if right == 0.0 {
                    return Err("Modulo by zero in TBLFM".into());
                }
                value = Value::scalar(value.number()?.rem_euclid(right))?;
            } else {
                break;
            }
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
        if self.eat("!") {
            return Value::scalar(f64::from(!self.unary()?.truthy()?));
        }
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
        loop {
            if !self.rest().trim_start().starts_with("!=") && self.eat("!") {
                let n = value.number()?;
                if n.fract() != 0.0 || !(0.0..=18.0).contains(&n) {
                    return Err("TBLFM factorial needs an integer from 0 to 18".into());
                }
                value = Value::scalar((1..=n as u64).product::<u64>() as f64)?;
            } else if self.postfix_percent() {
                self.position += 1;
                value = Value::scalar(value.number()? / 100.0)?;
            } else {
                break;
            }
        }
        if self.eat("^") {
            let result = value.number()?.powf(self.unary()?.number()?);
            if result.is_nan() {
                return Err("TBLFM power has no real-valued result".into());
            }
            value = Value::scalar(result)?;
        }
        Ok(value)
    }
    fn postfix_percent(&mut self) -> bool {
        self.space();
        let Some(rest) = self.rest().strip_prefix('%') else {
            return false;
        };
        rest.trim_start().as_bytes().first().is_none_or(|byte| {
            matches!(
                byte,
                b'*' | b'/' | b'+' | b'-' | b'^' | b')' | b',' | b'<' | b'>' | b'=' | b'&' | b'|'
            )
        })
    }
    fn atom(&mut self) -> Result<Value, String> {
        self.space();
        if self.rest().starts_with("@#") && !self.rest()[2..].starts_with('$') {
            self.position += 2;
            return Value::scalar(self.context.ok_or("Reference outside table")?.row as f64);
        }
        if self.rest().starts_with("$#") {
            self.position += 2;
            return Value::scalar(self.context.ok_or("Reference outside table")?.column as f64);
        }
        if self.eat("(") {
            let value = self.logical_or()?;
            if !self.eat(")") {
                return Err("Missing ')' in TBLFM".into());
            }
            return Ok(value);
        }
        if self.eat("\"") {
            let start = self.position;
            while let Some(byte) = self.source.as_bytes().get(self.position) {
                if *byte == b'\\' {
                    self.position = (self.position + 2).min(self.source.len());
                } else if *byte == b'"' {
                    break;
                } else {
                    self.position += 1;
                }
            }
            if self.position == self.source.len() {
                return Err("Unclosed string in TBLFM".into());
            }
            let quoted = &self.source[start..self.position];
            self.position += 1;
            if let Some(context) = self.context
                && (quoted.starts_with(['$', '@']) || a1_reference(quoted).is_some())
            {
                let mut reference = Parser::new(quoted, None);
                if let Ok(field) = reference.parse_reference()
                    && reference.finish().is_ok()
                {
                    let (row, column) = context.table.resolve(
                        field,
                        context.row,
                        context.column,
                        RangeEdge::Field,
                    )?;
                    let value = context.table.rows[row].cells[column].trim();
                    return Ok(Value::Text(
                        if value.is_empty() && matches!(context.empty_mode, EmptyMode::Nan) {
                            "nan".into()
                        } else {
                            value.to_owned()
                        },
                    ));
                }
            }
            return Ok(Value::Text(quoted.replace("\\\"", "\"")));
        }
        if let Some(source) = self.rest().strip_prefix('$')
            && source
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
        {
            let len = source
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                .count();
            let name = &source[..len];
            let context = self.context.ok_or("Reference outside table")?;
            self.position += len + 1;
            if self.eat("..") {
                if !self.eat("$") {
                    return Err("Named range needs a second named column".into());
                }
                let len = self
                    .rest()
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                let last_name = &self.rest()[..len];
                self.position += len;
                let first = CellRef {
                    row: Axis::Current,
                    column: Axis::Absolute(context.table.named_column(name)?),
                };
                let last = CellRef {
                    row: Axis::Current,
                    column: Axis::Absolute(context.table.named_column(last_name)?),
                };
                return Ok(Value::Vector(context.table.range_numbers(
                    first,
                    last,
                    context.row,
                    context.column,
                    context.empty_mode,
                    context.duration,
                )?));
            }
            return Value::scalar(context.table.named_number(
                name,
                context.row,
                context.empty_mode,
                context.duration,
            )?);
        }
        if self.rest().starts_with(['$', '@'])
            || a1_reference(self.rest())
                .is_some_and(|(_, len)| !self.rest()[len..].trim_start().starts_with('('))
        {
            let first = self.parse_reference()?;
            let context = self.context.ok_or("Reference outside table")?;
            if self.eat("..") {
                let last = if matches!(first.row, Axis::Hline { .. })
                    && roman_hline(self.rest()).is_some()
                {
                    let row = self.axis()?;
                    let column = if self.eat("$") {
                        self.axis()?
                    } else {
                        first.column
                    };
                    CellRef { row, column }
                } else {
                    self.parse_reference()?
                };
                return Ok(Value::Vector(context.table.range_numbers(
                    first,
                    last,
                    context.row,
                    context.column,
                    context.empty_mode,
                    context.duration,
                )?));
            }
            return Value::scalar(context.table.field_number(
                first,
                context.row,
                context.column,
                context.empty_mode,
                context.duration,
            )?);
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
                return match name.as_str() {
                    "pi" => Value::scalar(std::f64::consts::PI),
                    "e" => Value::scalar(std::f64::consts::E),
                    "nan" => Ok(Value::Nan),
                    _ => Ok(Value::Text(name)),
                };
            }
            if name == "remote" {
                self.space();
                let context = self.context.ok_or("Reference outside table")?;
                let table_name = if self.rest().starts_with(['$', '@']) {
                    let reference = self.parse_reference()?;
                    let (row, column) = context.table.resolve(
                        reference,
                        context.row,
                        context.column,
                        RangeEdge::Field,
                    )?;
                    context.table.rows[row].cells[column].trim().to_owned()
                } else {
                    let len = self
                        .rest()
                        .bytes()
                        .take_while(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                        })
                        .count();
                    if len == 0 {
                        return Err("Missing remote table name".into());
                    }
                    let name = self.rest()[..len].to_owned();
                    self.position += len;
                    name
                };
                if !self.eat(",") {
                    return Err("Missing ',' in remote table reference".into());
                }
                if let Some(source) = self.rest().strip_prefix('$')
                    && source
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic)
                {
                    let len = source
                        .bytes()
                        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                        .count();
                    let name = &source[..len];
                    let table = context
                        .table
                        .remotes
                        .get(&table_name)
                        .ok_or_else(|| format!("Unknown remote table: {table_name}"))?;
                    self.position += len + 1;
                    if self.eat("..") {
                        if !self.eat("$") {
                            return Err("Remote named range needs a second named column".into());
                        }
                        let len = self
                            .rest()
                            .bytes()
                            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                            .count();
                        let last_name = &self.rest()[..len];
                        self.position += len;
                        if !self.eat(")") {
                            return Err("Missing ')' in remote named range".into());
                        }
                        let first = CellRef {
                            row: Axis::Current,
                            column: Axis::Absolute(table.named_column(name)?),
                        };
                        let last = CellRef {
                            row: Axis::Current,
                            column: Axis::Absolute(table.named_column(last_name)?),
                        };
                        return Ok(Value::Vector(table.range_numbers(
                            first,
                            last,
                            context.row,
                            context.column,
                            context.empty_mode,
                            context.duration,
                        )?));
                    }
                    if !self.eat(")") {
                        return Err("Missing ')' in remote table reference".into());
                    }
                    return Value::scalar(table.named_number(
                        name,
                        context.row,
                        context.empty_mode,
                        context.duration,
                    )?);
                }
                let first = self.parse_reference()?;
                let last = self.eat("..").then(|| self.parse_reference()).transpose()?;
                if !self.eat(")") {
                    return Err("Missing ')' in remote table reference".into());
                }
                let table = context
                    .table
                    .remotes
                    .get(&table_name)
                    .ok_or_else(|| format!("Unknown remote table: {table_name}"))?;
                return match last {
                    Some(last) => Ok(Value::Vector(table.range_numbers(
                        first,
                        last,
                        context.row,
                        context.column,
                        context.empty_mode,
                        context.duration,
                    )?)),
                    None => Value::scalar(table.field_number(
                        first,
                        context.row,
                        context.column,
                        context.empty_mode,
                        context.duration,
                    )?),
                };
            }
            if name == "if" {
                let condition = self.logical_or()?.truthy()?;
                if !self.eat(",") {
                    return Err("Missing ',' after TBLFM condition".into());
                }
                let chosen = if condition {
                    let value = self.logical_or()?;
                    if !self.eat(",") {
                        return Err("Missing second ',' in TBLFM if".into());
                    }
                    self.skip_branch(b')')?;
                    value
                } else {
                    self.skip_branch(b',')?;
                    self.position += 1;
                    self.logical_or()?
                };
                if !self.eat(")") {
                    return Err("Missing ')' in TBLFM if".into());
                }
                return Ok(chosen);
            }
            let mut arguments = Vec::new();
            if !self.eat(")") {
                loop {
                    arguments.push(self.logical_or()?);
                    if self.eat(")") {
                        break;
                    }
                    if !self.eat(",") {
                        return Err("Expected ',' or ')' in TBLFM function".into());
                    }
                }
            }
            return eval_function(
                &name,
                &arguments,
                self.context.is_some_and(|context| context.radians),
            );
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
    fn skip_branch(&mut self, delimiter: u8) -> Result<(), String> {
        let bytes = self.source.as_bytes();
        let mut depth = 0_usize;
        let mut quoted = false;
        while self.position < bytes.len() {
            let byte = bytes[self.position];
            if byte == b'\\' && quoted {
                self.position = (self.position + 2).min(bytes.len());
                continue;
            }
            if byte == b'"' {
                quoted = !quoted;
            } else if !quoted {
                if byte == b'(' {
                    depth += 1;
                } else if byte == b')' && depth > 0 {
                    depth -= 1;
                } else if depth == 0 && byte == delimiter {
                    return Ok(());
                }
            }
            self.position += 1;
        }
        Err("Unclosed TBLFM if branch".into())
    }
    pub(super) fn parse_reference(&mut self) -> Result<CellRef, String> {
        self.space();
        if let Some((reference, consumed)) = a1_reference(self.rest()) {
            self.position += consumed;
            return Ok(reference);
        }
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
        if self.eat("@#") || self.eat("$#") {
            return Ok(Axis::Current);
        }
        if self.eat("#") || self.eat("0") {
            return Ok(Axis::Current);
        }
        if self.eat("<") {
            let mut offset = 0;
            while self.eat("<") {
                offset += 1;
            }
            return Ok(if offset == 0 {
                Axis::First
            } else {
                Axis::FromFirst(offset)
            });
        }
        if self.eat(">") {
            let mut offset = 0;
            while self.eat(">") {
                offset += 1;
            }
            return Ok(if offset == 0 {
                Axis::Last
            } else {
                Axis::FromLast(offset)
            });
        }
        let relative = if self.eat("+") { true } else { self.eat("-") };
        let negative = relative && self.source.as_bytes().get(self.position - 1) == Some(&b'-');
        if let Some((ordinal, consumed)) = roman_hline(self.rest()) {
            self.position += consumed;
            let offset = if self.eat("+") {
                1
            } else if self.eat("-") {
                -1
            } else {
                0
            };
            let offset = if offset == 0 {
                0
            } else {
                let len = self.rest().bytes().take_while(u8::is_ascii_digit).count();
                if len == 0 {
                    return Err("Invalid TBLFM hline offset".into());
                }
                let value = self.rest()[..len]
                    .parse::<isize>()
                    .map_err(|_| "Invalid TBLFM hline offset")?;
                self.position += len;
                offset * value
            };
            return Ok(Axis::Hline {
                ordinal,
                direction: if negative {
                    -1
                } else if relative {
                    1
                } else {
                    0
                },
                offset,
            });
        }
        let len = self.rest().bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 {
            return Err(format!("Invalid TBLFM reference near: {}", self.rest()));
        }
        let value = self.rest()[..len]
            .parse::<usize>()
            .map_err(|_| "Invalid TBLFM coordinate")?;
        self.position += len;
        if relative {
            let value = isize::try_from(value).map_err(|_| "TBLFM coordinate is too large")?;
            Ok(Axis::Relative(if negative { -value } else { value }))
        } else {
            Ok(Axis::Absolute(value))
        }
    }
}
