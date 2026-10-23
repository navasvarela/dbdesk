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
- ~~A file picker for SQLite paths.~~ Done: **Browse…** and **New…** (create an empty database).
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

**Completion demo note:** built. Typing in a console suggests columns of the tables the
statement names, tables, schemas and keywords from the explorer's cached catalog, loading
what's missing in the background; ⌃Space asks anywhere, `alias.` lists that table's columns.
Covered by tests for finding the prefix, qualifier and table references, ranking candidates,
reading and requesting catalog levels, accepting a candidate, and a UI test that completes a
column on a fresh connection.

**Grid editing demo note:** built. Open a table's data, double-click a cell (or F2), type, and
**Submit** writes one `UPDATE` per changed row, all or none; in manual commit mode they join
the open transaction. Results that aren't one table's rows, or lack its primary key, say
"Read-only" and why. Covered by a `describe_table` conformance check on both drivers, engine
tests for all-or-none edits in a transaction or a savepoint (SQLite and PostgreSQL), tests for
deciding what's editable and generating `UPDATE`s, and a UI test that edits and submits a
cell.

The original candidates:

- `keymap.toml`, themes, fonts.
- Custom catalog actions (`actions.toml`) and snippets.
- Autocompletion from the cached catalog.
- SSH tunnels (`russh`).
- Editing data in the grid.
- MySQL driver; a driver for the `db-data-structures` engine.

## SQL tools

Three additions to the query console, one PR each: a formatter, a syntax checker and linter,
and context-aware completion.

**Formatter demo note:** built. Type `select id,name from users where active=true` and press
⌥⌘L (or **Format**, or **Code → Format**): the statement is laid out clause by clause with
upper-case keywords and the cursor stays on its token; ⌘Z takes it back in one step. ⇧⌥⌘L
formats the whole console. **View → Settings…** switches keywords to lower case or as typed,
and the indent width, saved in `settings.toml`. The formatter is our own, over the lexer,
because `sqlformat` changed string literals and dollar-quoted bodies (see the SQL runner doc).
Covered by a corpus of PostgreSQL and SQLite statements formatted in every setting, whose
tokens must come out identical (ignoring whitespace and keyword case) and stable when formatted
twice; layout, cursor-mapping and safety-check tests; settings file and engine tests; and a UI
test that formats with ⌥⌘L, undoes it, and formats again from the Code menu after switching to
lower case.

**Linter demo note:** built. Type `select * from orders where id = null;` and, once typing
pauses, `= null` gets an amber wave; hover for "= NULL is never true; use IS NULL". An
unfinished `select id from orders where` gets a red one, and the toolbar says
**1 error, 1 warning**; F8 jumps between them. Run something so the console connects, and its
statements are also prepared (never run) on its own connection: a misspelt column then shows
the database's own message. Rules are switched off in View → Settings…. Syntax errors come
from `sqlparser`, only for data queries and changes, with a list of constructs it wrongly
rejects found by a corpus of valid statements. Covered by that corpus and lint tests in
`dbdesk-sql`, a `validate` conformance check on both drivers, engine tests that validation
never connects, never disturbs unread rows and skips open PostgreSQL transactions, problem
and catalog-lint tests, and a UI test that hovers a warning and an error.

**Completion demo note:** built, extending M5's completion. `SELECT * FROM cus` offers the
`customers` table, not columns; `JOIN customers c ON c.` lists its columns with their types;
`INSERT INTO orders (cu` offers only `orders`' columns; `WITH big AS (SELECT id AS big_id
FROM orders) SELECT * FROM big WHERE big_` offers `big_id`, read from the CTE by the parser
(the lexer when it doesn't parse); `coal` gives `coalesce()` with the cursor inside. `cid`
fuzzy-matches `customer_id`, and what you accepted lately rises to the top in every console.
The list has the explorer's icons and each candidate's type. Covered by context, derived
table, ranking, function and projection tests, and a UI test that completes a table after
`FROM` (where the column of the same prefix isn't offered) and an alias's column after its
dot. Fixed on the way: the list never grew past the height of its first frame.

## Release track

Downloads that install like any other desktop app, instead of building from source. The full
plan, including the decisions it needs first, is in [Releasing](releasing.md).

- **R1 — First downloadable build:** unsigned macOS (universal `.dmg`) and Linux (AppImage,
  `.deb`) builds attached to a draft GitHub release by a tag-triggered workflow; Windows in CI,
  its installer attached once tried on a real machine. Also `--version`, the MIT OR Apache-2.0
  licence files, and the Install chapter rewritten.
- **R2 — Trusted installs:** macOS signing and notarization, Windows signing, a Homebrew cask,
  generated release notes.
- **R3 — Updates:** an in-app update notice, then automatic updates.

## Database track

More engines, in order of how many people use them. The full plan, with the rankings, what
each engine needs and the open decisions, is in [More databases](databases.md).

- **DB0 — Groundwork:**
  - dialect facts gathered in one module;
  - TLS for network connections, PostgreSQL first;
  - one data source form shared by every server engine;
  - one CI container per engine.
- **DB1 — MySQL and MariaDB:** one driver on `mysql_async`, with cancel by `KILL QUERY` and
  `SHOW CREATE` definitions.
- **DB2 — SQL Server:** `tiberius`, SQL Server logins first, `GO` batches, XML plans.
- **DB3 — DuckDB:** files and in-memory data sources for querying CSV and Parquet, if the
  bundled engine keeps the downloads small enough.
- **DB4 — Oracle:** after R2, because its client needs Oracle's own library installed
  separately.

## Design and documentation track

Runs alongside the feature milestones rather than after them: each item is usable on its own.
They build on each other in this order, because each one reuses what the one before it made:

```mermaid
flowchart LR
    D1[D1 Logo and palette] --> D2[D2 Vibrant UI]
    D1 --> D4[D4 Website]
    D2 --> D3[D3 User manual]
    D3 --> D4
```

### D1 — Logo and brand palette

- A logo mark that works from 16 px (tab icon, favicon) up to a hero image: a simple, bold
  shape, readable in one colour, drawn as SVG.
- A small palette derived from it: one primary accent, one secondary accent, and the success,
  warning and error colours, each with light- and dark-theme variants. D2 builds the UI theme
  from these values, so the logo and the app clearly belong together.
- Exports: `assets/brand/logo.svg` (full colour), `logo-mono.svg` (one colour), a PNG set for
  the window icon (eframe's `ViewportBuilder::with_icon`) and the macOS `.icns`.
- Process: three or four concepts side by side, pick one, refine it.

**Exit:** the dbdesk window, the README and the repository's social preview show the logo.

**Demo note:** built in #7. The mark is "Strata", a database cylinder cut into a lid and two
slabs, picked from four concepts; [brand.md](brand.md) has its construction, the files and the
palette with its contrast ratios. `assets/brand/export.sh` regenerates every PNG and the
`.icns` reproducibly. The window and the README show the logo. The social preview is a
repository setting rather than a file: `assets/brand/social-preview.png` is uploaded under
**Settings → General → Social preview**, and has to be uploaded again whenever it changes.

### D2 — A more colourful, vibrant UI

egui's default look is deliberately neutral grey. This replaces it with a theme of dbdesk's own:

- **Theme module.** `Visuals` built from the D1 palette for light and dark mode: accent-tinted
  selection, hover and active states, panel and tab-bar colours, and an `egui_dock` style to
  match. The theme follows the system setting, with an override in the View menu.
- **Colour that carries meaning:**
  - A colour per data source, shown on its explorer row, its console tabs and a band along the
    top of the console (the colour planned in M1's follow-ups; it lands here).
  - Coloured icons per catalog node kind.
  - Status dots and run status in the palette's success, warning and error colours.
- **Icons.** An icon font (Phosphor, via `egui-phosphor`) for toolbar buttons, tree nodes and
  menus, instead of text-only buttons.
- **Typography.** Inter for the UI and JetBrains Mono for SQL and results, bundled (both
  under the SIL Open Font License).
- **Syntax and grid colours** retuned to the palette; a zebra-striped grid with a tinted
  header.
- **Contrast:** text stays at WCAG AA contrast against its background in both themes, checked
  by a unit test over the palette.

**Exit:** before/after screenshots of each panel in both themes are in the PR, and the app
still reads well in both.

**Demo note:** built in #10, with before/after screenshots of each panel in both themes.
Decisions, including why icons have a font family of their own, are in
[look-and-feel.md](features/look-and-feel.md).

### D3 — User manual

- **mdBook** in `book/`, written for someone who has never seen dbdesk. Chapters:
  1. Install and first launch
  2. Data sources: adding, testing, passwords and the keychain
  3. Explorer: browsing, filtering, definitions
  4. Query consoles: running, results, cancelling, errors
  5. Keyboard shortcuts
  6. Where dbdesk keeps its files
  7. Troubleshooting (TLS, keychain prompts, logs)
- **Screenshots generated, not hand-taken.** egui_kittest's snapshot feature renders the
  app headlessly (wgpu), against a fixture database. Running one test regenerates every image,
  so they never go stale.
- **Kept in step with the code.** Each milestone's PR updates the chapters it affects, and CI
  builds the book (`mdbook build`, plus a link check) so a broken page fails the PR.

**Exit:** the manual covers everything up to the current milestone, and builds in CI.

**Demo note:** built in #11, covering everything up to M5: data sources and the keychain,
the explorer, consoles (saved consoles, transactions, history, completion, sorting, filtering,
export and grid editing), the keymap, files and troubleshooting. All eleven images are rendered
by one ignored test (`screenshots::regenerate_screenshots`), and CI builds the book with
`mdbook-linkcheck2`.

### D4 — Website on GitHub Pages

- A landing page with the logo, a one-paragraph pitch, screenshots from D3, and links to the
  manual, the source and (later) downloads. Plain HTML/CSS in `site/`, styled with the D1
  palette. No framework needed for one page.
- The manual (D3) is published under `/manual/` on the same site.
- **Deployment:** a GitHub Actions workflow builds the landing page and the book on every push
  to `main` and publishes them with `actions/deploy-pages`. PRs only build, so a broken site
  fails the PR without going live.
- **Hosting:** GitHub Pages from this repository, which is public, so the free plan covers it.
  (While it was private, Pages would have needed a paid plan or a separate public repository.)

**Exit:** the site is live, linked from the README, and redeploys on every merge to `main`.

**Demo note:** built. <https://navasvarela.github.io/dbdesk/> is one hand-written page in
`site/` (HTML and CSS, the app's palette and bundled fonts, dark first with a light theme), with
the manual under `/manual/`. `site/build.sh` assembles both, copying the logo, fonts and database
logos from `assets/` rather than keeping second copies. `.github/workflows/pages.yml` builds it
on pull requests that touch it (checking the landing page's links with lychee, offline, and the
manual's with mdbook-linkcheck2) and publishes it on every push to `main`. The Welcome tab's
**Open the manual** now opens the published manual. The Download button goes to the releases
page, which is empty until the first release is published.
