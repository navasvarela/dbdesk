# Connection explorer

The left-hand panel. It lists data sources and lets you browse what each one contains.

## Data sources

A data source is a saved `ConnectionConfig`. M1 implements name, host/port/database/user,
password, and SQLite file + read-only; D2 added the colour and DB0 the TLS mode and CA file
(`TlsMode`, libpq's `sslmode` names, on rustls). The other rows are still planned:

| Field | Postgres | SQLite |
| --- | --- | --- |
| name, colour, safety level | ✓ | ✓ |
| host, port, database, user | ✓ | — |
| file path, read-only flag | — | ✓ |
| password | keychain | — |
| SSL mode, root cert | ✓ | — |
| default schema / search_path | ✓ | — |
| connect timeout, application_name | ✓ | — |

Dialog flow: **+** (New data source) → pick driver → form → **Test connection** (runs `TestConnection`; the result
is shown inline and in the log console) → **Save**. The dialog stays open until the engine
confirms the save, and shows the error if it fails. Edit and delete (with confirmation) are in
the context menu, as are Connect and Disconnect. Duplicate is still to do; it's handy
for "same server, other database".

- An empty name defaults to `database@host`, or the file name for SQLite.
- Leaving the password empty keeps the saved one. Typing one replaces it.
- **Browse…** opens the system's file dialog to pick an existing database. It doesn't filter
  by extension: SQLite files use many extensions or none, and on macOS a filter can't offer an
  "All files" option.
- **New…** opens the system's Save dialog. The engine then creates an empty database there
  (`Command::CreateSqliteDatabase`), writing the SQLite header straight away so other tools
  recognise the file. It refuses to overwrite an existing file, even if the Save dialog asked
  "Replace?" and you agreed.
- Connecting never creates a file: a typo in the path is an error, not a new empty database.
  Creating a database is always the explicit **New…** step.
- Both dialogs block the UI while open, like the export's save dialog. They are the exception
  to "the UI never waits": they are modal anyway, and macOS requires them on the main thread.

## Tree

```
▸ my-postgres ●                   data source, status dot: none/amber/green/red
  ▸ app_db                        database
    ▸ public                      schema
      ▸ tables (12)               folder
        ▸ users                   table
          ▸ columns
          ▸ indexes
          ▸ constraints
          ▸ triggers
      ▸ views
      ▸ materialized views
      ▸ functions
      ▸ sequences
▸ local.sqlite ●
  ▸ main                          schema (plus any attached databases)
    ▸ tables
      ▸ orders
        ▸ columns / indexes / constraints (foreign keys) / triggers
    ▸ views
```

Loaded folders show a count badge (`tables 4`). Every row has an icon for its driver or node
kind, coloured by kind; see [look-and-feel.md](look-and-feel.md#icons-and-colour). Columns show their type and `not null` / `primary key` next to
the name; indexes and constraints show their kind.

- **Lazy loading**: expanding a node sends `LoadChildren { id, path }`. Children appear when
  `ChildrenLoaded` arrives; until then the node shows a spinner. Nothing is introspected up front.
  Expanding a disconnected data source connects it first: the engine starts a session and
  queues the request until it is connected.
- **Paths**: a node is identified by a `CatalogPath`, the list of `(kind, name)` segments from
  the data source down, e.g. `[Database app, Schema public, Folder tables, Table users]`.
  Drivers match on the path's shape. Functions are named with their argument list
  (`add(integer, integer)`) so overloads are distinct.
- **Caching**: loaded children are kept in a `CatalogTree` per data source and reused until the
  user hits **Refresh** on that node (reloads its subtree and collapses its descendants), or the
  connection closes or fails (which forgets the whole tree).
- **Filtering**: a filter box above the tree matches loaded node labels, case-insensitively. A
  matching node is shown with its whole subtree. Its ancestors are shown and forced open even if
  they don't match themselves. It never triggers loading of unloaded subtrees (that would hit
  the server for every keystroke).
- **System objects**: `pg_catalog`, `information_schema`, `pg_toast*`, `pg_temp_*`, SQLite's
  `temp` schema and `sqlite_*` tables are hidden by default. The eye button in the toolbar
  ("Show system schemas and tables") shows them. It is global for now, not per data source.

## Introspection queries

Postgres reads `pg_catalog` directly rather than `information_schema`: it is faster, exposes
materialized views, and `information_schema` hides objects the user cannot access, which is
confusing in a browsing tool.

| Node | Postgres source | SQLite source |
| --- | --- | --- |
| databases | `pg_database` (connectable, not templates) | — |
| schemas | `pg_namespace` | `pragma_database_list` (main + attached) |
| tables/views/mat. views/sequences | `pg_class` by `relkind` | `sqlite_schema` by `type` |
| functions | `pg_proc` (`prokind` f/p/w; aggregates left out) | — |
| columns | `pg_attribute` + `format_type` | `pragma_table_info` |
| indexes | `pg_index` | `pragma_index_list` |
| constraints | `pg_constraint` | `pragma_foreign_key_list` (grouped by id; FKs have no names) |
| triggers | `pg_trigger` (not internal) | `sqlite_schema` where `type = 'trigger'` |
| definitions | `pg_get_viewdef`, `pg_get_functiondef`, `pg_get_indexdef`, `pg_get_constraintdef`, `pg_get_triggerdef` | the stored `CREATE` statement in `sqlite_schema` |

Postgres binds a connection to one database, so browsing a database other than the configured
one opens an extra connection to it inside the same `PostgresConnection`, on first use, using the
same credentials. It is kept until disconnect. The engine and UI never see this; the log console
shows "opened a connection for browsing another database".

## Actions (context menu)

Implemented (M2):
- Data source: Connect / Disconnect, Refresh, Edit…, Delete…. Double-click or the arrow
  expands (and connects).
- Any node with children: Refresh.
- Schema objects, columns, constraints, triggers: Copy name (`public.users`, quoted when needed).
- Views, materialized views, functions, indexes, constraints, triggers (and SQLite tables): Show
  definition, in a floating read-only window with a Copy button.

Planned (M3):
- New query console (bound to this data source, default schema set to the selected one)
- Open table data → new console with `SELECT * FROM <table>` and run it
- Open a definition in a query console instead of the read-only window

Later (H1 in [Hackability](../hackability.md)): custom actions from `actions.toml`, e.g.

```toml
[[action]]
name = "Row count"
applies_to = ["table", "view"]
sql = "SELECT count(*) FROM {qualified_name}"
run = true
```

## Open questions

- Generating full `CREATE TABLE` DDL for Postgres is non-trivial (no `pg_get_tabledef`).
  Defer, or shell out to `pg_dump --schema-only -t`?
  Proposed answer: rebuild it from `describe_table` and the loaded constraints, through the DDL
  builder of [Creating and changing database objects](../entity-helpers.md) (E1).
- Should the tree show row-count estimates (`reltuples`) next to tables? Cheap on Postgres,
  expensive on SQLite.
