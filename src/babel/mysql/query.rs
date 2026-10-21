//! Lexical scope checks followed by server-side PREPARE before execution.
pub(in crate::babel) fn validate_query(sql: &str) -> Result<(), String> {
    // Before connecting, accept either string escape mode. Execution validates
    // again using the actual mode of its dedicated server connection.
    validate_query_with_mode(sql, "")
        .or_else(|_| validate_query_with_mode(sql, "NO_BACKSLASH_ESCAPES"))
}

pub(super) fn validate_query_with_mode(sql: &str, mode: &str) -> Result<(), String> {
    let backslash_escapes = !mode
        .split(',')
        .any(|mode| mode.eq_ignore_ascii_case("NO_BACKSLASH_ESCAPES"));
    let ansi_quotes = mode
        .split(',')
        .any(|mode| mode.eq_ignore_ascii_case("ANSI_QUOTES"));
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut words = Vec::new();
    let mut ended = false;
    while i < bytes.len() {
        match bytes[i] {
            b if b.is_ascii_whitespace() => i += 1,
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
                if bytes.get(i + 2) == Some(&b'!') || bytes.get(i + 2..i + 4) == Some(b"M!") {
                    return Err(
                        "MySQL executable comments are not supported in read-only blocks".into(),
                    );
                }
                i += 2;
                while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                    i += 1;
                }
                if i + 1 >= bytes.len() {
                    return Err("Unclosed SQL comment".into());
                }
                i += 2;
            }
            _ if ended => return Err("Run exactly one SQL statement per source block".into()),
            b';' => {
                ended = true;
                i += 1;
            }
            delimiter @ (b'\'' | b'"' | 0x60) => {
                i += 1;
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == b'\\'
                        && backslash_escapes
                        && delimiter != 0x60
                        && !(delimiter == b'"' && ansi_quotes)
                    {
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
            }
            b if b.is_ascii_alphabetic() || b == b'_' => {
                let start = i;
                while bytes
                    .get(i)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
                {
                    i += 1;
                }
                let word = sql[start..i].to_ascii_uppercase();
                let function = matches!(word.as_str(), "INSERT" | "REPLACE")
                    && bytes[i..]
                        .iter()
                        .copied()
                        .find(|b| !b.is_ascii_whitespace())
                        == Some(b'(');
                if !function {
                    words.push(word);
                }
            }
            b'@' => return Err("Session variables are not supported in read-only queries".into()),
            b':' if bytes.get(i + 1) == Some(&b'=') => {
                return Err("SQL assignments are not supported".into());
            }
            _ => i += 1,
        }
    }
    if !words
        .first()
        .is_some_and(|word| matches!(word.as_str(), "SELECT" | "WITH" | "EXPLAIN"))
    {
        return Err(
            "MySQL blocks currently support SELECT, read-only WITH and EXPLAIN SELECT".into(),
        );
    }
    let prohibited = [
        "INSERT",
        "UPDATE",
        "DELETE",
        "REPLACE",
        "CREATE",
        "ALTER",
        "DROP",
        "TRUNCATE",
        "CALL",
        "GRANT",
        "REVOKE",
        "COMMIT",
        "ROLLBACK",
        "START",
        "BEGIN",
        "SET",
        "INTO",
        "OUTFILE",
        "DUMPFILE",
        "FOR",
        "LOCK",
        "ANALYZE",
        "LOAD_FILE",
        "GET_LOCK",
        "RELEASE_LOCK",
    ];
    if let Some(word) = words
        .iter()
        .find(|word| prohibited.contains(&word.as_str()))
    {
        return Err(format!("MySQL read-only blocks do not support {word}"));
    }
    if words[0] == "EXPLAIN" && !words.iter().any(|word| word == "SELECT") {
        return Err("Only EXPLAIN SELECT is supported".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permits_queries_and_semicolons_inside_literals() {
        for query in [
            "SELECT 1;",
            "SELECT ';DROP' AS x",
            "-- comment\nWITH x AS (SELECT 1) SELECT * FROM x;",
            "EXPLAIN SELECT * FROM orders",
            "SELECT 'it''s; fine'; /* trailing */",
            "SELECT REPLACE('a', 'a', 'b'), INSERT('a', 1, 1, 'b')",
        ] {
            validate_query(query).unwrap();
        }
    }
    #[test]
    fn refuses_multiple_statements_and_mysql_side_effects() {
        for query in [
            "SELECT 1; DELETE FROM t",
            "SELECT 1;;",
            "/*!50000 DELETE FROM t */ SELECT 1",
            "SELECT 1 INTO OUTFILE '/tmp/x'",
            "SELECT * FROM t FOR UPDATE",
            "EXPLAIN ANALYZE SELECT 1",
            "WITH x AS (SELECT 1) UPDATE t SET a=1",
            "SELECT GET_LOCK('x',1)",
            "SELECT @x:=1",
            "SELECT 'unfinished",
            "SELECT 1 /* unfinished",
            "SHOW TABLES",
            "",
        ] {
            assert!(validate_query(query).is_err(), "{query}");
        }
    }
    #[test]
    fn statement_scope_uses_the_server_quote_mode() {
        let sql = r"SELECT '\' INTO OUTFILE '/tmp/x' -- '";
        assert!(validate_query_with_mode(sql, "").is_ok());
        assert!(validate_query_with_mode(sql, "NO_BACKSLASH_ESCAPES").is_err());
        let sql = r#"SELECT "\" INTO OUTFILE '/tmp/x' -- ""#;
        assert!(validate_query_with_mode(sql, "").is_ok());
        assert!(validate_query_with_mode(sql, "ANSI_QUOTES").is_err());
    }
}
