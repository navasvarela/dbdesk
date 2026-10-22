# Testing

## Driver conformance suite

The driver traits are only honest if every driver passes the same tests. `dbdesk-core` exposes a
test-support module with a suite of async test functions, each taking a `&dyn Driver` plus a
fixture: connect, execute DDL/DML, stream more rows than one page, cancel a long query,
introspect each node kind, map each value type, report error positions.

Each driver crate runs the suite against its own backend:

- **SQLite**: in-memory database, always runs.
- **Postgres**: a container from `docker-compose.yml` (or `testcontainers`). Tests are skipped
  with a clear message when `DBDESK_TEST_POSTGRES_URL` is unset, so `cargo test` works offline.

Cancellation tests use `pg_sleep(10)` and a recursive CTE for SQLite, and assert the run ends in
well under the sleep time.

## Engine

The engine is tested headless: send `Command`s, collect `Event`s, assert on the sequence.
A fake driver (in-memory, scripted responses) covers ordering cases that are hard to trigger
against a real server, such as a cancel arriving after the last row.

## UI

- `AppState` reducers (applying an `Event` to state) are pure functions with plain unit tests.
- Widgets get smoke tests through `egui_kittest`, which drives egui headlessly and supports
  snapshot images. Used sparingly, for the result grid and explorer tree.

## CI

GitHub Actions: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
with a Postgres service container, on macOS and Linux.
