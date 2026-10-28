# Log console

The bottom panel. A single, filterable stream of everything the app did and everything the
servers said.

## What gets logged

| Category | Examples | Level |
| --- | --- | --- |
| Connection | connecting to `host:port/db`, connected in 42 ms, server version, disconnected, reconnect | info |
| Query | statement text (truncated), duration, rows fetched / affected, cancelled | info |
| Server notice | Postgres `NOTICE`/`WARNING` from `RAISE`, `VACUUM VERBOSE`, etc. | info / warn |
| Introspection | loaded `public.tables` (12 nodes) in 8 ms | debug |
| Error | SQLSTATE, message, detail, hint, position | error |
| App | config loaded, keychain access denied, layout restored | info / warn |

Every entry carries a timestamp, level, category, and optionally the data source and session it
belongs to.

## Pipeline

All logging goes through `tracing`. Engine code emits ordinary `tracing` events with structured
fields (`data_source`, `session`, `run`, `category`). A custom `tracing_subscriber::Layer`:

1. Converts each event into a `LogEntry` (fields extracted with a `Visit` implementation).
2. Sends it over a channel to the UI and, if the event came from another thread, requests a
   repaint.
3. The UI drains the channel every frame into a ring buffer of 10 000 entries (oldest dropped
   first). Nothing is shared behind a lock.

The UI-thread exception in step 2 avoids a deadlock: `request_repaint` takes egui's context
lock, which is not reentrant, and an event logged while that lock is held would block forever.
Entries logged on the UI thread are collected by a second drain at the end of each frame, which
requests the repaint itself.

Records from the `log` crate (used by eframe and winit) are bridged into `tracing`. A target
filter keeps DbRover's own crates at debug and everything else at warn and above.

A second layer writes the same events to a rolling file (`tracing-appender`) for debugging
crashes after the fact.

Server notices are a deliberate design point for Postgres: `tokio-postgres` only surfaces
`NOTICE` messages if the `Connection` future is polled with `poll_message` instead of being
spawned and forgotten. The Postgres driver therefore runs its own connection task that forwards
`AsyncMessage::Notice` into the session's notice queue.

## UI

- Columns: time, level icon, data source (coloured chip), message.
- Filters: level, data source, category, free-text search.
- Auto-scroll that pauses while the user is scrolled up, with a "jump to latest" button.
- Clicking a query entry focuses the console tab that ran it; clicking an error with a position
  jumps to it in the editor.
- Clear, copy selection, save to file.

## Open questions

- Should each data source get its own log tab (some tools, such as DBeaver and DataGrip, show
  output per session) in addition to the global stream with a filter? (Leaning: global stream only; filters are enough.)
