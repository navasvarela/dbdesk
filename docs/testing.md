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

- `check_describe_table` (M5): an unquoted name resolves ignoring case, columns come in table
  order and the primary key in key order, a keyless table has an empty key, and views and
  missing tables are `None`.

Races hide in cancellation, so after changing it, run the cancel tests in a loop. A one-off
pass proved nothing in M3: the console's cancel test hung 2 runs in 5 before its fix.

Each driver crate runs the suite against its own backend:

- **SQLite**: a temporary database file, always runs.
- **Postgres**: the container from `docker-compose.yml` (`docker compose up -d --wait`, port
  55432). Tests are skipped with a message when `DBDESK_TEST_POSTGRES_URL` is unset, so
  `cargo test` works offline:
  `DBDESK_TEST_POSTGRES_URL=postgres://dbdesk:dbdesk@localhost:55432/dbdesk cargo test`.

Cancellation tests use `pg_sleep(10)` and a recursive CTE for SQLite, and assert the run ends in
well under the sleep time.

## Engine

The engine is tested headless (`dbdesk-engine/tests/engine.rs` and `console.rs`, sharing
`tests/common`): send `Command`s, collect `Event`s, assert on the sequence. It uses a temp
directory for `data_sources.toml`, console files, `history.sqlite` and exports, and an
in-memory `SecretStore`. Tests that need PostgreSQL (a commit of a failed transaction, say)
skip without `DBDESK_TEST_POSTGRES_URL`, like the driver tests. Planned: a fake driver (in-memory, scripted responses) covers ordering cases that are hard to trigger
against a real server, such as a cancel arriving after the last row.

## UI

- `AppState` reducers (applying an `Event` to state) are pure functions with plain unit tests.
- Widgets get smoke tests through `egui_kittest`, which drives egui headlessly. The app tests
  share their harness (a real engine, a temporary data source file, in-memory passwords) through
  `test_support`, and run at the app's default window size.
- `theme::tests::text_colours_meet_wcag_aa` checks every text colour against the backgrounds it
  is drawn on, in both themes, for WCAG AA contrast (4.5:1).

### Screenshots

`screenshots::regenerate_screenshots` renders the app with egui_kittest's `wgpu` renderer and
writes PNGs, every scene in the light and the dark theme, at 2x:

```bash
DBDESK_SCREENSHOTS_DIR=/tmp/shots cargo test -p dbdesk-app regenerate_screenshots -- --ignored
```

It is ignored by default because it needs a GPU adapter and rewrites files. Each scene starts a
fresh app against a SQLite fixture (a small shop database built from SQL in the test) and drives
it the way a user would: expanding nodes, opening a console, typing and running SQL. With
`DBDESK_TEST_POSTGRES_URL` set, a PostgreSQL data source joins the fixture. Without
`DBDESK_SCREENSHOTS_DIR` the images go to `target/screenshots`. Engine logs reach the log
console through one global `tracing` subscriber whose layer is swapped per scene.

Hard-learned from the M4 UI tests: `egui_kittest` clicks at the centre of a widget's last
known rectangle, in an 800×600 window. A widget laid out off-screen, or covered by another one
that egui hit-tests first, simply doesn't get the click, and the test only notices when the
expected effect never happens. When a click seems ignored, print the widget's rect and egui's
`interaction_snapshot` before suspecting the code under test. Modals are measured on their
first frame and placed on the next, so run a frame before clicking inside one.

## CI

GitHub Actions: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
on macOS and Linux. On Linux the workflow starts the compose Postgres and sets
`DBDESK_TEST_POSTGRES_URL`. GitHub can't run containers on macOS runners, so the Postgres tests
skip there.
