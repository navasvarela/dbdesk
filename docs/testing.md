# Testing

## Driver conformance suite

The driver traits are only honest if every driver passes the same tests. `dbdesk-core` exposes
them as `dbdesk_core::conformance` behind the `conformance` feature, which driver crates enable
in their dev-dependencies.

Implemented:
- `check_lifecycle`: server info, two pings, clean close.
- `check_connect_failure`: a bad config is a `DriverError::Connect`.
- `walk_catalog` (M2): walks the tree breadth-first (system nodes skipped) and checks that every
  child's path extends its parent's, labels aren't empty, and every node with
  `has_definition` returns a non-empty definition. It returns the visited nodes so each driver
  test can assert on its own fixture: a schema with one object of every kind.
- `check_invalid_path` (M2): nonsense paths are `DriverError::InvalidPath`, without touching the
  server. (This caught the Postgres driver trying to connect to a "database" named in a bogus path.)
- `check_execution` (M3): DDL, a 1200-row insert with its count, paging through it in pages
  of at most 500, `NULL` vs empty text, abandoning a cursor halfway, and a syntax error's byte
  position. Uses SQL both engines accept and a temporary table.
- `check_cancel` (M3): a statement that would run far longer than 10 s (`pg_sleep(60)`, an
  endless recursive CTE) stops within 10 s of a cancel from another task, and the connection
  stays usable.

- `check_transactions` (M4): `transaction_status` follows `BEGIN`, `COMMIT` and `ROLLBACK`,
  committed rows stay and rolled-back ones don't, and a failure inside a transaction leaves it
  open (failed, on PostgreSQL) until it is rolled back.

- `check_validate` (SQL tools): preparing runs nothing (an accepted `INSERT` inserts no row),
  temporary tables are visible, unknown names and syntax errors are told apart, syntax errors
  carry their position, and no transaction is left open.

- `check_describe_table` (M5): an unquoted name resolves ignoring case, columns come in table
  order and the primary key in key order, a keyless table has an empty key, and views and
  missing tables are `None`.

Races hide in cancellation, so after changing it, run the cancel tests in a loop. A one-off
pass proved nothing in M3: the console's cancel test hung 2 runs in 5 before its fix.

Each driver crate runs the suite against its own backend:

- **SQLite**: a temporary database file, always runs.
- **Postgres**: the containers from `docker-compose.yml`. `postgres` (port 55432) is a plain
  server; `postgres-tls` (port 55443) refuses unencrypted connections, and its certificate is
  signed by the test authority in `docker/postgres-tls/` (`generate.sh` makes both), which the
  `verify-full` tests trust. Tests are skipped with a message when `DBDESK_TEST_POSTGRES_URL`
  or `DBDESK_TEST_POSTGRES_TLS_URL` is unset, so `cargo test` works offline.
- **MySQL and MariaDB**: the `mysql` (8.4, port 55306) and `mariadb` (11.4, port 55307)
  containers, both through the same tests, each skipped when its `DBDESK_TEST_MYSQL_URL` or
  `DBDESK_TEST_MARIADB_URL` is unset. Both make a self-signed certificate on first start, so
  the TLS tests need no fixture of their own. `check_describe_table` asks the server's
  `lower_case_table_names` whether unquoted names match ignoring case: on Linux they don't.
- **DuckDB**: in memory or a temporary file, always runs, like SQLite: DuckDB is inside the
  driver, compiled or, in CI, linked from DuckDB's prebuilt library (see
  [duckdb-prebuilt.md](duckdb-prebuilt.md)); a test checks either way that it is DuckDB 1.5.6
  with Parquet and JSON. It runs the suite with `Spelling::STANDARD`, and reading a Parquet file from an
  in-memory database is a test of its own.
- **ClickHouse**: the `clickhouse` container (25.8, HTTP on port 58123 and HTTPS on 58443,
  with the `postgres-tls` certificate), skipped when `DBDESK_TEST_CLICKHOUSE_URL` or
  `DBDESK_TEST_CLICKHOUSE_TLS_URL` is unset. It runs `check_execution`, `check_validate` and
  `check_cancel` with `Spelling::STANDARD`, but not `check_transactions` (ClickHouse has none
  to speak of) or `check_describe_table` (no key tells a table's rows apart, so results are
  never editable).
- **SQL Server**: the `sqlserver` container (2022, port 51433), skipped when
  `DBDESK_TEST_SQLSERVER_URL` is unset. Its image is x86-64 only: native on CI's Linux runner,
  emulated (slowly, and it can crash) by Docker on an Apple-silicon Mac. The image can't
  create a database on first start, so the compose health check creates `dbdesk`. The suite
  runs with `Spelling::SQL_SERVER`: temporary tables are `#name`, and a transaction opens with
  `BEGIN TRANSACTION`, since `BEGIN` alone opens a block there. The server makes its own
  certificate, so the TLS tests need no fixture.

The SSH tunnel (`dbdesk-ssh`) is tested against the `ssh` service (port 55222): OpenSSH on
Alpine, built from `docker/ssh/` with committed test keys (`generate.sh` makes them), which
signs in `dbdesk` with a key or its password and forwards only to the `postgres` service. Its
tests skip when `DBDESK_TEST_SSH_URL` or `DBDESK_TEST_SSH_POSTGRES_URL` is unset. They fail
logins and abandon handshakes on purpose, so the server turns off OpenSSH's penalties for an
address that does that (`PerSourcePenalties`), which would otherwise refuse the next test's
connection.

Each network driver also connects through `conformance::forwarder`, a local port standing in
for a tunnel's end, with `Route::Tunnel`. The PostgreSQL and ClickHouse tests also check that
TLS is still verified against the server's own name: their certificate names `localhost`, so
the same server under another name is refused. The engine's `tests/ssh.rs` connects through the
`ssh` service itself, signing in with the test key or a prompted password.

Every server engine follows the same pattern: a service in `docker-compose.yml` and its URL in
`test-servers.env`, which CI reads too. To run everything locally:

```bash
docker compose up -d --wait
set -a; . ./test-servers.env; set +a
cargo test --workspace
```

Cancellation tests use `pg_sleep(10)` and a recursive CTE for SQLite, and assert the run ends in
well under the sleep time.

## Engine

The engine is tested headless (`dbdesk-engine/tests/engine.rs` and `console.rs`, sharing
`tests/common`): send `Command`s, collect `Event`s, assert on the sequence. It uses a temp
directory for `data_sources.toml`, console files, `history.sqlite` and exports, and an
in-memory `SecretStore`. Tests that need PostgreSQL (a commit of a failed transaction, say)
skip without `DBDESK_TEST_POSTGRES_URL`, like the driver tests. Planned: a fake driver (in-memory, scripted responses) covers ordering cases that are hard to trigger
against a real server, such as a cancel arriving after the last row.

## MCP server

- `dbdesk_sql::classify` has a corpus per dialect: reads, writes hidden in CTEs, `SELECT …
  INTO`, locking reads, DDL, transaction control and administration, and statements the parser
  can't read, which must never come out as reads.
- `dbdesk-mcp/tests/stdio.rs` starts the server as another process (the test binary again, so
  it has no libtest harness: libtest would print onto the protocol's standard output) and
  drives it with rmcp's client over stdin and stdout. It runs against SQLite always, and
  against PostgreSQL, MySQL and MariaDB when their `DBDESK_TEST_*_URL` is set. Each engine gets
  a table and a function that writes, both with a random suffix so runs sharing the servers
  don't clash. The MySQL one needs `log_bin_trust_function_creators`, which
  `docker-compose.yml` sets.
- `dbdesk-app/tests/mcp_subcommand.rs` runs the real `dbdesk mcp` binary with `HOME` pointed at
  a temporary folder: argument errors, nothing on stdout but the protocol, and one query.
  Unix only, since Windows ignores `HOME`.
- `dbdesk-store`'s history test writes from two processes at once, the second being the test
  binary again.

## UI

- `AppState` reducers (applying an `Event` to state) are pure functions with plain unit tests.
- Widgets get smoke tests through `egui_kittest`, which drives egui headlessly. The app tests
  share their harness (a real engine, a temporary data source file, in-memory passwords) through
  `test_support`, and run at the app's default window size.
- `theme::tests::text_colours_meet_wcag_aa` checks every text colour against the backgrounds it
  is drawn on, in both themes, for WCAG AA contrast (4.5:1).

Hard-learned from the M4 UI tests: `egui_kittest` clicks at the centre of a widget's last
known rectangle. The app tests run in a window of the app's default size (`DEFAULT_WINDOW_SIZE`,
1440×900), because egui_kittest's own 800×600 is too narrow for the console with the bundled
fonts. A widget laid
out off-screen, or covered by another one that egui hit-tests first, simply doesn't get the
click, and the test only notices when the expected effect never happens. When a click seems
ignored, print the widget's rect and egui's `interaction_snapshot` before suspecting the code
under test. Modals are measured on their first frame and placed on the next, so run a frame
before clicking inside one.

### Screenshots

`screenshots::regenerate_screenshots` renders the app with egui_kittest's `wgpu` renderer and
writes every picture in the user manual, at 2x, into `book/src/images`:

```bash
cargo test -p dbdesk-app regenerate_screenshots -- --ignored
```

`DBDESK_SCREENSHOTS_DIR` writes them somewhere else, and `DBDESK_SCREENSHOTS_THEMES=light,dark`
renders each scene in both themes (with a `-light` / `-dark` suffix), which is what design
reviews want. Regenerate the manual's images after a visible UI change, and run it with
`DBDESK_TEST_POSTGRES_URL` set so the workspace picture includes the PostgreSQL data source.

It is ignored by default because it needs a GPU adapter and rewrites files. Each scene starts a
fresh app against a SQLite fixture (a small shop database built from SQL in the test) and drives
it the way a user would: expanding nodes, opening a console, typing and running SQL. With
`DBDESK_TEST_POSTGRES_URL` set, a PostgreSQL data source joins the fixture. Engine logs reach the log
console through one global `tracing` subscriber whose layer is swapped per scene.

A scene that needs a test server names its variables (`needs`) and is skipped without them,
leaving its image as it was: `host-key-prompt` connects through the `ssh` service, and
`console-reconnect` ends its own PostgreSQL session. Every scene's log console shows the time,
so regenerating changes every image a little; commit only the images whose scene changed.

## User manual

The manual in `book/` is an mdBook. `mdbook build book` builds it into `book/book/html` and,
through the `mdbook-linkcheck2` backend, fails on a link to a page or image that doesn't exist.
Install both with `cargo install mdbook --version 0.5.4` and
`cargo install mdbook-linkcheck2 --version 0.13.0`.

## Website

`site/build.sh` assembles the website (`site/` plus the manual) into `target/site`; preview it
with `python3 -m http.server -d target/site`. The Pages workflow checks the landing page's links
with `lychee --offline`, which fails on a link to a file the build didn't produce.

## CI

GitHub Actions: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
on macOS and Linux, and the mdBook build with its link check. A separate Pages workflow builds
the website and manual on pull requests that touch them, and publishes them from `main`. On Linux the workflow starts every compose service and exports
the URLs in `test-servers.env`; adding an engine's container changes those two files and not
the workflow. GitHub can't run containers on macOS or Windows runners, so the server tests skip
there.
