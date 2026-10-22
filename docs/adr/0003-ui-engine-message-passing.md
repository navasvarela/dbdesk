# 0003. UI and engine communicate only by messages

- Status: Accepted
- Date: 2026-10-22

## Context

egui runs its update loop on the main thread and expects each frame to finish in a few
milliseconds. Database calls take anywhere from a millisecond to hours. The UI must stay
responsive and every operation must be cancellable.

## Decision

- The engine runs on its own multi-threaded `tokio` runtime on a background thread.
- The UI sends `Command`s over an unbounded `tokio::sync::mpsc` channel (its sender works from
  synchronous code).
- The engine sends `Event`s back over a channel the UI drains with `try_recv` at the start of
  each frame, then calls `request_repaint()` through a small `Repaint` trait so the engine crate
  doesn't depend on egui.
- The UI owns `AppState` exclusively. Applying an event is a pure function
  `fn apply(state: &mut AppState, event: Event)`.

## Consequences

- No locks shared between UI and engine for application state; the only shared structure is the
  log ring buffer, locked briefly.
- The UI can be tested with a fake engine, and the engine headlessly by sending commands.
- Every engine response must be an event, including errors; nothing is returned "directly".
  Correlating requests uses ids (`RunId`, `CatalogPath`), and late events for cancelled runs are
  ignored by id.
- The event channel is unbounded. Row batches are bounded by paging (the engine never reads more
  than one page ahead without a `FetchMore`), so memory stays bounded in practice.

## Alternatives considered

- **`Arc<Mutex<AppState>>` shared with async tasks**: simpler at first, but leads to lock
  contention with the render loop and makes it easy to hold a lock across `.await`.
- **`poll-promise` per request** (common in egui apps): fine for one-off requests, awkward for
  streams of row batches and server notices that arrive without a request.
- **Run tokio on the main thread and egui inside it**: eframe owns the main thread on macOS, so
  this fights the framework.
