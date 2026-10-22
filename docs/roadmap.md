# Roadmap

Each milestone ends with something usable and a short demo note in this file. Milestones are
sized for evenings-and-weekends pace; the order matters more than the dates.

## M0 — Foundations

- Cargo workspace with the crates from [Architecture](architecture.md), mostly empty.
- CI (fmt, clippy `-D warnings`, test).
- `eframe` window with an `egui_dock` layout: explorer (left), console tabs (centre),
  log console (bottom). Layout persisted.
- `tracing` → log console pipeline working with app events.
- Engine runtime started, `Command`/`Event` channels wired, repaint on event.

**Exit:** the app opens, shows three panels, logs "engine started" in the log console.

**Demo note:** done. `cargo run -p dbdesk-app` opens the docked layout, and the status bar shows
"Engine running". The log console shows "engine started" with level/search filters, and the
same entries go to `logs/dbdesk.<date>.log`. The layout survives restarts; **View → Reset layout**
restores the default. An egui_kittest test drives the whole app headlessly.

## M1 — Data sources and sessions

- New / edit / delete data source dialog for Postgres and SQLite.
- `data_sources.toml` read/write, passwords in keychain.
- Connect, disconnect, test connection; status dot in the tree.
- Connection lifecycle in the log console.
- Driver conformance suite started (connect, simple query).

**Exit:** I can add my local Postgres and a SQLite file, connect to both, and see it logged.

**Demo note:** done. In the explorer, **+** opens the data source dialog for PostgreSQL or
SQLite, with **Test connection**. Data sources are saved to `data_sources.toml` and passwords to
the OS keychain. Double-click (or the context menu) connects and disconnects. The status dot
shows disconnected / connecting / connected / failed, and the tooltip shows the server version
or the error. Connection lifecycle and timings appear in the log console. Covered by driver
conformance tests against a real Postgres 17 and SQLite, headless engine tests, and a UI test
that adds a SQLite data source through the dialog.

Follow-ups left out of M1:

- TLS for Postgres (`sslmode`). Servers that require TLS currently reject the connection.
- A file picker for SQLite paths.
- Prompting for a password when none is saved or the keychain is unavailable.
- Noticing when an established connection drops (the session still shows "connected").
- Duplicate data source; colour and safety level per data source.
- Preserving comments in `data_sources.toml` (`toml_edit`).

## M2 — Connection explorer

- Lazy catalog tree down to columns, indexes, constraints for both drivers.
- Refresh subtree, filter loaded nodes, hide system schemas.
- Show definition for views and functions.

**Exit:** I can browse a real work schema without waiting on the UI.

**Demo note:** done. Expanding a data source connects it and loads the tree one level at a time,
with a spinner per loading node. PostgreSQL shows databases (others open their own connection
on demand), schemas, tables, views, materialized views, functions and sequences, then columns,
indexes, constraints and triggers per table. SQLite shows schemas, tables and views, then
columns, indexes, foreign keys and triggers. The filter box matches loaded nodes, the
**System** checkbox shows system schemas and tables, Refresh reloads a subtree, and Show
definition opens the source in a read-only window. Covered by a catalog walk over a fixture with
every object kind on both drivers, an engine test that connects on first expand, and a UI test
that clicks down to a table's columns.

Follow-ups left out of M2:

- `CREATE TABLE` DDL for Postgres tables (no `pg_get_tabledef`; see the explorer doc's open
  questions).
- Counts on folders, and row estimates on tables.
- Keyboard navigation and selection in the tree.
- A per-data-source "show system objects" setting (it is global now).
- Detecting dropped connections (still from M1).

## M3 — SQL runner

- Query console tabs with highlighting and statement-under-cursor.
- Run statement / selection / script, with `dbdesk-sql` splitting.
- Streaming result grid with paging and `FetchMore`.
- Cancellation for both drivers.
- Errors with position underline; Postgres notices in the log console.
- Safety confirmations for `UPDATE`/`DELETE` without `WHERE`.

**Exit:** I use dbdesk instead of `psql` for a day of ordinary queries.

## M4 — Daily-driver polish

- Console files persisted; reopen where I left off.
- Query history with search.
- Manual transaction mode.
- Copy as TSV, export CSV/JSON.
- `EXPLAIN` action.

**Exit:** I use dbdesk instead of DataGrip for a week, and keep a list of what I missed.

## M5 — Personalisation

Picked from the "what I missed" list, likely:

- `keymap.toml`, themes, fonts.
- Custom catalog actions (`actions.toml`) and snippets.
- Autocompletion from the cached catalog.
- SSH tunnels (`russh`).
- Editing data in the grid.
- MySQL driver; a driver for the `db-data-structures` engine.
