# Power in the grid

Status: **planned** (2026-10-26). This is the design of W5 in
[A well-rounded app](well-rounded.md#w5--power-in-the-grid); the
[roadmap](roadmap.md#well-rounded-track) tracks it.

The grid shows a statement's rows a page at a time, sorts and filters what has loaded, exports
it, and edits cells of a single table's rows into `UPDATE`s (M5). W5 makes it the place where a
table's data is worked on: adding and deleting rows, following foreign keys, sorting and
filtering every row in the database, exporting every row, and importing a CSV.

## What stays the same

- **Results are text** ([ADR 0004](adr/0004-results-as-text.md)). Every value goes back as a
  string literal (or `NULL`), and the database turns it into the column's type, as cell edits
  already do. Nothing here needs typed values.
- **Which rows are editable** is unchanged: a `SELECT` from one table, without joins, grouping
  or `DISTINCT`, whose result holds the table's whole primary key. Inserting and deleting need
  exactly what editing does.
- **Safety levels gate every write the same way** ([W2](well-rounded.md#w2--safety-per-data-source)):
  a read-only data source's grid isn't editable and the engine refuses its writes, imports
  included; a careful one asks first, naming the data source and what will change.
- **The engine runs nothing the user didn't review or ask for.** Rows are written by Submit, an
  import by Import, and a query runs again only when the user turns on sorting in the
  database, follows a key, or exports every row.

## The SQL, in one place

The statements W5 writes are built in a new `dbdesk_sql::dml` module, next to the object
helpers' DDL, so they are tested per dialect without a database and the engine tests run the
very statements the app sends:

- `UPDATE`, `INSERT` and `DELETE` for one row, by its primary key as loaded. Today's `UPDATE`
  builder moves here from the app.
- An `INSERT` of a row with every column left to its default, which dialects spell
  differently: `DEFAULT VALUES` (PostgreSQL, SQLite, SQL Server, DuckDB), `() VALUES ()`
  (MySQL and MariaDB).
- `SELECT * FROM table WHERE key = …` for following a foreign key either way.
- The sorted and filtered wrapper around a query (see below), and multi-row `INSERT`s for
  importing.

Names are quoted with the dialect's quoting, and values written as its string literals
(`N'…'` on SQL Server, backslashes doubled on MySQL), as cell edits already are.

## W5a — Inserting and deleting rows

- **Add row** in the grid's toolbar adds an empty row under the loaded ones. Each cell shows
  what the database will put there if left alone: `DEFAULT` with the column's default on
  hover, or `NULL`. Typing into a cell sets it; **Set NULL** sets `NULL` on purpose. Columns
  the result doesn't show are left to their defaults.
- **Delete** marks the selected rows (from the context menu, or the toolbar button for the
  selected cells' rows). A marked row is struck through and tinted; **Restore row** unmarks it.
  Deleting a row drops the edits typed into it.
- **Submit** writes everything in one go, all or none, in the order that can't trip over
  itself: the `DELETE`s, then the `UPDATE`s, then the `INSERT`s, so a key freed by a delete can
  be reused. Each statement must change exactly one row, as edits must today. Inside an open
  transaction the changes stay part of it, behind a savepoint, as edits do.
- **After Submit** deleted rows leave the grid and edited cells keep their new values. If rows
  were added, the statement runs again, so the values the database chose (a generated id, a
  default timestamp) show: guessing them would be wrong.
- **A failure** names the row it happened on (*Row 12* or *New row 2*), and everything stays
  pending to fix and submit again.
- **The toolbar counts what is pending:** *2 changed cells · 1 new row · 1 deleted row*, with
  Submit and Revert.
- **Careful data sources** ask *Apply edits to "production"?* with what will happen: *this
  inserts 1 row, updates 2 edited rows and deletes 1 row.*
- **The history records every statement the grid sends**, inserts, updates and deletes alike,
  as it records the explorer's DDL. (W4 will mark them as grid edits and keep the values they
  replaced.)

## W5b — Following foreign keys

- **The table description learns foreign keys,** both ways: the keys the table has
  (`foreign_keys`), and the keys of other tables that point at it (`referenced_by`), each with
  its columns in order. Every driver fills them from its catalog: `pg_constraint`, SQLite's
  `pragma_foreign_key_list`, MySQL's `KEY_COLUMN_USAGE`, SQL Server's `sys.foreign_keys`, and
  DuckDB's `duckdb_constraints()`. ClickHouse has no foreign keys, so both stay empty.
- **A single table's result keeps its description even when it can't be edited** (no primary
  key, say), since following a key doesn't need one.
- **A value in a foreign key column is a link:** underlined in the accent colour. ⌘-click (Ctrl
  on Windows and Linux), or **Open customers row** in its context menu, opens a console on the
  referenced table, selecting the row by the referenced columns, and runs it. A `NULL` in any of
  the key's columns refers to nothing, so it isn't a link.
- **Referencing rows ›** in a row's context menu lists the tables whose keys point at this one
  (*orders.customer_id*), and opens the matching rows the same way. An entry is disabled, saying
  why, if the result doesn't hold the columns the key refers to.
- Each opens a new console tab rather than replacing the result, so the tab you came from is the
  way back.

## W5c — Sorting and filtering in the database

- **A Sort and filter in the database switch** next to the grid's filter, off by default, as the
  [decisions](well-rounded.md#decisions) say. While on:
  - clicking a column header, or pressing Enter in the filter, runs the statement again, wrapped:

    ```sql
    SELECT * FROM (<the statement>) AS dbdesk_rows
    WHERE LOWER(CAST(dbdesk_rows."name" AS text)) LIKE '%ada%' ESCAPE '\' OR …
    ORDER BY 2 DESC
    ```

  - The filter keeps its meaning: rows where any value contains the text, ignoring case. Each
    dialect casts to its own text type (`text`, `CHAR`, `NVARCHAR(MAX)`, `VARCHAR`, `String`),
    and `%`, `_` and the escape character in the text are escaped.
  - The sort is by position, so it works whatever the column is called.
  - The toolbar says *Sorted and filtered by the database*, and rows page in as usual.
- **The grid stays editable.** It keeps the statement as typed to decide what can be edited,
  since the wrapper selects the same table's rows.
- **Which statements can be wrapped:** those that start with `SELECT`, `WITH` or `VALUES`, whose
  result names each column once (otherwise a filter couldn't tell them apart). Elsewhere the
  switch is disabled and says why. A trailing semicolon is dropped. SQL Server refuses `WITH`
  inside a subquery and an `ORDER BY` there without `TOP` or `OFFSET`: the first disables the
  switch, and the second's error is shown as the database gives it.
- **Only the shown statement runs again.** Turning the switch off runs the statement as typed.

## W5d — Exporting every row

- **Export** offers **Loaded rows** (today's: the rows shown, as sorted and filtered) and **All
  rows**, each as CSV or JSON.
- **All rows runs the statement again on the console's own connection**, so it sees the
  console's open transaction and temporary tables, and pages through it, writing each page as it
  arrives: the rows are never all in memory. With sorting in the database on, the wrapped
  statement is what runs, so the file has the grid's order and filter.
- **Running it again needs the connection,** so a result still loading more rows stops loading:
  the rows already in the grid stay, as when another statement runs.
- **The toolbar shows progress** (*Exporting… 125,000 rows*) with **Cancel**, which stops the
  statement and removes the partial file. The file is written beside its final name and renamed
  when complete, so a failed export never leaves half a file under the name chosen.
- **Read-only data sources** export as they run statements: in the database's read-only mode.
- The store's writers become streaming (`ExportWriter`: create, write a page, finish), and the
  loaded-rows export writes through the same writer, so both write identical files.

## W5e — Importing a CSV

- **Import CSV…** on a table in the explorer (where the object helpers would be offered, and not
  on a read-only data source) opens a dialog:
  1. **The file**, chosen with the platform's dialog, with **First line is a header** (on) and
     the separator (comma, semicolon or tab).
  2. **A preview** of the first 20 records, and a **mapping** from each of the table's columns to
     a column of the file or to *(default)*. Columns are matched by header name ignoring case,
     else by position.
  3. **Type checks against the preview:** a value that can't be the column's type (*'abc' isn't
     a number*), or an empty field for a `NOT NULL` column without a default, is flagged with
     its line. They are warnings: the database has the last word.
- **Import** reads the file again in the engine, streaming, and inserts on the explorer's
  connection in batches of 500 rows (one multi-row `INSERT` each), all in one transaction.
- **On the first failure everything is rolled back,** and the dialog shows the database's
  message with the line number. Inside a batch the failing line is found by retrying that batch
  a row at a time behind a savepoint; DuckDB, which has no savepoints, gives the batch's range
  of lines instead.
- **Empty fields are `NULL`**, so a number column can be left empty. Every other field goes in as
  a string literal, which the database casts to the column's type.
- **Careful data sources** ask before importing; **read-only** ones never offer it, and the
  engine refuses it anyway.
- **The CSV reader** is RFC 4180 (quoted fields, doubled quotes, line breaks inside quotes, CRLF
  or LF), the counterpart of the export's writer, in `dbdesk-store`. It counts lines, not
  records, so a reported line is where the record starts in the file. No new dependency.

## Engine commands

| Command | Event | Session |
| --- | --- | --- |
| `ApplyEdits` (now inserts and deletes too, each recorded in the history) | `EditsApplied` | the console's |
| `DescribeTable` (now with foreign keys both ways) | `TableDescribed` | the explorer's |
| `ExportAllRows { console, run, sql, path, format }` | `ExportProgress`, then `ExportFinished` | the console's |
| `ImportRows { data_source, table, file, mapping }` | `ImportProgress`, then `RowsImported` | the explorer's |

Following a key and sorting in the database are runs like any other (`Execute`).

## Testing

- **`dbdesk_sql::dml`** golden tests per dialect: each statement for all six dialects, with
  names and values that need quoting.
- **Engine tests on every engine with a test server** (SQLite and DuckDB always; PostgreSQL,
  MySQL, MariaDB and SQL Server from `docker compose`): a submit that deletes, updates and
  inserts, and one that fails and leaves nothing changed; foreign keys both ways in the table
  description; a wrapped query that sorts and filters; an export of more rows than a page, in
  both formats, and a cancelled one; an import that succeeds and one that fails at a known line.
  ClickHouse runs the wrapped query and the export, which need no keys.
- **kittest UI tests** on SQLite: adding, editing and deleting in one submit; a careful data
  source's confirmation; following a key and back through referencing rows; the database sort
  and filter; exporting all rows; the import dialog's mapping, warnings and failure.
- **Store tests** of the streaming writers and the CSV reader (quotes, line breaks, line
  numbers).

## Order

Each layer is one pull request, in a stack: this plan, then W5a to W5e, then the manual chapter
and the demo note.

## Later

- Recording imports in the history, once W4 marks where statements came from.
- Typed values for inserted rows, which would show defaults without running the statement again.
- A free-form `WHERE` in the database filter, next to the text search.
- Importing into a new table, and other formats (JSON, Parquet through DuckDB).
