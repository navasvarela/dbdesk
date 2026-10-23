<img src="assets/brand/logo.svg" alt="The dbdesk logo: a database cylinder in teal and violet" width="96">

# dbdesk

**Browse and Query databases.**

dbdesk is a desktop database tool written in Rust. It connects to PostgreSQL and SQLite today,
and support for MySQL, ClickHouse, Oracle and others is planned. It is a learning project first
and a personal daily driver second: the goal is a tool small enough to understand end to end
and easy to bend to my own workflow.

Status: **M5 (personalisation)**. You can add PostgreSQL and SQLite data sources and browse
their catalogs. Query consoles have highlighting, line numbers and completion from the catalog;
they run the statement at the cursor or a whole script, explain it, page through results,
cancel long queries, and point at errors. Consoles are saved as `.sql` files and reopen on
start, every statement goes to a searchable history, and a console can hold a transaction open
until you commit. The grid selects, copies, sorts and filters loaded rows, shows long values,
exports CSV or JSON, and edits a table's rows. Console and grid shortcuts can be changed in
`keymap.toml`, and the interface comes in light and dark themes.

Website: **<https://navasvarela.github.io/dbdesk/>**, with the
[user manual](https://navasvarela.github.io/dbdesk/manual/), which explains how to install and
use dbdesk. Its sources are in `book/`; `mdbook serve book` shows it locally.

## Building and running

The everyday commands are recipes in the `justfile`, for [just](https://github.com/casey/just)
(`brew install just` or `cargo install just`); `just` lists them.

```bash
just run
```

Before pushing, run the checks CI runs. `just format` fixes what `just lint` reports about
formatting.

```bash
just lint
```

```bash
just test
```

Each recipe is one or two `cargo` commands, which also work without `just`. The `justfile`
shows them.

The PostgreSQL tests need a server and are skipped without one. To run them, start the bundled
containers (a plain PostgreSQL and one that requires TLS) and load their URLs from
`test-servers.env`:

```bash
docker compose up -d --wait
```

```bash
set -a; . ./test-servers.env; set +a; just test
```

Files live in `~/Library/Application Support/dbdesk` on macOS: data sources in
`data_sources.toml`, consoles in `consoles/`, the query history in `history.sqlite`, window and
layout state in `app.ron`, your shortcuts in `keymap.toml`, daily log files in `logs/`.
Passwords are in the macOS Keychain under the service `dbdesk`.

## Core features (v1)

- **Connection explorer**: a tree of data sources → databases → schemas → objects, loaded lazily.
- **SQL runner**: a query console per data source with syntax highlighting, run statement /
  selection / script, a streaming result grid, and cancellation.
- **Log console**: one place for connection lifecycle events, query timings, server notices and errors.

Engines: **PostgreSQL** and **SQLite**, with MySQL, ClickHouse, Oracle and others planned.
UI: **egui** (via `eframe`) with **egui_dock** for dockable panels.

## Documents

| Document | What it covers |
| --- | --- |
| [User manual](book/src/SUMMARY.md) | Installing and using dbdesk, for people who have never seen it |
| [Vision](docs/vision.md) | Goals, non-goals, terminology, what "personalised" means |
| [Architecture](docs/architecture.md) | Crates, threading model, data flow, core types |
| [Connection explorer](docs/features/connection-explorer.md) | Data sources, catalog tree, introspection |
| [SQL runner](docs/features/sql-runner.md) | Editor, execution, result grid, cancellation |
| [Log console](docs/features/log-console.md) | Logging pipeline and what gets logged |
| [Keymap](docs/features/keymap.md) | `keymap.toml`: action ids and rebinding keys |
| [Persistence](docs/persistence.md) | Config files, secrets, history, layout state |
| [Testing](docs/testing.md) | Driver conformance suite, UI testing strategy, screenshots, the manual's build |
| [Brand](docs/brand.md) | The logo, its files, and the colour palette |
| [Look and feel](docs/features/look-and-feel.md) | Themes, panels, fonts, icons, data source colours |
| [Roadmap](docs/roadmap.md) | Milestones M0–M5, and the release (R1–R3), database (DB0–DB4), object helpers (E0–E4) and design and documentation (D1–D4) tracks, with exit criteria |
| [Releasing](docs/releasing.md) | Plan for downloadable builds: packaging, signing, the release workflow (proposed) |
| [More databases](docs/databases.md) | Which engines come next (MySQL/MariaDB, SQL Server, DuckDB, Oracle later) and what each needs (proposed) |
| [Creating and changing database objects](docs/entity-helpers.md) | Dialogs for tables, views and users that show the SQL before running it (proposed) |
| [Contributing](CONTRIBUTING.md) | How commits, branches and pull requests are named |
| [ADRs](docs/adr/README.md) | Architecture decision records |

## Licence

dbdesk is licensed under either of

- the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), or
- the MIT licence ([LICENSE-MIT](LICENSE-MIT)),

at your option.

The bundled fonts keep their own licences: Inter
([`assets/fonts/inter/LICENSE.txt`](assets/fonts/inter/LICENSE.txt)) and JetBrains Mono
([`assets/fonts/jetbrains-mono/OFL.txt`](assets/fonts/jetbrains-mono/OFL.txt)), both under the
SIL Open Font License 1.1.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion
in dbdesk, as defined in the Apache-2.0 licence, is dual licensed as above, without any
additional terms or conditions.
