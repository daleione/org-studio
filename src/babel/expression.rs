use super::value::Value;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Expression {
    Literal(Value),
    Reference {
        target: String,
        arguments: Vec<Argument>,
        selector: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Argument {
    pub name: Option<String>,
    pub expression: Expression,
}

#[derive(Clone, Debug)]
pub(super) struct Call {
    pub target: String,
    pub inside: String,
    pub arguments: Vec<Argument>,
    pub end: String,
}

/// Split only at the outer level; quoted strings and nested calls are data.
pub(super) fn split(text: &str, separator: impl Fn(char) -> bool) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if c == delimiter {
                quote = None;
            }
            continue;
        }
        if c == '"' || c == '\'' && !text[i + 1..].starts_with('(') {
            quote = Some(c);
            continue;
        }
        match c {
            '(' => stack.push(')'),
            '[' => stack.push(']'),
            '{' => stack.push('}'),
            ')' | ']' | '}' if stack.pop() != Some(c) => {
                return Err("Unbalanced Babel expression".into());
            }
            _ => {}
        }
        if stack.is_empty() && separator(c) {
            if !text[start..i].trim().is_empty() {
                parts.push(text[start..i].trim());
            }
            start = i + c.len_utf8();
        }
    }
    if quote.is_some() || !stack.is_empty() {
        return Err("Unclosed Babel string or expression".into());
    }
    if !text[start..].trim().is_empty() {
        parts.push(text[start..].trim());
    }
    Ok(parts)
}

pub(super) fn bindings(text: &str) -> Result<Vec<(String, Expression)>, String> {
    split(text, char::is_whitespace)?
        .into_iter()
        .map(|part| {
            let (name, value) = part.split_once('=').ok_or("Use :var name=value")?;
            identifier(name)?;
            Ok((name.to_owned(), parse(value)?))
        })
        .collect()
}

pub(super) fn identifier(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || i > 0 && c.is_ascii_digit())
    {
        return Err(format!("Invalid Babel variable name: {name}"));
    }
    Ok(())
}

pub(super) fn parse(text: &str) -> Result<Expression, String> {
    let text = text.trim();
    if let Some(value) = Value::literal(text)? {
        return Ok(Expression::Literal(value));
    }
    let end = text.find(['(', '[']).unwrap_or(text.len());
    let target = text[..end].trim();
    if target.contains("://") {
        return Err("Quote URL literals in :var; URLs are not Babel references".into());
    }
    if target.is_empty()
        || target.chars().any(char::is_whitespace)
        || target.contains(['=', '"', '\'', '{', '}'])
    {
        return Err("Unsupported Babel expression; use a literal or named reference".into());
    }
    let mut rest = &text[end..];
    let arguments = if rest.starts_with('(') {
        let (body, tail) = group(rest, '(', ')')?;
        rest = tail;
        arguments(body)?
    } else {
        Vec::new()
    };
    let selector = if rest.starts_with('[') {
        let (body, tail) = group(rest, '[', ']')?;
        rest = tail;
        Some(body.to_owned())
    } else {
        None
    };
    if !rest.trim().is_empty() {
        return Err("Unexpected text after Babel reference".into());
    }
    Ok(Expression::Reference {
        target: target.into(),
        arguments,
        selector,
    })
}

pub(super) fn call(text: &str) -> Result<Call, String> {
    let text = text.trim();
    let end = text.find(['[', '(', ' ', '\t']).unwrap_or(text.len());
    let target = text[..end].to_owned();
    if target.is_empty() {
        return Err("#+CALL requires a named source block".into());
    }
    let mut rest = text[end..].trim_start();
    let inside = if rest.starts_with('[') {
        let (body, tail) = group(rest, '[', ']')?;
        rest = tail.trim_start();
        body.into()
    } else {
        String::new()
    };
    let (args, tail) = group(rest, '(', ')')?;
    Ok(Call {
        target,
        inside,
        arguments: arguments(args)?,
        end: tail.trim().into(),
    })
}

fn arguments(text: &str) -> Result<Vec<Argument>, String> {
    split(text, |c| c == ',')?
        .into_iter()
        .map(|part| {
            if let Some((name, value)) = part.split_once('=')
                && identifier(name.trim()).is_ok()
            {
                Ok(Argument {
                    name: Some(name.trim().into()),
                    expression: parse(value)?,
                })
            } else {
                Ok(Argument {
                    name: None,
                    expression: parse(part)?,
                })
            }
        })
        .collect()
}

fn group(text: &str, open: char, close: char) -> Result<(&str, &str), String> {
    if !text.starts_with(open) {
        return Err(format!("Expected {open} in Babel call"));
    }
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && c == '\\' {
            escaped = true;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
        }
        if quoted {
            continue;
        }
        if c == open {
            depth += 1;
        }
        if c == close {
            depth -= 1;
            if depth == 0 {
                return Ok((&text[1..i], &text[i + 1..]));
            }
        }
    }
    Err("Unclosed Babel call or selector".into())
}
