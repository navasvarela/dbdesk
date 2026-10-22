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
- Widgets get smoke tests through `egui_kittest`, which drives egui headlessly and supports
  snapshot images. Used sparingly, for the result grid and explorer tree.

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
