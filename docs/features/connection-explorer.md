# Connection explorer

The left-hand panel. It lists data sources and lets you browse what each one contains.

## Data sources

A data source is a saved `ConnectionConfig`. M1 implements name, host/port/database/user,
password, and SQLite file + read-only. The other rows are still planned:

| Field | Postgres | SQLite |
| --- | --- | --- |
| name, colour, safety level | ✓ | ✓ |
| host, port, database, user | ✓ | — |
| file path, read-only flag | — | ✓ |
| password | keychain | — |
| SSL mode, root cert | ✓ | — |
| default schema / search_path | ✓ | — |
| connect timeout, application_name | ✓ | — |

Dialog flow: **+** → pick driver → form → **Test connection** (runs `TestConnection`; the result
is shown inline and in the log console) → **Save**. The dialog stays open until the engine
confirms the save, and shows the error if it fails. Edit and delete (with confirmation) are in
the context menu. Double-click connects or disconnects. Duplicate is still to do; it's handy
for "same server, other database".

- An empty name defaults to `database@host`, or the file name for SQLite.
- Leaving the password empty keeps the saved one. Typing one replaces it.
- SQLite files are never created: a typo in the path is an error, not a new empty database.
  A file picker is still to do; for now, paste the path.

## Tree

```
▸ my-postgres (●)                 data source, status dot: grey/amber/green/red
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
▸ local.sqlite (●)
  ▸ main
    ▸ tables
    ▸ views
    ▸ indexes
```

- **Lazy loading**: expanding a node sends `Introspect { node }`. Children appear when
  `CatalogLoaded` arrives; until then the node shows a spinner. Nothing is introspected up front.
- **Caching**: loaded children are kept in a `CatalogTree` per data source and reused until the
  user hits **Refresh** on that node (refreshes the subtree) or reconnects.
- **Filtering**: a filter box above the tree matches loaded nodes by name. It does not trigger
  introspection of unloaded subtrees (that would hit the server for every keystroke).
- **Hidden schemas**: `pg_catalog`, `information_schema` and `pg_toast*` are hidden by default,
  toggle per data source.

## Introspection queries

Postgres reads `pg_catalog` directly rather than `information_schema`: it is faster, exposes
materialized views, and `information_schema` hides objects the user cannot access, which is
confusing in a browsing tool.

| Node | Postgres source | SQLite source |
| --- | --- | --- |
| databases | `pg_database` (not templates) | attached DBs via `pragma_database_list` |
| schemas | `pg_namespace` | — (database = schema) |
| tables/views | `pg_class` by `relkind` | `sqlite_schema` |
| columns | `pg_attribute` + `format_type` | `pragma_table_info` |
| indexes | `pg_index` + `pg_get_indexdef` | `pragma_index_list` |
| constraints | `pg_constraint` + `pg_get_constraintdef` | `pragma_foreign_key_list` |
| functions | `pg_proc` | — |

Postgres connects to one database per connection, so expanding a second database opens a
second session behind the scenes. The tree hides that detail; the log console shows it.

## Actions (context menu)

v1:
- Connect / Disconnect / Refresh
- New query console (bound to this data source, default schema set to the selected one)
- Open table data → new console with `SELECT * FROM <table>` and run it
- Copy qualified name
- Show definition (views and functions via `pg_get_viewdef` / `pg_get_functiondef`;
  SQLite stores the original `CREATE` statement)

Later (M5): custom actions from `actions.toml`, e.g.

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
- Should the tree show row-count estimates (`reltuples`) next to tables? Cheap on Postgres,
  expensive on SQLite.
