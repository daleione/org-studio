use super::*;
use sqlx::mysql::MySqlSslMode;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
};

struct Fixture {
    process: Child,
    directory: tempfile::TempDir,
    port: u16,
}
impl Fixture {
    fn start() -> Self {
        Self::with_certificate_names("DNS:localhost,IP:127.0.0.1")
    }
    fn with_certificate_names(names: &str) -> Self {
        Self::with_server_mode(names, "")
    }
    fn with_server_mode(names: &str, mode: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut process = Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/mysql_wire_server.py"
            ))
            .arg(directory.path())
            .arg(names)
            .arg(mode)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Python 3 and OpenSSL are required for the MySQL TLS protocol fixture");
        let mut line = String::new();
        BufReader::new(process.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(!line.is_empty(), "MySQL protocol fixture failed to start");
        let port = serde_json::from_str::<serde_json::Value>(&line).unwrap()["port"]
            .as_u64()
            .unwrap() as u16;
        Self {
            process,
            directory,
            port,
        }
    }
    fn request(&self) -> MySqlRequest {
        let mut url = url::Url::parse(&format!(
            "mysql://reader:test-password@127.0.0.1:{}/analytics",
            self.port,
        ))
        .unwrap();
        url.query_pairs_mut()
            .append_pair("ssl-mode", "VERIFY_IDENTITY")
            .append_pair(
                "ssl-ca",
                self.directory.path().join("ca.crt").to_str().unwrap(),
            );
        let mut headers = Headers::default();
        headers.parse(&format!(":dbconnection \"{url}\"")).unwrap();
        MySqlRequest {
            connection: connection::from_headers(&headers).unwrap(),
            cancel: CancelToken::default(),
            ask_each_time: false,
            headers,
            limits: QueryLimits::default(),
        }
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.directory.path().join("queries.log")).unwrap_or_default()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[test]
fn rustls_query_preserves_exact_text_nulls_unicode_and_read_only_transaction() {
    let fixture = Fixture::start();
    let table = execute(&fixture.request(), "SELECT 'fixture' AS code").unwrap();
    assert_eq!(
        table.columns,
        ["code", "amount", "total", "note", "message"]
    );
    assert_eq!(table.numeric_columns, [false, true, true, false, false]);
    assert_eq!(table.rows[0][0].as_deref(), Some("00123"));
    assert_eq!(table.rows[0][1].as_deref(), Some("18446744073709551615"));
    assert_eq!(
        table.rows[0][2].as_deref(),
        Some("1234567890.123456789012345678")
    );
    assert!(table.rows[0][3].is_none());
    assert_eq!(table.rows[0][4].as_deref(), Some("a|b\n中文"));
    let log = fixture.log();
    assert!(log.contains("TLS"));
    assert!(log.contains("QUERY START TRANSACTION READ ONLY"));
    assert!(log.contains("QUERY ROLLBACK"));
    assert_eq!(log.matches("QUERY SELECT 'fixture'").count(), 1);

    let empty = execute(&fixture.request(), "SELECT 'fixture' AS code WHERE 1=0").unwrap();
    assert_eq!(empty.columns, table.columns);
    assert!(empty.rows.is_empty());
}

#[test]
fn bound_mysql_values_feed_python_without_losing_types_or_publishing_dependency_results() {
    use crate::{
        babel::{execute_plan, prepare_execution, value::Value},
        document::{ByteOffset, DocumentSnapshot, Selection},
    };
    let fixture = Fixture::start();
    let directory = tempfile::tempdir().unwrap();
    let text = format!(
        "#+PROPERTY: header-args:sql :dbconnection {}\n#+name: records\n#+begin_src sql :var minimum=12 category=\"' OR 1=1 --\" missing=nil big=18446744073709551615\nSELECT $minimum, '$category', $missing, $big /* BABEL_DATES */;\n#+end_src\n\n#+RESULTS: records\n| previous |\n\n#+name: summary\n#+begin_src python :var rows=records\nfrom decimal import Decimal\nassert rows[0][0] == '00123'\nassert rows[0][1] == 18446744073709551615\nassert rows[0][2] == Decimal('1234567890.123456789012345678')\nassert rows[0][3] is None\nassert rows[0][4] == 'a|b\\n中文'\nassert rows[0][5] == '2026-01-02 03:04:05.123456'\nassert rows[0][6] == '2026-01-02'\nreturn [[rows[0][0], rows[0][1], rows[0][2]]]\n#+end_src\n",
        fixture.request().headers.get(":dbconnection").unwrap()
    );
    let snapshot = DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap();
    let plan = prepare_execution(
        &snapshot,
        &directory.path().join("analysis.org"),
        Selection::caret(ByteOffset(text.find("return [[").unwrap() as u64)),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        plan.requests()
            .iter()
            .filter(|(_, r)| r.mysql.is_some())
            .count(),
        1
    );
    let output = execute_plan(plan).unwrap();
    assert_eq!(
        output.data.value,
        Value::List(vec![Value::List(vec![
            Value::Text("00123".into()),
            Value::Integer("18446744073709551615".into()),
            Value::Decimal("1234567890.123456789012345678".into())
        ])])
    );
    let edit = output.result_edit.unwrap();
    assert!(edit.range.start.0 >= text.find("#+name: summary").unwrap() as u64);
    let log = fixture.log();
    assert!(
        log.contains("EXECUTE SELECT ?, ?, ?, ? /* BABEL_DATES */;"),
        "{log}"
    );
    assert!(
        !log.contains("OR 1=1"),
        "values must not be interpolated into SQL"
    );
    let binds = log.lines().find_map(|l| l.strip_prefix("BINDS ")).unwrap();
    assert!(
        binds.starts_with("0401"),
        "third parameter must be SQL NULL: {binds}"
    );
    assert!(
        binds.contains("0c00000000000000"),
        "first parameter must be integer 12"
    );
    let attack = "' OR 1=1 --"
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    assert!(binds.contains(&attack), "text is sent as data");
    assert!(
        binds.ends_with("ffffffffffffffff"),
        "unsigned 64-bit value stays exact"
    );
    assert!(log.contains("QUERY ROLLBACK"));
}

#[test]
fn named_connection_replaces_results_above_the_source_without_moving_the_caret_off_sql() {
    use crate::document::{
        ByteOffset, ByteRange, DocumentBuffer, EditTransaction, Selection, TextSnapshot,
    };
    let fixture = Fixture::start();
    let connection = fixture.request();
    let source = format!(
        "* Reports\n:PROPERTIES:\n:REPORTS: {}\n:END:\n\n#+RESULTS: report\n| old value |\n\nExplanation.\n\n#+name: report\n#+begin_src sql :engine mysql :dbconnection reports\nSELECT 'fixture';\n#+end_src\n",
        connection.headers.get(":dbconnection").unwrap()
    );
    let mut document = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
    let selection = Selection::caret(ByteOffset(source.find("SELECT").unwrap() as u64));
    let request = crate::babel::prepare_source_block_execution(
        &document.snapshot(),
        std::path::Path::new("/tmp/report.org"),
        selection,
    )
    .unwrap();
    let output = crate::babel::execute_source_block(request).unwrap();
    let caret = output.offset_after_result_edit(output.selection.head());
    let block = output.offset_after_result_edit(output.source_block_start);
    document
        .commit(EditTransaction::new(
            output.revision,
            vec![output.result_edit.unwrap()],
        ))
        .unwrap();
    let snapshot = document.snapshot();
    let result = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
    assert_eq!(result.matches("#+RESULTS: report").count(), 1);
    assert!(!result.contains("old value"));
    assert!(result.contains("Explanation."));
    assert!(
        snapshot
            .copy_range(ByteRange::new(caret.0, snapshot.len_bytes()))
            .starts_with("SELECT 'fixture';")
    );
    assert!(
        snapshot
            .copy_range(ByteRange::new(block.0, caret.0))
            .starts_with("#+begin_src sql")
    );
}

#[test]
fn rustls_explain_uses_runtime_metadata_when_prepare_has_no_columns() {
    let fixture = Fixture::start();
    let table = execute(&fixture.request(), "EXPLAIN SELECT * FROM orders").unwrap();
    assert_eq!(
        table.columns,
        ["id", "select_type", "table", "rows", "Extra"]
    );
    assert_eq!(table.numeric_columns, [true, false, false, true, false]);
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.rows[0].len(), table.columns.len());
    assert_eq!(table.rows[0][1].as_deref(), Some("SIMPLE"));
    assert_eq!(table.rows[0][4].as_deref(), Some("Using where"));
}

#[test]
fn rustls_rejects_untrusted_certificates_without_running_sql() {
    let fixture = Fixture::start();
    let mut request = fixture.request();
    request.connection.as_mut().unwrap().options = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(fixture.port)
        .username("reader")
        .password("test-password")
        .database("analytics")
        .ssl_mode(MySqlSslMode::VerifyIdentity);
    assert!(execute(&request, "SELECT 1").is_err());
    assert!(!fixture.log().contains("START TRANSACTION"));
}

#[test]
fn rustls_rejects_a_trusted_certificate_for_a_different_host() {
    let fixture = Fixture::with_certificate_names("DNS:other.example");
    let error = execute(&fixture.request(), "SELECT 1").unwrap_err();
    assert!(error.contains("certificate"), "{error}");
    assert!(!fixture.log().contains("START TRANSACTION"));
}

#[test]
fn mysql_checks_actual_sql_mode_before_submitting_document_sql() {
    let fixture = Fixture::with_server_mode("DNS:localhost,IP:127.0.0.1", "NO_BACKSLASH_ESCAPES");
    let sql = r"SELECT '\' INTO OUTFILE '/tmp/x' -- '";
    let error = execute(&fixture.request(), sql).unwrap_err();
    assert!(error.contains("INTO"), "{error}");
    assert!(!fixture.log().contains("QUERY SELECT"));
    assert!(!fixture.log().contains("START TRANSACTION"));
}

#[test]
fn mysql_timeout_cancels_the_server_query_and_preserves_results() {
    let fixture = Fixture::start();
    let mut request = fixture.request();
    request.limits.query_timeout_seconds = 1;
    let error = execute(&request, "SELECT SLEEP(10)").unwrap_err();
    assert!(error.contains("timed out"), "{error}");
    assert!(!error.contains("could not be confirmed"), "{error}");
    assert!(fixture.log().contains("QUERY KILL QUERY"));
    assert!(fixture.log().contains("QUERY ROLLBACK"));
}

#[test]
fn mysql_syntax_error_and_result_limit_do_not_return_partial_results() {
    let fixture = Fixture::start();
    assert!(
        execute(&fixture.request(), "SELECT SYNTAX_ERROR")
            .unwrap_err()
            .contains("1064")
    );
    let error = execute(&fixture.request(), "SELECT MANY_ROWS").unwrap_err();
    assert!(error.contains("1000 rows"), "{error}");
}

#[test]
fn mysql_stop_sends_kill_query_and_cleans_the_original_transaction() {
    let fixture = Fixture::start();
    let request = fixture.request();
    let cancel = request.cancel.clone();
    let path = fixture.directory.path().join("queries.log");
    let signal = std::thread::spawn(move || {
        for _ in 0..200 {
            if std::fs::read_to_string(&path)
                .unwrap_or_default()
                .contains("QUERY SELECT SLEEP")
            {
                cancel.cancel();
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        cancel.cancel();
    });
    let error = execute(&request, "SELECT SLEEP(10)").unwrap_err();
    signal.join().unwrap();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!error.contains("could not be confirmed"), "{error}");
    assert!(fixture.log().contains("QUERY KILL QUERY"));
    assert!(fixture.log().contains("QUERY ROLLBACK"));
}
