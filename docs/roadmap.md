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

**Demo note:** built; the exit criterion (a day of real use) is still to be tried. Right-click a
data source → **New query console**, or a table → **Open data**. Each console has its own
connection:
- **Editing:** SQL is highlighted by `dbdesk-sql`'s lexer.
- **Running:** ⌘↵ runs the selection or the statement at the cursor; ⇧⌘↵ runs everything,
  stopping at the first failure. ⌘. cancels on both drivers.
- **Results:** the grid shows 500 rows; **Load more rows** or scrolling to the bottom loads the
  next 500.
- **Errors:** the error's position is underlined in the editor.
- **Safety:** `UPDATE`/`DELETE` without `WHERE` asks "Run anyway?".
- **Notices:** PostgreSQL notices appear in the log console.

Covered by:
- Driver checks for execution (paging, NULL, abandoning a result, error positions) and
  cancellation on PostgreSQL 17 and SQLite.
- Headless console engine tests (scripts, paging, failures, cancel, a new run discarding unread
  rows).
- Console state tests, and a UI test that opens a table's data and loads the next page.

Writing that UI test found a real bug: "Load more rows" was drawn below a grid that takes all
the space, so it couldn't be clicked; it now sits in the summary line. A stress run found a
cancel race that left endless SQLite queries running; see "Hard-learned" in the SQL runner doc.

Follow-ups left out of M3 (most belong to M4 anyway):

- Explain (⌥⌘E), manual transactions, persisted consoles, query history, export.
- Cell selection and copying ranges, a cell viewer, client-side sort and filter.
- Line numbers, current-statement highlight, completion.
- Per-data-source safety levels (confirm any change on production).
- Positions for SQLite's "no such table/column" errors (SQLite doesn't report them).

## M4 — Daily-driver polish

- Console files persisted; reopen where I left off.
- Query history with search.
- Manual transaction mode.
- Copy as TSV, export CSV/JSON.
- `EXPLAIN` action.

**Exit:** I use dbdesk instead of DataGrip for a week, and keep a list of what I missed.

**Demo note:** built; the exit criterion (a week of real use) is still to be tried.
- **Consoles are files:** each console is `consoles/<data-source-id>/console.sql` (then
  `console_2.sql`, …), saved a second after typing stops and on quit. Quit with consoles open
  and they come back with their text. Closed consoles you typed in are under the data source's
  **Open saved console** menu.
- **History:** **History** in a console searches every statement run on its data source, with
  time, outcome, duration and rows; **Insert** puts one back at the cursor.
- **Transactions:** switch a console to **Manual commit**, run an `INSERT`, and the toolbar
  shows "● Transaction open" until **Commit** or **Rollback**. Closing the console,
  disconnecting or quitting first asks whether to roll it back.
- **Explain:** ⌥⌘E (or **Explain**) shows the plan of the statement at the cursor.
- **Grid:** click, shift-click or drag to select cells, ⌘C copies them as TSV; click a header to
  sort and type to filter the loaded rows (labelled as such); double-click or Space opens a
  cell in a viewer that pretty-prints JSON; **Export → CSV…/JSON…** saves the rows as shown.
- **Editor:** line numbers, and the statement ⌘↵ would run is shaded.

Covered by:
- Store tests for console files, the history (ordering, `LIKE` escaping) and CSV/JSON export.
- A driver check for transaction status on PostgreSQL 17 and SQLite.
- Headless engine tests: console files across a restart, history records and search, manual
  mode with rollback, a user's own `BEGIN` and disconnecting, committing a failed PostgreSQL
  transaction, and exports.
- Console and grid state tests (sorting, filtering, selection, TSV, statement insertion,
  Explain error positions), and UI tests that restore a console from its file and save an
  edit, insert from the history, roll back in manual mode and confirm closing, explain, and
  sort, view and export rows.

Writing those UI tests found two older layout bugs: the default layout gave the explorer 80% of
the width (egui_dock's split fraction is the first node's share), and in a short console the
resizable results panel drew its contents under the editor, which took their clicks. Editor
and results are now split by hand.

Follow-ups left out of M4:

- Renaming consoles, and deleting saved ones from the UI.
- Pruning old history entries; searching all data sources at once.
- "Commit and close" in the rollback warning.
- Sorting and filtering in the database (re-running the query with `ORDER BY`/`WHERE`).
- Copying a selection with its column headers; exporting all rows, not only loaded ones.
- A visual check of the editor gutter and grid under the new theme (UI tests can't see paint).

## M5 — Personalisation

Picked after M4, in this order, one PR each: `keymap.toml`, autocompletion from the cached
catalog, editing data in the grid.

**Keymap demo note:** built. **View → Keymap…** lists every action with its id and shortcut;
**Create keymap.toml** writes a file with every default commented out, and **Reload** applies
edits. A typo in the file is listed as a problem (and counted in the status bar) instead of
discarding the rest. Covered by store and engine tests for reading, creating and reloading the
file, parser and conflict tests, and a UI test that runs a statement with a rebound key.

The original candidates:

- `keymap.toml`, themes, fonts.
- Custom catalog actions (`actions.toml`) and snippets.
- Autocompletion from the cached catalog.
- SSH tunnels (`russh`).
- Editing data in the grid.
- MySQL driver; a driver for the `db-data-structures` engine.
