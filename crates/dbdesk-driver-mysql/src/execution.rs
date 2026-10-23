//! Running user statements. `mysql_async` borrows the connection for as long as a result is
//! open, so each statement runs on its own task that owns the connection's lock and reads
//! rows only when the cursor asks for a page, as the SQLite driver's statement thread does.
//! The task keeps the lock until the cursor is finished, which is what keeps the connection
//! busy meanwhile.

use std::sync::Arc;

use async_trait::async_trait;
use dbdesk_core::{Cell, ColumnInfo, DriverError, Rejection, ResultCursor, Row};
use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::prelude::Queryable;
use mysql_async::{Column, Conn, Value};
use tokio::sync::{Mutex, OwnedMutexGuard, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::connection_error;
use crate::errors::{rejection, statement_error};

/// The character set (collation) id of bytes that are not text: `BINARY`, `BLOB` and the
/// like, and also numbers and dates, which the server sends as text in it anyway.
const BINARY_CHARSET: u16 = 63;

/// Binary values longer than this are shown as their size, not their bytes: a result grid is
/// no place for a megabyte of hex.
const MAX_BINARY_HEX_BYTES: usize = 64;

/// What the statement task reports once the server has answered: the result's columns, or,
/// for a statement without rows, how many it changed.
struct Started {
    columns: Vec<ColumnInfo>,
    rows_changed: Option<u64>,
}

struct Page {
    rows: Vec<Row>,
    exhausted: bool,
}

pub(crate) async fn execute(
    conn: &Arc<Mutex<Conn>>,
    sql: &str,
) -> Result<Box<dyn ResultCursor>, DriverError> {
    let conn = Arc::clone(conn).lock_owned().await;
    let (started_sender, started) = oneshot::channel();
    // Capacity 1: the task reads at most one page ahead of what was asked for.
    let (demand, demand_receiver) = mpsc::channel(1);
    let (page_sender, pages) = mpsc::channel(1);
    let channels = Channels {
        started: started_sender,
        demand: demand_receiver,
        pages: page_sender,
    };
    let task = tokio::spawn(run_statement(conn, sql.to_owned(), channels));

    let started = started
        .await
        .map_err(|_| connection_error("the statement task stopped unexpectedly"))??;
    let exhausted = started.columns.is_empty();
    Ok(Box::new(MysqlCursor {
        columns: started.columns,
        rows_changed: started.rows_changed,
        exhausted,
        demand: Some(demand),
        pages,
        task,
    }))
}

/// How the statement task talks to its cursor.
struct Channels {
    started: oneshot::Sender<Result<Started, DriverError>>,
    demand: mpsc::Receiver<usize>,
    pages: mpsc::Sender<Result<Page, DriverError>>,
}

/// The statement task: runs `sql`, then reads one page per demand until the result is
/// exhausted, fails, or the cursor goes away. Whatever is left is read and discarded, so the
/// connection is ready for the next statement, and how that went is the task's result,
/// which [`MysqlCursor::finish`] reports.
async fn run_statement(
    mut conn: OwnedMutexGuard<Conn>,
    sql: String,
    channels: Channels,
) -> Result<(), DriverError> {
    let Channels {
        started,
        mut demand,
        pages,
    } = channels;
    let mut result = match conn.query_iter(sql.as_str()).await {
        Ok(result) => result,
        Err(error) => {
            // The receiver only disappears if the cursor's caller gave up; nothing to do.
            let _ = started.send(Err(statement_error(error, &sql)));
            return Ok(());
        }
    };
    let columns = result.columns_ref();
    let formats: Vec<ValueFormat> = columns.iter().map(ValueFormat::of).collect();
    let column_info: Vec<ColumnInfo> = columns.iter().map(column_info).collect();
    if column_info.is_empty() {
        let rows_changed = result.affected_rows();
        let finished = result
            .drop_result()
            .await
            .map(|()| Started {
                columns: column_info,
                rows_changed: Some(rows_changed),
            })
            .map_err(|error| statement_error(error, &sql));
        let _ = started.send(finished);
        return Ok(());
    }
    if started
        .send(Ok(Started {
            columns: column_info,
            rows_changed: None,
        }))
        .is_err()
    {
        return result
            .drop_result()
            .await
            .map_err(|error| statement_error(error, &sql));
    }
    while let Some(max_rows) = demand.recv().await {
        let mut page = Vec::new();
        let mut exhausted = false;
        while page.len() < max_rows {
            match result.next().await {
                Ok(Some(row)) => page.push(cells(row, &formats)),
                Ok(None) => {
                    exhausted = true;
                    break;
                }
                Err(error) => {
                    let _ = pages.send(Err(statement_error(error, &sql))).await;
                    return Ok(());
                }
            }
        }
        let sent = pages
            .send(Ok(Page {
                rows: page,
                exhausted,
            }))
            .await;
        if sent.is_err() || exhausted {
            break;
        }
    }
    // Rows the cursor never asked for, and any further result sets (a procedure's `CALL`
    // returns one per `SELECT` it runs, then a status).
    result
        .drop_result()
        .await
        .map_err(|error| statement_error(error, &sql))
}

fn column_info(column: &Column) -> ColumnInfo {
    ColumnInfo {
        name: column.name_str().into_owned(),
        type_name: type_name(column).map(str::to_owned),
    }
}

/// The column's type as MySQL names it in `CREATE TABLE`, from what the result's metadata
/// reports. Text and binary types share wire types, told apart by the binary character set.
fn type_name(column: &Column) -> Option<&'static str> {
    let binary = is_binary_charset(column);
    let flags = column.flags();
    Some(match column.column_type() {
        ColumnType::MYSQL_TYPE_TINY => "tinyint",
        ColumnType::MYSQL_TYPE_SHORT => "smallint",
        ColumnType::MYSQL_TYPE_INT24 => "mediumint",
        ColumnType::MYSQL_TYPE_LONG => "int",
        ColumnType::MYSQL_TYPE_LONGLONG => "bigint",
        ColumnType::MYSQL_TYPE_FLOAT => "float",
        ColumnType::MYSQL_TYPE_DOUBLE => "double",
        ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => "decimal",
        ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => "date",
        ColumnType::MYSQL_TYPE_TIME | ColumnType::MYSQL_TYPE_TIME2 => "time",
        ColumnType::MYSQL_TYPE_DATETIME | ColumnType::MYSQL_TYPE_DATETIME2 => "datetime",
        ColumnType::MYSQL_TYPE_TIMESTAMP | ColumnType::MYSQL_TYPE_TIMESTAMP2 => "timestamp",
        ColumnType::MYSQL_TYPE_YEAR => "year",
        ColumnType::MYSQL_TYPE_BIT => "bit",
        ColumnType::MYSQL_TYPE_JSON => "json",
        ColumnType::MYSQL_TYPE_GEOMETRY => "geometry",
        ColumnType::MYSQL_TYPE_ENUM => "enum",
        ColumnType::MYSQL_TYPE_SET => "set",
        ColumnType::MYSQL_TYPE_STRING if flags.contains(ColumnFlags::ENUM_FLAG) => "enum",
        ColumnType::MYSQL_TYPE_STRING if flags.contains(ColumnFlags::SET_FLAG) => "set",
        ColumnType::MYSQL_TYPE_STRING if binary => "binary",
        ColumnType::MYSQL_TYPE_STRING => "char",
        ColumnType::MYSQL_TYPE_VARCHAR | ColumnType::MYSQL_TYPE_VAR_STRING if binary => "varbinary",
        ColumnType::MYSQL_TYPE_VARCHAR | ColumnType::MYSQL_TYPE_VAR_STRING => "varchar",
        ColumnType::MYSQL_TYPE_TINY_BLOB
        | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
        | ColumnType::MYSQL_TYPE_LONG_BLOB
        | ColumnType::MYSQL_TYPE_BLOB
            if binary =>
        {
            "blob"
        }
        ColumnType::MYSQL_TYPE_TINY_BLOB
        | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
        | ColumnType::MYSQL_TYPE_LONG_BLOB
        | ColumnType::MYSQL_TYPE_BLOB => "text",
        _ => return None,
    })
}

fn is_binary_charset(column: &Column) -> bool {
    column.character_set() == BINARY_CHARSET
}

/// How a column's values read as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueFormat {
    Text,
    /// Bytes that are not text: `BINARY`, `VARBINARY`, `BLOB` and `BIT` columns. Numbers and
    /// dates also report the binary character set, but they arrive as text.
    Binary,
}

impl ValueFormat {
    fn of(column: &Column) -> Self {
        let byte_type = matches!(
            column.column_type(),
            ColumnType::MYSQL_TYPE_STRING
                | ColumnType::MYSQL_TYPE_VAR_STRING
                | ColumnType::MYSQL_TYPE_VARCHAR
                | ColumnType::MYSQL_TYPE_TINY_BLOB
                | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                | ColumnType::MYSQL_TYPE_LONG_BLOB
                | ColumnType::MYSQL_TYPE_BLOB
                | ColumnType::MYSQL_TYPE_GEOMETRY
        );
        if column.column_type() == ColumnType::MYSQL_TYPE_BIT
            || (byte_type && is_binary_charset(column))
        {
            Self::Binary
        } else {
            Self::Text
        }
    }
}

fn cells(row: mysql_async::Row, formats: &[ValueFormat]) -> Row {
    row.unwrap_raw()
        .into_iter()
        .zip(
            formats
                .iter()
                .copied()
                .chain(std::iter::repeat(ValueFormat::Text)),
        )
        .map(|(value, format)| cell(value.unwrap_or(Value::NULL), format))
        .collect()
}

/// One value as text. The text protocol sends every value as the server formats it, so only
/// `NULL` and bytes arrive here; the other variants are for completeness.
fn cell(value: Value, format: ValueFormat) -> Cell {
    match (value, format) {
        (Value::NULL, _) => None,
        (Value::Bytes(bytes), ValueFormat::Binary) if bytes.len() > MAX_BINARY_HEX_BYTES => {
            Some(format!("<binary, {} bytes>", bytes.len()))
        }
        (Value::Bytes(bytes), ValueFormat::Binary) => Some(format!(
            "0x{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        )),
        (Value::Bytes(bytes), ValueFormat::Text) => {
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
        (Value::Int(number), _) => Some(number.to_string()),
        (Value::UInt(number), _) => Some(number.to_string()),
        (Value::Float(number), _) => Some(number.to_string()),
        (Value::Double(number), _) => Some(number.to_string()),
        (value @ (Value::Date(..) | Value::Time(..)), _) => {
            Some(value.as_sql(true).trim_matches('\'').to_owned())
        }
    }
}

/// Prepares `sql` and closes the statement again, so nothing runs: the server parses it and
/// resolves its names, as it would to run it.
pub(crate) async fn validate(conn: &mut Conn, sql: &str) -> Result<Option<Rejection>, DriverError> {
    match conn.prep(sql).await {
        Ok(statement) => {
            conn.close(statement).await.map_err(connection_error)?;
            Ok(None)
        }
        Err(mysql_async::Error::Server(error)) => Ok(Some(rejection(&error, sql))),
        Err(error) => Err(connection_error(error)),
    }
}

struct MysqlCursor {
    columns: Vec<ColumnInfo>,
    rows_changed: Option<u64>,
    exhausted: bool,
    /// Dropping it tells the statement task to discard the rest and release the connection.
    demand: Option<mpsc::Sender<usize>>,
    pages: mpsc::Receiver<Result<Page, DriverError>>,
    task: JoinHandle<Result<(), DriverError>>,
}

#[async_trait]
impl ResultCursor for MysqlCursor {
    fn columns(&self) -> &[ColumnInfo] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>, DriverError> {
        let Some(demand) = self.demand.as_ref().filter(|_| !self.exhausted) else {
            return Ok(Vec::new());
        };
        if demand.send(max_rows).await.is_err() {
            self.exhausted = true;
            return Ok(Vec::new());
        }
        match self.pages.recv().await {
            Some(Ok(page)) => {
                self.exhausted = page.exhausted;
                Ok(page.rows)
            }
            Some(Err(error)) => {
                self.exhausted = true;
                Err(error)
            }
            None => {
                self.exhausted = true;
                Ok(Vec::new())
            }
        }
    }

    fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Stops the statement task and waits for it, so the connection is free again. Reports
    /// an error the server sent while the rest of the result was discarded, a cancel's
    /// included.
    async fn finish(mut self: Box<Self>) -> Result<Option<u64>, DriverError> {
        self.demand = None;
        self.pages.close();
        (&mut self.task).await.map_err(connection_error)??;
        Ok(self.rows_changed)
    }
}
