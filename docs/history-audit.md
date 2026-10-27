# History as an audit trail

Status: **planned** (2026-10-27). This is the design of W4 in
[A well-rounded app](well-rounded.md#w4--history-as-an-audit-trail); the
[roadmap](roadmap.md#well-rounded-track) tracks it.

The query history (`history.sqlite`, see [Persistence](persistence.md#query-history)) already
gets every statement dbdesk sends: console runs, the grid's `UPDATE`s, `INSERT`s and
`DELETE`s, the object helpers' DDL, the statement behind **Export › All rows**, and what MCP
clients run through `dbdesk mcp`. Each row says whether it came from the app or from MCP, and
the History window searches one data source's statements. W4 makes the same file answer
"what changed, when, and from where", without a second log.

## What stays the same

- **One file, shared.** The app and every `dbdesk mcp` write the same file at once, in WAL
  mode, each connection waiting up to 5 s for another's write (`BUSY_TIMEOUT`), commits not
  synced (`synchronous = NORMAL`), and the switch to WAL retried. #82 and #113 learned those
  the hard way; nothing here changes them. Every new write is short: one row, or one batch of
  pruning.
- **An older dbdesk keeps working on a migrated file.** During an upgrade the old app or an old
  `dbdesk mcp` may still have the file open. Migrations only add nullable columns and indexes,
  so its `INSERT`s still work, and `source` keeps its two values (`app`, `mcp`), so its
  History window still parses every row.
- **Only statements, never result rows.** The grid's old values are the one exception: a cell
  edit's before and after values, which are what the audit is for.
- **Nothing is undone automatically.** The history shows the statement that would put a grid
  edit back; running it is up to the user, in a console.
- **Not tamper-proof.** The file is the user's, and can be edited or deleted. A real audit
  belongs on the server (`pgaudit`, SQL Server Audit, MySQL's audit plugins); the manual says
  so.

## The schema, version 2

`PRAGMA user_version` numbers the schema from now on (0 is every file before W4). Opening a
file below 2 migrates it in the immediate transaction `migrate` already uses, so two
processes opening an old file at once take turns:

```sql
ALTER TABLE history ADD COLUMN origin  TEXT;  -- console | grid | object | import | export | mcp
ALTER TABLE history ADD COLUMN detail  TEXT;  -- the MCP client's name, the imported file
ALTER TABLE history ADD COLUMN changes TEXT;  -- a grid edit's row change, JSON
CREATE INDEX history_by_time   ON history (executed_at DESC);
CREATE INDEX history_by_origin ON history (coalesce(origin, source), executed_at DESC);
```

- **Old rows aren't rewritten.** Updating every row would hold the write lock for as long as
  the file is big, and an older process keeps inserting rows without `origin` anyway. A row
  without one reads as `coalesce(origin, source)`: `mcp` is an MCP client whose name wasn't
  recorded, `app` is "dbdesk, before it recorded where".
- **Indexes** cover the searches the window makes: by data source and time (there already),
  by time alone (every data source), and by kind and time. The index builds are the slowest
  part of the migration: about a second per million rows, inside the lock, well within
  another process's 5 s wait for the sizes a history reaches.

## W4a — Where each statement came from

`HistorySource` becomes the typed provenance, written to `origin` and `detail` (and `source`,
for older readers):

| Variant | Recorded by | `source` |
| --- | --- | --- |
| `Console` | a console's run | `app` |
| `GridEdit(RowChange)` | Submit in the grid, one per row (see W4c) | `app` |
| `ObjectHelper` | `ApplyDdl`: the explorer's create, alter, drop, rename and copy dialogs | `app` |
| `Import { file }` | Import CSV, one entry per import | `app` |
| `Export` | Export › All rows, the statement it ran | `app` |
| `Mcp { client }` | `dbdesk mcp`, with the client's name from its `initialize` | `mcp` |
| `Dbdesk` | rows written before W4, or by an older dbdesk | `app` |

- **Importing** isn't recorded today. It gets one entry, not one per 500-row batch: the
  `INSERT`'s table and columns (`INSERT INTO "orders" ("id", "name") VALUES …`), the rows
  imported, the time it took and how it ended, and the file in `detail`. The rows themselves
  are the file's, not the history's.
- **The MCP client's name** is the `clientInfo.name` it sends when it connects (for example
  `claude-ai`). The server reads it once, on `initialized`.
- **The History window** shows a badge per kind (MCP with the client's name on hover), and its
  menu filters by kind: every source, consoles, grid edits, object helpers, imports, exports,
  MCP clients, and earlier dbdesk.

## W4b — Secrets stay out

The history is a plain file, so a password must never reach it, whoever sends the statement.
`History::record` is the one place every writer goes through (the app's engine and
`dbdesk mcp`), so the store redacts there, before the row is written:

- **Password clauses**: a string literal after `PASSWORD` (PostgreSQL roles, SQL Server
  logins, MySQL's `SET PASSWORD`), `IDENTIFIED BY` / `IDENTIFIED WITH … BY` (MySQL, MariaDB,
  ClickHouse), and DuckDB's `CREATE SECRET` (`SECRET`, `KEY_ID`, `TOKEN`, …), up to the end of
  the clause, becomes `'<redacted>'`.
- **Connection strings** in any literal: `password=…` and `pwd=…` (libpq and ODBC style, as
  `dblink`, `postgres_fdw` user mappings and DuckDB's `ATTACH` take them), and the password in
  a URL's `user:password@`.
- The scanner knows quoted identifiers, `--` and `/* */` comments, and `''`, `N'…'`, `E'…'`
  and `$tag$…$tag$` literals, so a name or a comment containing `password` isn't touched. It
  errs on the side of hiding: a literal it isn't sure about is redacted.
- **A statement put back** from the history then has `'<redacted>'` in place of the secret,
  which the database refuses rather than setting a wrong password.
- **The guard:** a store test records statements with a known secret in every form above,
  through every writer's path, and then reads the database file and its WAL as bytes: the
  secret must be in neither. An engine test does the same through a console run and
  `ApplyDdl`, and the log lines `ApplyDdl` writes use the same redaction.

## W4c — Grid edits with the values they replaced

Today the grid sends `ApplyEdits { statements: Vec<String> }`, and the history gets each
`UPDATE` with its new values only. The app knows more when it builds the statements: the row
as loaded. So each statement now travels with its change, built in the same place
(`PendingEdits::statements`):

```rust
pub struct RowChange {
    /// The table, as the statement names it.
    pub table: String,
    pub kind: RowChangeKind,
    /// The statement that would put the row back, in the data source's dialect.
    pub revert: Option<String>,
}

pub enum RowChangeKind {
    /// `key` is the row's primary key as loaded.
    Updated { key: Vec<ColumnValue>, cells: Vec<ReplacedValue> },
    /// The whole row, as loaded.
    Deleted { key: Vec<ColumnValue>, row: Vec<ColumnValue> },
    /// The values typed; the database chose the rest.
    Inserted { values: Vec<ColumnValue> },
}
```

- Stored as JSON in `changes`, through serde, already a dependency of the store.
- **The statement to put it back** is built by `dbdesk_sql::RowSql`, like the edit itself:
  an `UPDATE` setting the old values, keyed by the new key when the edit changed the key; an
  `INSERT` of the deleted row. An inserted row has none: its generated key isn't known.
- **The History window** gets a detail pane for the selected entry: the whole statement, where
  it came from, and for a grid edit a table of each column's old and new value, with **Copy**
  and **Insert into console** for the statement that puts it back.

## W4d — Searching every data source

`HistoryQuery` grows to:

- `data_source: Option<DataSourceId>`, `None` for every data source in the file;
- `text`, as today: a case-insensitive substring of the SQL;
- `kind: Option<HistorySourceKind>`;
- `since` and `until`, an optional time range.

The window adds **This data source / Every data source** and a time menu (any time, the last
hour, 24 hours, 7 days, 30 days). Searching every data source shows each entry's data source
by name, or *Removed data source* for one no longer saved.

**Why not SQLite's full-text search.** The bundled SQLite has FTS5 with the trigram tokenizer,
which matches substrings. On a generated history of a million statements (250 MB) it was the
wrong trade:

| Search, 1M rows | `LIKE` and indexes | FTS5 trigram |
| --- | --- | --- |
| a word in most statements | 1 ms | 430 ms |
| a word in none, every data source | 1.2 s | 2 ms |
| a word in none, one data source | 46 ms | — |
| file size | 250 MB | 515 MB |
| building the index on upgrade | — | 10 s, holding the write lock |

The last line rules it out: another process opening the file would wait out its 5 s and fail.
The search stays `LIKE`, newest first, stopping at 500 matches, on the indexes above; a search
of every data source for something rare in a very large history takes about a second, off the
UI thread. Retention keeps it from growing without bound.

## W4e — Retention

`settings.toml` gets a `[history]` table, set in View › Settings:

```toml
[history]
max_age_days = 365       # absent: no limit
max_per_data_source = 50000
```

- **Off by default**, as today: nothing is deleted until the user asks.
- **Applied by the app** at start-up, and when the setting is saved, on the history thread.
  `dbdesk mcp` doesn't prune; it only adds rows.
- **Safe beside other processes.** Pruning deletes in batches of 2,000 rows, each its own
  short transaction, so another process's write never waits more than a few milliseconds,
  and a crash halfway leaves a consistent file with fewer rows to prune next time. By age it
  deletes rows older than the cut-off, on `history_by_time`; by count, for each data source it
  finds the row at the limit on `history_by_source` and deletes the older ones.
- Freed pages are reused; the file isn't vacuumed, which would need every other process to
  close it.

## W4f — Export

**Export…** in the History window writes the current search, without its 500-row limit, as
CSV or JSON through the store's `ExportWriter`: time, data source (name and id), kind, detail,
outcome, duration, rows, the SQL, and a grid edit's change as JSON. The engine streams it
from the history thread, a page at a time, so a large history never sits in memory.
`Command::ExportHistory` is answered by `Event::HistoryExported`.

## Tests

- **Store:** each kind round-trips; a version-0 file (with and without `source`) migrates,
  and an old process's inserts after the migration read as `Dbdesk`; two processes open and
  write an old file at once (the existing child-process test, starting from a version-0
  file); search by every filter; pruning by age and by count, and two processes pruning and
  writing at once; export of both formats; the secrets guard.
- **Engine** (SQLite, no servers needed): each recorder's kind (console, grid edit with old
  values, `ApplyDdl`, import, export); retention at start-up; export of a search.
- **MCP:** the client's name lands in the history.
- **UI (kittest):** the kind badges and filter, the detail pane of a grid edit with its old
  values and the statement to put it back, searching every data source, the retention
  settings, and the export.

None of them needs a database server, so they add seconds, not minutes, to CI.

## Layers

One stack, each layer a pull request:

1. This plan.
2. **W4a** provenance: the version-2 schema, `HistorySource`, the import entry, the MCP
   client's name, the badges and the kind filter.
3. **W4b** secrets: the redaction and its guard.
4. **W4c** grid edits' old values, the statement to put them back, and the detail pane.
5. **W4d** search across data sources and time.
6. **W4e** retention.
7. **W4f** export.
8. The manual's chapter and the demo note.
