# Testing

## Driver conformance suite

The driver traits are only honest if every driver passes the same tests. `dbdesk-core` exposes
them as `dbdesk_core::conformance` behind the `conformance` feature, which driver crates enable
in their dev-dependencies.

Implemented (M1): `check_lifecycle` (server info, two pings, clean close) and
`check_connect_failure` (bad config is a `DriverError::Connect`). Planned as the trait grows:
execute DDL/DML, stream more rows than one page, cancel a long query, introspect each node kind,
map each value type, report error positions.

Each driver crate runs the suite against its own backend:

- **SQLite**: a temporary database file, always runs.
- **Postgres**: the container from `docker-compose.yml` (`docker compose up -d --wait`, port
  55432). Tests are skipped with a message when `DBDESK_TEST_POSTGRES_URL` is unset, so
  `cargo test` works offline:
  `DBDESK_TEST_POSTGRES_URL=postgres://dbdesk:dbdesk@localhost:55432/dbdesk cargo test`.

Cancellation tests use `pg_sleep(10)` and a recursive CTE for SQLite, and assert the run ends in
well under the sleep time.

## Engine

The engine is tested headless (`dbdesk-engine/tests/engine.rs`): send `Command`s, collect
`Event`s, assert on the sequence. It uses a temp directory for `data_sources.toml` and an
in-memory `SecretStore`. Planned: a fake driver (in-memory, scripted responses) covers ordering cases that are hard to trigger
against a real server, such as a cancel arriving after the last row.

## UI

- `AppState` reducers (applying an `Event` to state) are pure functions with plain unit tests.
- Widgets get smoke tests through `egui_kittest`, which drives egui headlessly and supports
  snapshot images. Used sparingly, for the result grid and explorer tree.

## CI

GitHub Actions: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
on macOS and Linux. On Linux the workflow starts the compose Postgres and sets
`DBDESK_TEST_POSTGRES_URL`. GitHub can't run containers on macOS runners, so the Postgres tests
skip there.
