# 0002. Native drivers behind a trait, not sqlx

- Status: Accepted
- Date: 2026-10-22

## Context

A database *tool* needs things an application rarely does: row streaming with early stop,
out-of-band cancellation, server notices, full error detail (SQLSTATE, position, hint), and
graceful handling of column types it has never seen. It also needs to support several engines
through one interface.

## Decision

Each engine gets its own crate implementing the `Connection` trait from `dbdesk-core`, using
the engine's native Rust client:

- Postgres: `tokio-postgres`
- SQLite: `rusqlite` (bundled SQLite), run on `spawn_blocking`

The trait is used as a trait object (`Box<dyn Connection>`), with `async-trait` for the async
methods, because native `async fn` in traits is not yet dyn-compatible.

**Amended in M1 (2026-10-22):** the original design also had a `Driver` trait and a driver
registry, so that adding an engine needed no edits outside its crate. In practice every engine
needs its own variant in the data source file format and its own form in the UI anyway. So
connection settings are a closed enum, `ConnectionConfig { Postgres(..), Sqlite(..) }`, in
`dbdesk-core`. Opening a connection is one `match` over it in the engine (`dbdesk-engine/src/open.rs`).
Each driver crate exposes a plain `connect` function that returns its concrete connection type.
Adding an engine is now a compiler-guided change: a new variant, then every non-exhaustive
`match` the compiler points at. Once connected, everything still goes through `dyn Connection`.

## Consequences

- Full access to engine features: `cancel_token`, `AsyncMessage::Notice`, `query_raw` streaming,
  `InterruptHandle`, type OIDs.
- Unknown types map to `Value::Other` with the server's text form instead of failing.
- Adding an engine means a new crate, a new `ConnectionConfig` variant, a form in the UI, and one
  arm in the engine's `connect` match (see the amendment above). Existing drivers are untouched.
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
