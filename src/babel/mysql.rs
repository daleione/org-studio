//! SQLx MySQL execution using standard Babel headers and connection URLs.
mod connection;
mod query;
mod table;
#[cfg(test)]
mod tests;

pub(crate) use connection::CONNECTION_HELP;
pub(super) use query::validate_query;
pub(crate) use table::QueryTable;

use super::headers::Headers;
use futures_util::TryStreamExt;
use serde::Deserialize;
use sqlx::mysql::{MySqlColumn, MySqlConnectOptions, MySqlConnection};
use sqlx::{Column, ConnectOptions, Decode, Executor, Row, TypeInfo, ValueRef};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(default)]
struct QueryLimits {
    max_rows: usize,
    max_result_bytes: usize,
    query_timeout_seconds: u64,
}
impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_rows: 1_000,
            max_result_bytes: 1024 * 1024,
            query_timeout_seconds: 30,
        }
    }
}
impl QueryLimits {
    fn load() -> Result<Self, String> {
        let path = crate::settings::application_support_dir()
            .ok_or("Could not find the application configuration directory")?
            .join("mysql-connections.json");
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| "Invalid MySQL execution limits configuration".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err("Could not read MySQL execution limits".into()),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    async fn wait(&self) {
        while !self.cancelled() {
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}

pub(crate) struct MySqlRequest {
    connection: Option<connection::Connection>,
    pub(crate) cancel: CancelToken,
    pub(crate) ask_each_time: bool,
    pub(crate) source_name: Option<String>,
    pub(super) headers: Headers,
    limits: QueryLimits,
}

pub(crate) struct ConnectionTarget {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) user: String,
    pub(crate) database: String,
}

impl MySqlRequest {
    pub(super) fn from_headers(headers: Headers) -> Result<Self, String> {
        Ok(Self {
            connection: connection::from_headers(&headers)?,
            ask_each_time: headers
                .get(":eval")
                .is_some_and(|v| v.eq_ignore_ascii_case("query")),
            headers,
            cancel: CancelToken::default(),
            source_name: None,
            limits: QueryLimits::load()?,
        })
    }
    pub(crate) fn ready(&self) -> bool {
        self.connection.is_some()
    }
    pub(crate) fn identity(&self) -> String {
        self.connection
            .as_ref()
            .map(|connection| connection.identity.clone())
            .unwrap_or_default()
    }
    pub(crate) fn target(&self) -> Option<ConnectionTarget> {
        let options = &self.connection.as_ref()?.options;
        Some(ConnectionTarget {
            host: options.get_host().to_owned(),
            port: options.get_port(),
            user: options.get_username().to_owned(),
            database: options.get_database().unwrap_or("").to_owned(),
        })
    }
    pub(super) fn byte_limit(&self) -> usize {
        self.limits.max_result_bytes.clamp(1024, 64 * 1024 * 1024)
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}
async fn connect(options: &MySqlConnectOptions, password: &str) -> Result<MySqlConnection, String> {
    tokio::time::timeout(CONNECT_TIMEOUT, options.connect())
        .await
        .map_err(|_| "MySQL connection timed out after 10 seconds".to_owned())?
        .map_err(|e| database_error(e, password))
}
fn database_error(error: sqlx::Error, password: &str) -> String {
    let text = match error {
        sqlx::Error::Database(error) => {
            let number = error
                .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                .map(|e| e.number().to_string())
                .unwrap_or_default();
            format!(
                "MySQL {number} ({}): {}",
                error.code().unwrap_or_default(),
                error.message()
            )
        }
        error => format!("MySQL: {error}"),
    };
    if password.is_empty() {
        text
    } else {
        text.replace(password, "[redacted]")
    }
}

pub(super) fn execute(request: &MySqlRequest, source: &str) -> Result<QueryTable, String> {
    validate_query(source)?;
    let connection = request
        .connection
        .as_ref()
        .ok_or(connection::CONNECTION_HELP)?;
    let password = connection.password.as_str();
    let options = &connection.options;
    runtime()?.block_on(async {
        let mut conn = tokio::select! {
            result = connect(options, password) => result?,
            _ = request.cancel.wait() => return Err("MySQL query cancelled before connecting".into()),
        };
        let (id, sql_mode): (u64, String) = tokio::select! {
            result = tokio::time::timeout(CONNECT_TIMEOUT, sqlx::query_as("SELECT CONNECTION_ID(), @@SESSION.sql_mode").fetch_one(&mut conn)) =>
                result.map_err(|_| "MySQL connection setup timed out")?
                    .map_err(|e| database_error(e, password))?,
            _ = request.cancel.wait() => return Err("MySQL query cancelled before execution".into()),
        };
        query::validate_query_with_mode(source, &sql_mode)?;
        let start = Instant::now();
        let timeout = Duration::from_secs(request.limits.query_timeout_seconds.clamp(1, 3600));
        let outcome = tokio::select! {
            result = run_query(&mut conn, source, &request.limits) => Some(result),
            _ = request.cancel.wait() => None,
            _ = tokio::time::sleep(timeout) => None,
        };
        match outcome {
            Some(Ok(mut table)) => {
                let cleanup = tokio::time::timeout(CONNECT_TIMEOUT, conn.execute("ROLLBACK")).await;
                drop(conn);
                if !matches!(cleanup, Ok(Ok(_))) { return Err("MySQL transaction cleanup failed; results were not updated".into()); }
                table.elapsed = start.elapsed();
                Ok(table)
            }
            Some(Err(error)) => {
                // A stream stopped by a size limit can still be executing on
                // the server. Cancel it before attempting to drain/roll back.
                let confirmed = kill_and_cleanup(options, password, id, &mut conn).await;
                drop(conn);
                let mut error = if password.is_empty() { error } else { error.replace(password, "[redacted]") };
                if !confirmed { error.push_str("; server cancellation could not be confirmed"); }
                Err(error)
            }
            None => {
                // Never reuse this connection or its server ID while cancellation
                // is in flight. A dropped UI future is not server cancellation.
                let confirmed = kill_and_cleanup(options, password, id, &mut conn).await;
                drop(conn);
                let reason = if request.cancel.cancelled() { "cancelled" } else { "timed out" };
                Err(if confirmed { format!("MySQL query {reason}; previous results were preserved") }
                    else { format!("MySQL query {reason}; server cancellation could not be confirmed") })
            }
        }
    })
}
async fn kill_and_cleanup(
    options: &MySqlConnectOptions,
    password: &str,
    id: u64,
    conn: &mut MySqlConnection,
) -> bool {
    let killed = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let mut control = connect(options, password).await?;
        control
            .execute(format!("KILL QUERY {id}").as_str())
            .await
            .map_err(|e| database_error(e, password))?;
        Ok::<_, String>(())
    })
    .await;
    let cleaned = tokio::time::timeout(CONNECT_TIMEOUT, async {
        match conn.execute("ROLLBACK").await {
            // SQLx first drains a dropped row stream. An interrupted query can
            // surface here before ROLLBACK has actually been sent.
            Err(sqlx::Error::Database(error))
                if error
                    .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                    .is_some_and(|e| e.number() == 1317) =>
            {
                conn.execute("ROLLBACK").await
            }
            result => result,
        }
    })
    .await;
    matches!(killed, Ok(Ok(()))) && matches!(cleaned, Ok(Ok(_)))
}
async fn run_query(
    conn: &mut MySqlConnection,
    source: &str,
    limits: &QueryLimits,
) -> Result<QueryTable, String> {
    conn.execute("START TRANSACTION READ ONLY")
        .await
        .map_err(|e| database_error(e, ""))?;
    // PREPARE validates the complete statement without executing it and supplies
    // column names even for zero rows. The server rejects multiple statements,
    // independently of client tokenization and the server's sql_mode.
    let description = conn
        .describe(source)
        .await
        .map_err(|e| database_error(e, ""))?;
    let mut table = table_with_columns(description.columns());
    let max_rows = limits.max_rows.clamp(1, 100_000);
    let max_bytes = limits.max_result_bytes.clamp(1024, 64 * 1024 * 1024);
    let mut stream = (&mut *conn).fetch(source);
    while let Some(row) = stream.try_next().await.map_err(|e| database_error(e, ""))? {
        if table.rows.is_empty() {
            // MySQL can omit EXPLAIN metadata during PREPARE. Runtime metadata
            // is authoritative; keep the description only for zero-row results.
            table = table_with_columns(row.columns());
        }
        if table.rows.len() >= max_rows {
            return Err(format!(
                "MySQL result exceeds {max_rows} rows; add LIMIT or adjust the local result limit. Previous results were preserved"
            ));
        }
        let mut cells = Vec::with_capacity(row.len());
        for column in row.columns() {
            let value = row
                .try_get_raw(column.ordinal())
                .map_err(|e| database_error(e, ""))?;
            if value.is_null() {
                cells.push(None);
                continue;
            }
            // All types in the text protocol retain their exact server bytes.
            let bytes = <&[u8] as Decode<sqlx::MySql>>::decode(value)
                .map_err(|_| "Could not decode a MySQL cell")?;
            let text = match std::str::from_utf8(bytes) {
                Ok(text) if !text.contains('\0') => text.to_owned(),
                _ => {
                    let mut text = String::from("0x");
                    use std::fmt::Write;
                    for byte in bytes {
                        let _ = write!(text, "{byte:02X}");
                    }
                    text
                }
            };
            table.raw_bytes += text.len();
            if table.raw_bytes > max_bytes {
                return Err(format!(
                    "MySQL result exceeds {max_bytes} bytes; previous results were preserved"
                ));
            }
            cells.push(Some(text));
        }
        table.rows.push(cells);
    }
    Ok(table)
}

fn table_with_columns(columns: &[MySqlColumn]) -> QueryTable {
    let mut table = QueryTable::new(columns.iter().map(|c| c.name().to_owned()).collect());
    table.numeric_columns = columns
        .iter()
        .map(|column| {
            matches!(
                column.type_info().name().split_whitespace().next(),
                Some(
                    "TINYINT"
                        | "SMALLINT"
                        | "MEDIUMINT"
                        | "INT"
                        | "BIGINT"
                        | "FLOAT"
                        | "DOUBLE"
                        | "DECIMAL"
                )
            )
        })
        .collect();
    table
}
