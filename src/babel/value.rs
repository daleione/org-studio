use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub(super) enum Value {
    #[default]
    Nil,
    Bool(bool),
    Integer(String),
    Float(f64),
    Decimal(String),
    Text(String),
    List(Vec<Value>),
    Hline,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Data {
    pub value: Value,
    pub columns: Option<Vec<Value>>,
    pub row_names: Vec<Value>,
}

impl Value {
    pub(super) fn byte_len(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Text(v) | Self::Integer(v) | Self::Decimal(v) => v.len(),
                Self::List(values) => values.iter().map(Self::byte_len).sum(),
                _ => 0,
            }
    }
    pub(super) fn literal(text: &str) -> Result<Option<Self>, String> {
        if text.starts_with('"') {
            return serde_json::from_str::<String>(text)
                .map(|v| Some(Self::Text(v)))
                .map_err(|_| "Invalid quoted Babel string; use JSON string escapes".into());
        }
        if text == "nil" {
            return Ok(Some(Self::Nil));
        }
        if text == "t" {
            return Ok(Some(Self::Bool(true)));
        }
        let digits = if text.starts_with(['+', '-']) {
            &text[1..]
        } else {
            text
        };
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(Some(Self::Integer(text.into())));
        }
        if let Ok(number) = text.parse::<f64>() {
            if !number.is_finite() {
                return Err("Babel numbers must be finite".into());
            }
            return Ok(Some(Self::Float(number)));
        }
        if let Some(body) = text.strip_prefix("'(").and_then(|s| s.strip_suffix(')')) {
            let values = super::expression::split(body, char::is_whitespace)?
                .into_iter()
                .map(|s| {
                    Self::literal(s)?
                        .ok_or_else(|| "Only literal values are allowed in a Babel list".into())
                })
                .collect::<Result<_, String>>()?;
            return Ok(Some(Self::List(values)));
        }
        Ok(None)
    }

    pub(super) fn display(&self) -> String {
        match self {
            Self::Nil => "nil".into(),
            Self::Bool(true) => "t".into(),
            Self::Bool(false) => "nil".into(),
            Self::Text(v) | Self::Integer(v) | Self::Decimal(v) => v.clone(),
            Self::Float(v) => v.to_string(),
            Self::Hline => "hline".into(),
            Self::List(_) => String::new(),
        }
    }

    pub(super) fn number(&self) -> bool {
        matches!(self, Self::Integer(_) | Self::Float(_) | Self::Decimal(_))
    }
}

impl Data {
    pub(super) fn into_raw(mut self) -> Self {
        if let Value::List(rows) = &mut self.value {
            let mut names = self.row_names.into_iter();
            for row in rows.iter_mut() {
                if let Value::List(cells) = row
                    && let Some(name) = names.next()
                {
                    cells.insert(0, name);
                }
            }
            if let Some(columns) = self.columns.take() {
                rows.insert(0, Value::Hline);
                rows.insert(0, Value::List(columns));
            }
        }
        self.row_names = Vec::new();
        self
    }

    pub(super) fn preprocess(mut self, headers: &super::headers::Headers) -> Result<Self, String> {
        let Value::List(rows) = &mut self.value else {
            return Ok(self);
        };
        let detected = rows.get(1) == Some(&Value::Hline)
            && rows.iter().filter(|v| matches!(v, Value::Hline)).count() == 1;
        let names = match headers.get(":colnames").unwrap_or("nil") {
            "yes" => true,
            "no" => false,
            "nil" => detected,
            _ => return Err("Use :colnames yes, no, or nil".into()),
        };
        if self.columns.is_none()
            && names
            && let Some(index) = rows.iter().position(|v| matches!(v, Value::List(_)))
        {
            let Value::List(names) = rows[index].clone() else {
                unreachable!()
            };
            rows.drain(..=index);
            self.columns = Some(names);
            if rows.first() == Some(&Value::Hline) {
                rows.remove(0);
            }
        }
        if headers.get(":hlines") != Some("yes") {
            rows.retain(|v| *v != Value::Hline);
        }
        if headers.get(":rownames") == Some("yes") {
            rows.retain(|v| *v != Value::Hline);
            for row in rows {
                if let Value::List(cells) = row
                    && !cells.is_empty()
                {
                    self.row_names.push(cells.remove(0));
                }
            }
        }
        Ok(self)
    }
}
