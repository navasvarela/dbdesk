# 0002. Native drivers behind a trait, not sqlx

- Status: Accepted
- Date: 2026-10-22

## Context

A database *tool* needs things an application rarely does: row streaming with early stop,
out-of-band cancellation, server notices, full error detail (SQLSTATE, position, hint), and
graceful handling of column types it has never seen. It also needs to support several engines
through one interface.

## Decision

Each engine gets its own crate implementing the `Driver` and `Connection` traits from
`dbdesk-core`, using the engine's native Rust client:

- Postgres: `tokio-postgres`
- SQLite: `rusqlite` (bundled SQLite), run on `spawn_blocking`

The traits are used as trait objects (`Box<dyn Connection>`), with `async-trait` for the async
methods, because native `async fn` in traits is not yet dyn-compatible.

## Consequences

- Full access to engine features: `cancel_token`, `AsyncMessage::Notice`, `query_raw` streaming,
  `InterruptHandle`, type OIDs.
- Unknown types map to `Value::Other` with the server's text form instead of failing.
- Adding an engine means a new crate plus one registration line in the engine's driver registry,
  with no changes to existing drivers or the UI (open/closed).
- Each driver has to implement type mapping and introspection itself. The shared conformance
  suite ([Testing](../testing.md)) keeps them consistent.
- `async-trait` boxes each call's future. That cost is negligible next to a network round trip.

## Alternatives considered

- **sqlx with `AnyPool`**: one API for all engines, but the `Any` driver erases type information,
  has no portable cancellation or notice support, and its compile-time checked queries add
  nothing for user-written SQL.
- **Enum dispatch** (`enum AnyConnection { Postgres(..), Sqlite(..) }`): no boxing, but every new
  engine edits a central `match` in many places, which is the scattered change this design aims
  to avoid. Can be revisited if profiling ever shows dispatch matters (it won't).
- **ODBC / JDBC bridge**: broad engine coverage, but heavy native dependencies and far less
  control. Defeats the learning goal.
