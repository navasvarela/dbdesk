<img src="assets/brand/logo.svg" alt="The dbdesk logo: a database cylinder in teal and violet" width="96">

# dbdesk

A desktop database tool in the spirit of JetBrains DataGrip, written in Rust.
It is a learning project first and a personal daily driver second: the goal is a tool
small enough to understand end to end and easy to bend to my own workflow.

Status: **M4 (daily-driver polish)**. You can add PostgreSQL and SQLite data sources and browse
their catalogs. Query consoles have highlighting and line numbers, run the statement at the
cursor or a whole script, explain it, page through results, cancel long queries, and point at
errors. Consoles are saved as `.sql` files and reopen on start, every statement goes to a
searchable history, and a console can hold a transaction open until you commit. The grid
selects, copies, sorts and filters loaded rows, shows long values, and exports CSV or JSON.

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

The PostgreSQL tests need a server and are skipped without one. To run them, start the bundled
container and point the tests at it:

```bash
docker compose up -d --wait
```

```bash
DBDESK_TEST_POSTGRES_URL=postgres://dbdesk:dbdesk@localhost:55432/dbdesk cargo test --workspace
```

Files live in `~/Library/Application Support/dbdesk` on macOS: data sources in
`data_sources.toml`, consoles in `consoles/`, the query history in `history.sqlite`, window
and layout state in `app.ron`, daily log files in `logs/`. Passwords
are in the macOS Keychain under the service `dbdesk`.

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
| [Keymap](docs/features/keymap.md) | `keymap.toml`: action ids and rebinding keys |
| [Persistence](docs/persistence.md) | Config files, secrets, history, layout state |
| [Testing](docs/testing.md) | Driver conformance suite, UI testing strategy |
| [Brand](docs/brand.md) | The logo, its files, and the colour palette |
| [Look and feel](docs/features/look-and-feel.md) | Themes, panels, fonts, icons, data source colours |
| [Roadmap](docs/roadmap.md) | Milestones M0–M5 with exit criteria |
| [ADRs](docs/adr/README.md) | Architecture decision records |
