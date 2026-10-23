use crate::babel::value::Value;

pub(in crate::babel) fn preflight(
    source: &str,
    variables: &[(String, Value)],
) -> Result<(String, Vec<Value>), String> {
    expand(source, variables, "").or_else(|_| expand(source, variables, "NO_BACKSLASH_ESCAPES"))
}

/// SQL values use SQLx binds. Identifiers, comments and literal fragments are
/// never expanded; only $name and the complete single-quoted '$name' are valid.
pub(in crate::babel) fn expand(
    source: &str,
    variables: &[(String, Value)],
    mode: &str,
) -> Result<(String, Vec<Value>), String> {
    let bytes = source.as_bytes();
    let backslash = !mode
        .split(',')
        .any(|m| m.eq_ignore_ascii_case("NO_BACKSLASH_ESCAPES"));
    let mut output = String::new();
    let mut values = Vec::new();
    let mut i = 0;
    let name_end = |start: usize| {
        if !bytes
            .get(start)
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            return start;
        }
        let mut end = start + 1;
        while bytes
            .get(end)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            end += 1;
        }
        end
    };
    let value = |name: &str, quoted: bool| -> Result<Value, String> {
        let value = &variables
            .iter()
            .find(|(n, _)| n == name)
            .ok_or_else(|| format!("Unknown SQL variable: {name}"))?
            .1;
        if matches!(value, Value::List(_) | Value::Hline) {
            return Err(format!("SQL variable {name} must be a scalar"));
        }
        Ok(if quoted {
            Value::Text(value.display())
        } else {
            value.clone()
        })
    };
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'-' if bytes.get(i + 1) == Some(&b'-')
                && bytes.get(i + 2).is_none_or(u8::is_ascii_whitespace) =>
            {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                    i += 1;
                }
                if i + 1 >= bytes.len() {
                    return Err("Unclosed SQL comment".into());
                }
                i += 2;
            }
            delimiter @ (b'\'' | b'"' | b'`') => {
                i += 1;
                let content = i;
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && backslash && delimiter != b'`' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == delimiter {
                        i += 1;
                        if bytes.get(i) == Some(&delimiter) {
                            i += 1;
                            continue;
                        }
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    return Err("Unclosed SQL string or identifier".into());
                }
                let text = &source[content..i - 1];
                if delimiter == b'\'' && text.starts_with('$') && name_end(content + 1) == i - 1 {
                    values.push(value(&text[1..], true)?);
                    output.push('?');
                    continue;
                }
                if delimiter != b'`'
                    && text
                        .char_indices()
                        .any(|(j, c)| c == '$' && name_end(content + j + 1) > content + j + 1)
                {
                    return Err("Use a complete '$name' value or bare $name; SQL literal templates and quoted identifiers are not supported".into());
                }
            }
            b'$' if start == 0
                || !matches!(bytes[start-1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'$') =>
            {
                let end = name_end(i + 1);
                if end > i + 1 {
                    values.push(value(&source[i + 1..end], false)?);
                    output.push('?');
                    i = end;
                    continue;
                }
                i += 1;
            }
            _ => {
                i += source[i..].chars().next().unwrap().len_utf8();
            }
        }
        output.push_str(&source[start..i]);
    }
    Ok((output, values))
}

pub(super) fn arguments(values: &[Value]) -> Result<sqlx::mysql::MySqlArguments, String> {
    use sqlx::Arguments;
    let mut args = sqlx::mysql::MySqlArguments::default();
    for value in values {
        let result = match value {
            Value::Nil => args.add(Option::<String>::None),
            Value::Bool(v) => args.add(*v),
            Value::Integer(v) => {
                if let Ok(v) = v.parse::<i64>() {
                    args.add(v)
                } else {
                    args.add(
                        v.parse::<u64>()
                            .map_err(|_| "SQL integer exceeds the 64-bit range")?,
                    )
                }
            }
            Value::Float(v) => args.add(*v),
            Value::Decimal(v) | Value::Text(v) => args.add(v.clone()),
            _ => return Err("SQL variables must be scalar values".into()),
        };
        result.map_err(|_| "Could not bind SQL variable")?;
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_whole_values_without_expanding_comments_identifiers_or_prefixes() {
        let variables = vec![
            ("n".into(), Value::Integer("12".into())),
            ("category".into(), Value::Text("' OR 1=1 --".into())),
        ];
        let (sql, values) = expand(
            "SELECT $n, '$category', `cost$n`, 1 /* $ignored */ -- $ignored\n",
            &variables,
            "",
        )
        .unwrap();
        assert_eq!(sql, "SELECT ?, ?, `cost$n`, 1 /* $ignored */ -- $ignored\n");
        assert_eq!(values, [variables[0].1.clone(), variables[1].1.clone()]);
        assert!(
            expand("SELECT $nsuffix", &variables, "")
                .unwrap_err()
                .contains("nsuffix")
        );
        for sql in [
            "SELECT 'prefix $category'",
            "SELECT '$category suffix'",
            "SELECT \"$category\"",
        ] {
            assert!(expand(sql, &variables, "").is_err());
        }
        let (sql, values) = expand("SELECT '$n', $n", &variables, "NO_BACKSLASH_ESCAPES").unwrap();
        assert_eq!(sql, "SELECT ?, ?");
        assert_eq!(
            values,
            [Value::Text("12".into()), Value::Integer("12".into())]
        );
        assert!(arguments(&[Value::Integer("18446744073709551616".into())]).is_err());
    }
}
