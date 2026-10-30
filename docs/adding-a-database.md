# Adding a database

This guide is for contributors who want DbRover to speak a new database engine. It walks through
every place an engine touches, in the order that keeps each step reviewable. The pull request
names in brackets refer to how MySQL and MariaDB support was added, which is the best worked
example to read next to this guide:
- [#32](https://github.com/navasvarela/dbrover/pull/32) made quoting and the default schema
  per-dialect facts;
- [#35](https://github.com/navasvarela/dbrover/pull/35) added the dialect;
- [#36](https://github.com/navasvarela/dbrover/pull/36) added the driver;
- [#37](https://github.com/navasvarela/dbrover/pull/37) finished the console and the manual.

[More databases](databases.md) says which engines are planned and in what order. Its
"What a new engine needs" section is the short version of this guide.
[ADR 0002](adr/0002-native-drivers-behind-a-trait.md) explains why every engine has its own
native driver behind one trait.

## Before writing code

1. **Say which engine you are adding.** Open an issue, or add or update the engine's section in
   [More databases](databases.md). Agree on the client crate and the scope there first: which
   versions, which ways to log in, and what is left out.
2. **Choose the client crate.** In order of importance, it should:
   - **ship with DbRover without anything installed separately.** Pure Rust is best. A bundled C
     library is acceptable when it builds with only a C compiler. A library the user must install
     (as Oracle's needs) is a reason to wait.
   - **use rustls for TLS, never OpenSSL.** Prefer the `ring` provider, which the workspace
     already builds (see `docs/releasing.md` and the comment on `rustls` in the workspace
     `Cargo.toml`).
   - **stream rows,** so a large result costs nothing until it is paged through.
   - **cancel a running statement,** in the protocol or from a second connection.
   - **report full errors:** a code, a message and, ideally, the position in the statement.
   - **be maintained,** with a licence compatible with MIT OR Apache-2.0.

   An async client running on tokio is simplest. A blocking one works too, run on
   `spawn_blocking` as the SQLite driver does.
3. **Find the server's container image.** A healthcheck must say when it accepts connections,
   and it must run on GitHub's Linux runners.

## The steps

Each step is one pull request, in this order. Every step compiles and passes the tests on its
own. The engine lists are closed enums on purpose (ADR 0002), so adding a variant makes the
compiler point at every place that must handle it. Follow its errors rather than searching.

### 1. The dialect (`dbrover-sql`) [#35]

Everything DbRover knows about an engine's SQL lives in `crates/dbrover-sql/src/dialect/`. Add a
variant to `Dialect`, then fill in each fact the compiler asks for:

| Fact | Where | What it decides |
| --- | --- | --- |
| `lexical_rules` | `dialect/mod.rs` | Quoting, comments and string escapes (`LexicalRules`): highlighting, splitting and the formatter |
| `has_delimiter_command` | `dialect/mod.rs` | Client-side batch separators such as MySQL's `DELIMITER` or SQL Server's `GO` |
| `sqlparser_dialect` and `parser_gaps` | `dialect/mod.rs` | Which `sqlparser` dialect the syntax checker uses, and the valid statements it wrongly rejects |
| `builtin_functions` | `dialect/functions.rs` | Function names offered by completion |
| `explain_prefix` | `dialect/mod.rs` | How ⌥⌘E asks for a plan, possibly depending on the server version |
| `quoting` | `dialect/mod.rs` | How generated SQL (Open data, grid edits) quotes identifiers |
| `is_system_name` | `dialect/mod.rs` | Which schemas and tables the explorer hides unless **System** is ticked |
| `insert_verbs`, `implicit_commit_verbs` | `dialect/mod.rs` | Safety warnings and manual commit mode: statements that write, and DDL that commits by itself |
| `From<DriverKind>` | `dialect/mod.rs` | Which dialect a data source's driver uses; filled in at step 3 |

Tests:
- add the dialect's cases to the lexer, splitter (`split.rs`) and formatter (`format/tests.rs`)
  tables;
- give the syntax checker a corpus of real, valid statements (`check/tests.rs`). Every one must
  parse, or be listed as a parser gap with the reason.

**Done when:** a script in the new dialect, comments and quoting tricks included, splits and
formats as the engine itself would read it, and its corpus passes.

### 2. The driver crate [#36]

Create `crates/dbrover-driver-<engine>/`, using `dbrover-driver-mysql` as the template:

| File | Holds |
| --- | --- |
| `lib.rs` | `pub async fn connect(config, password) -> Result<…Connection, DriverError>`, and the `Connection` implementation |
| `catalog.rs` | `children` and `definition`: the tree, one level per call |
| `execution.rs` | `execute`, returning a `ResultCursor` that reads rows lazily, plus the cancel handle |
| `table.rs` | `describe_table`, for editing in the grid |
| `errors.rs` | Mapping the client's errors to `DriverError`, keeping the code, message and position |
| `tls.rs` | Turning the shared `TlsMode` into the client's TLS setup. A client that takes a rustls `ClientConfig` gets it from `dbrover-tls`, as PostgreSQL and ClickHouse do |
| `tests.rs` | The conformance suite against a real server |

What each `Connection` method must do is documented on the trait in
`crates/dbrover-core/src/connection.rs`:
- **The catalog's shape** is up to the engine: databases, schemas, then folders of tables,
  views and so on. Use the existing `NodeKind` and `FolderKind` values where they fit, and add
  new ones in `dbrover-core/src/catalog.rs` only for objects the others don't have.
- **Values reach the grid as text** (ADR 0004). Map each column type to its text form, and an
  unknown type to the server's own text rather than an error.
- **`validate`** prepares a statement without running it, and returns `None` when the database
  would accept it. It must never run anything. If the engine has no way to prepare without
  running, say so in its plan before choosing what to do. Returning `None` would hide the
  database's own errors behind the syntax check.
- **Cancel** must stop a statement that is still running and leave the connection usable.

Tests use the conformance suite in `dbrover-core/src/conformance.rs`:
- `check_lifecycle` and `check_connect_failure`;
- `walk_catalog` and `check_invalid_path`, and `check_children_pages` if the driver pages a
  level that can grow large (`Connection::children_page`; the default loads it whole);
- `check_execution`, `check_transactions`, `check_describe_table`, `check_validate`, and
  `check_cancel` (with a statement that runs long, such as MySQL's `SLEEP`).

An engine without transactions, or without a key that tells a table's rows apart, skips
`check_transactions` or `check_describe_table` and says why in its tests, as ClickHouse does.

Each test reads its server from a `DBROVER_TEST_<ENGINE>_URL` variable and skips when it is
unset, so `cargo test` works offline (see the top of `dbrover-driver-mysql/src/tests.rs`).

### 3. Core and engine wiring [#36]

- **`dbrover-core/src/data_source.rs`:**
  - A `ConnectionConfig` variant holding an `<Engine>Config`. A server engine embeds the shared
    `ServerConfig` (host, port, database, user, TLS) and adds a `DEFAULT_PORT`. A file engine
    looks like `SqliteConfig`.
  - A `DriverKind` variant, added to `DriverKind::ALL`, with its `display_name`. Also set
    `uses_password`, which decides whether the keychain is asked.
  - The `default_name`, `database` and `default_schema` arms.
  - The serde tag (`driver = "<engine>"` in `data_sources.toml`), with a round-trip test.
- **`dbrover-engine/src/open.rs`:** one arm in `connect` calling your crate's `connect`. Add the
  crate to `dbrover-engine/Cargo.toml` and the workspace dependencies.
- **`dbrover-engine/tests/common/mod.rs`:** a helper reading the test server's URL, so engine
  and console tests can run against it.
- **`dbrover-language`:** the `From<DriverKind> for QueryLanguage` arm, `QueryLanguage::Sql` with
  the new dialect.

### 4. The app [#36, #37]

- **The data source form** (`dbrover-app/src/data_source_form/`):
  - a `DriverFields` variant;
  - a server engine reuses `ServerFields` (`server.rs`) for host, port, database, user,
    password and TLS, and adds only its own extras;
  - a file engine follows SQLite's fields, with Browse… and, if the engine can, New….
- **The logo:**
  - add the SVG to `assets/icons/databases/`, following that folder's README (source, licence,
    white fill, trademark note);
  - add its row to `database_brand.rs`;
  - the theme's contrast test checks the two shades.
- **Whatever else the compiler points at.** For MySQL that included the grid's edit statements
  (`console/edit.rs`) and the hazard warnings for DDL that commits (`hazard_dialog.rs`).
- **The MCP server's read-only mode** (`dbrover-mcp/src/read_only.rs`): how the engine refuses
  writes on its own, or `None`, which keeps its data sources from being served until it has one
  (see `docs/mcp-server.md`).
- **A UI test** that adds a data source of the new engine through the dialog. With a test server
  set, also browse it and run a query.

### 5. Test servers in CI [#36]

- **`docker-compose.yml`:** a service on an unused port in the 55xxx range, with a healthcheck
  that only passes once the server accepts real connections. Explain any workaround in a
  comment, as the MySQL service does.
- **`test-servers.env`:** the line `DBROVER_TEST_<ENGINE>_URL=…`. CI starts every compose service
  and exports every line of this file on its Linux runner, so nothing else in
  `.github/workflows/ci.yml` changes.
- **`docs/testing.md`:** the service, its version and its port.

Locally:

```bash
docker compose up -d --wait
```

```bash
set -a; . ./test-servers.env; set +a; just test
```

### 6. Documentation [#37]

- **The manual (`book/src/`):** the new engine in `data-sources.md` (its fields, TLS, and
  anything unusual about logging in), plus anything that behaves differently in `explorer.md`
  and `consoles.md`.
- **Screenshots** showing the new data source, regenerated with
  `cargo test -p dbrover-app regenerate_screenshots -- --ignored` (see `docs/testing.md`).
- **The website:** the engine in `site/index.html`'s list, with its logo class in
  `site/style.css`.
- **The design docs:**
  - the new crate in `docs/architecture.md`;
  - anything engine-specific in the feature docs, such as `docs/features/sql-runner.md`;
  - the engine's status in `docs/databases.md`;
  - a demo note under its milestone in `docs/roadmap.md`, in the style of the others.

## Checklist

- [ ] The client crate is agreed: TLS on rustls, rows streamed, cancel possible, nothing for
      users to install.
- [ ] The `Dialect` variant is in, with every fact filled in, and lexer, splitter, formatter and
      parser corpus tests.
- [ ] The driver crate implements `Connection`, and passes the whole conformance suite against
      a real server.
- [ ] The `ConnectionConfig` and `DriverKind` variants are in, with a serde round-trip test and
      the engine's `connect` arm.
- [ ] The form, the logo, and everything else the compiler pointed at are done, plus a UI test.
- [ ] The compose service and its `test-servers.env` line are in, and the tests run in CI.
- [ ] The manual, screenshots, website and design docs are updated, with a roadmap demo note.
- [ ] `just lint` and `just test` pass, with the test servers set.

Commit messages, branch names and pull request rules are in
[CONTRIBUTING.md](../CONTRIBUTING.md).
