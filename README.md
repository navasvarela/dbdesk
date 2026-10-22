# dbdesk

A desktop database tool in the spirit of JetBrains DataGrip, written in Rust.
It is a learning project first and a personal daily driver second: the goal is a tool
small enough to understand end to end and easy to bend to my own workflow.

Status: **M0 (foundations)**. The app opens with the explorer, query console area and log
console docked, and the engine runs in the background. There are no data sources yet.

## Building and running

```bash
cargo run -p dbdesk-app
```

Checks run in CI (and should pass locally before pushing):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Files live in `~/Library/Application Support/dbdesk` on macOS: window and layout state in
`app.ron`, daily log files in `logs/`.

## Core features (v1)

- **Connection explorer**: a tree of data sources → databases → schemas → objects, loaded lazily.
- **SQL runner**: a query console per data source with syntax highlighting, run statement /
  selection / script, a streaming result grid, and cancellation.
- **Log console**: one place for connection lifecycle events, query timings, server notices and errors.

First engines: **PostgreSQL** and **SQLite**.
UI: **egui** (via `eframe`) with **egui_dock** for DataGrip-style dockable panels.

## Documents

| Document | What it covers |
| --- | --- |
| [Vision](docs/vision.md) | Goals, non-goals, terminology, what "personalised" means |
| [Architecture](docs/architecture.md) | Crates, threading model, data flow, core types |
| [Connection explorer](docs/features/connection-explorer.md) | Data sources, catalog tree, introspection |
| [SQL runner](docs/features/sql-runner.md) | Editor, execution, result grid, cancellation |
| [Log console](docs/features/log-console.md) | Logging pipeline and what gets logged |
| [Persistence](docs/persistence.md) | Config files, secrets, history, layout state |
| [Testing](docs/testing.md) | Driver conformance suite, UI testing strategy |
| [Roadmap](docs/roadmap.md) | Milestones M0–M5 with exit criteria |
| [ADRs](docs/adr/README.md) | Architecture decision records |
