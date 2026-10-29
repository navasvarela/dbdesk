# DbRover and other database tools

What the tools people already use do well, and where DbRover fits among them, read on
2026-10-28 and 2026-10-29. This page has four jobs:

- name the tools closest to DbRover and say why;
- compare features area by area;
- rank the gaps worth closing soon;
- say what DbRover brings.

The guide to other tools on dbrover.org and the roadmap's W9 to W13 and ER1 are built from
it. Its start-up and size numbers come from [Measurements](measurements.md).

The site's page credits each tool for what it does well, says plainly what DbRover doesn't do
yet, and leads with what DbRover brings rather than with how it beats anyone. This page is
research, so it ranks and weighs, but it keeps to that tone too.

How to read it:

- **DbRover's facts** come from its own manual and roadmap, linked as `[R…]`. They describe
  0.13.0.
- **Other tools' facts** come from their official sites, docs, pricing pages, release notes and
  GitHub repositories, and each one is linked to its source: `[D…]` DBeaver, `[J…]` DataGrip,
  `[T…]` TablePlus, `[B…]` Beekeeper Studio, `[X…]` DBX, `[L…]` everything else. The sources
  are listed with their dates at the end.
- **Unconfirmed** means no official page said so either way. Where vendors' own pages disagree,
  the cell says so. A gap here is a gap in the docs, not proof the feature is missing.
- Vendors change these things often. Check a fact against its source before quoting it outside
  this repository, and date the quote.

## The field

No survey ranks database GUIs: the Stack Overflow 2025 survey and JetBrains' 2025 developer
ecosystem report list database engines and IDEs only [L1][L2]. Adoption below rests on GitHub
stars and downloads and on marketplace installs, read on 2026-10-28.

Tools were ranked on five questions, the first weighing most:

1. **Audience overlap:** aimed at developers, several databases, desktop, all three systems.
2. **Native and fast:** is it small and quick to start (no JVM, no Electron)?
3. **Safe AI access:** does it serve databases to AI agents over MCP, with permissions?
4. **Adoption:** stars, downloads, installs.
5. **Momentum:** release cadence and growth.

| Rank | Tool | What it does well, and how it relates to DbRover |
| --- | --- | --- |
| 1 | **DBeaver Community** | The default free multi-database client: 52k stars [D2], Apache-2.0, every system and every database DbRover has. Java on Eclipse RCP [D2]; its MCP server and most AI providers are in the paid editions [D6][D9]. The tool most people know already. |
| 2 | **DBX** | The closest to DbRover in spirit, and new: 28k stars since its repository was created on 2026-04-29 [X1]. Free (Apache-2.0), all three systems, "100+" databases, a Rust core under a Vue interface in the system web view (Tauri) [X1], a 43 MiB download for Apple silicon [X3], an AI assistant and an MCP server with read-only, read-write and full-access modes [X1]. It shares DbRover's aims: small, free and open, with safe AI access. |
| 3 | **Beekeeper Studio** | The "friendly open-source SQL client" for the same developers: 24k stars [B2], all systems, Electron [B2]. DuckDB, ClickHouse, ER diagrams and its AI shell are paid [B1]; no official MCP server [B16]. |
| 4 | **DataGrip** | The benchmark for completion, refactoring and inspections, free for non-commercial use since October 2025 [J23]. JVM [J24]. Its built-in MCP server asks for consent before agents read or change schema and data [J9][J22]. |
| 5 | **Tabularis** | Rust and Tauri, Apache-2.0, 5.3k stars since January 2026 [L3], MCP with a read-only mode enforced by the database and approval gates [L4]. Aimed at the same Rust audience as DbRover's soft launch. |
| 6 | **TablePlus** | The "native and fast" reference among paid tools [T1], now with an MCP server whose tokens are read-only, read-write or full access [T14]. Proprietary, perpetual licence [T2]. |
| 7 | **DbGate** | Open source, all systems, DuckDB and ClickHouse; a read-only MCP server in its free build since July 2026 [L5]. Electron and web-first. |

**Watch list:**

- **TablePro**: native Swift, AGPL, MCP and a Safe Mode in its free tier, 6.3k stars, but macOS
  and iOS only [L6].
- **Chat2DB**: 28k stars, built around AI [L7].
- **DBCode**: a VS Code extension with MCP tools and per-connection AI permissions in its free
  tier [L8].
- **Sequel Ace**: macOS and MySQL only, but its MCP server is off by default and read-only by
  default, as DbRover's is [L9].
- **Small Rust clients** on GPUI (DBFlux, OnetCli, ZQLZ) and on egui, DbRover's own stack
  (Ferox, Tabular) [L10]. They will come up on r/rust.

**Narrower or retired:**

- **Azure Data Studio** was retired on 2026-02-28; Microsoft sends its users to VS Code with
  the MSSQL extension [L11].
- **Postico 2** is a fine native Mac app, but for PostgreSQL only and macOS only [L12].

The matrix below compares DbRover with the four tools the launch plan named (DBeaver,
DataGrip, TablePlus and Beekeeper Studio) and with DBX.

## Feature matrix

✅ yes · ⚠️ partly, or only in a paid edition (the cell says which) · ❌ no · ❔ unconfirmed.
DBeaver's column is Community unless a cell names a PRO edition (Lite, Enterprise, Ultimate).
Beekeeper's is Community unless it names a paid tier (Indie, Professional, Business).

### Databases

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| Engines | 7 [R1] | "100+" through JDBC and ODBC [D2] | "over 50" [J1] | "over 20" [T1] | about 25 across tiers [B1] | "100+" [X1] |
| PostgreSQL, MySQL, MariaDB, SQLite, SQL Server | ✅ [R1] | ✅ [D4] | ✅ [J1] | ✅ [T1] | ✅ [B1] | ✅ [X4] |
| DuckDB | ✅ built in [R2] | ❔ listed [D4], edition not stated | ✅ [J10] | ✅ [T3] | ⚠️ Indie and up [B1] | ✅ [X4]; MCP needs a separate driver [X5] |
| ClickHouse | ✅ [R2] | ❔ listed [D4], edition not stated | ✅ [J1] | ✅ [T1] | ⚠️ Professional and up [B1] | ✅ [X4] |
| Oracle | ❌ planned [R2] | ✅ [D4] | ✅ [J1] | ✅ [T1] | ⚠️ Indie and up [B1] | ✅ [X4] |
| MongoDB, Redis | ❌ planned [R16] | ⚠️ PRO [D1] | ✅ [J1] | ✅ [T1] | ⚠️ Redis free, MongoDB Professional [B1] | ✅ [X1] |

### Connections

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| SSH tunnel | ✅ key, agent or password; checks `known_hosts` [R3] | ✅ with jump servers [D13] | ✅ [J7] | ✅ [T4] | ✅ with jump hosts [B3] | ✅ chained with SOCKS or HTTP proxies [X7] |
| TLS with CA file, verify-full | ✅ [R3] | ⚠️ listed as a PRO feature [D1]; Community ❔ | ✅ [J7] | ✅ [T4] | ✅ [B3] | ✅ [X3][X14] |
| AWS IAM / GCP IAM / Entra ID sign-in | ❌ planned, AWS first [R4] | ⚠️ Lite to Ultimate; the edition table says Ultimate only [D1][D14] | ⚠️ cloud account explorers; RDS IAM ❔ [J5] | ⚠️ Entra ID for SQL Server; a password command for the rest [T4] | ⚠️ AWS IAM and Entra ID in Business [B1] | ❔ none found |
| Passwords in the OS keychain | ✅ [R3] | ❔ | ❔ | ✅ [T4] | ❔ | ✅ [X1] |
| `${VARIABLE}` placeholders in connections | ✅ [R3] | ❔ | ❔ | ❔ | ❔ | ❔ |

### SQL editor

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| Completion from the schema | ✅ follows aliases and CTEs, fuzzy [R5] | ✅ [D15] | ✅ including whole JOINs from keys [J6] | ✅ [T5] | ✅ [B9] | ✅ [X8] |
| Formatter | ✅ [R5] | ✅ [D16] | ✅ [J14] | ✅ [T5] | ✅ [B10] | ✅ [X8] |
| Errors and warnings as you type | ✅ lint rules and the database's own check [R5] | ❔ | ✅ inspections [J6] | ❔ | ❔ | ❔ |
| Rename refactoring | ❌ | ❔ | ✅ [J6] | ❔ | ❔ | ❔ |
| Explain plan | ⚠️ text, tree or XML per engine [R5] | ✅ graph view; some engines PRO [D17] | ✅ diagram and flame graph [J15] | ❔ | ❔ | ✅ plan tree with costs [X8] |
| `:name` parameters | ✅ [R6] | ❔ | ❔ | ❔ | ✅ [B9] | ❔ |
| `.sql` files, find and replace | ✅ [R6] | ✅ scripts [D2] | ✅ [J1] | ❔ | ❔ | ❔ |

### Data grid, import and export

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| Edit, add and delete rows | ✅ submitted together, all or none [R7] | ✅ [D3] | ✅ [J8] | ✅ reviewed before ⌘S [T6][T7] | ✅ tables; query results Indie and up [B1] | ✅ shows the SQL before saving [X9] |
| Follow foreign keys | ✅ both ways [R7] | ❔ | ❔ | ❔ | ❔ | ❔ |
| Sort and filter in the database | ✅ [R7] | ❔ | ❔ | ❔ | ❔ | ❔ |
| Export | ⚠️ CSV, JSON; every row streamed [R7] | ✅ CSV, JSON, HTML, Markdown, SQL, TXT; XLSX, XML, Parquet PRO [D18] | ✅ CSV, JSON, HTML, Markdown, Excel, custom [J8] | ✅ CSV, JSON, SQL, Excel [T8] | ⚠️ Indie and up: CSV, Excel, JSON, Markdown, SQL [B1][B12] | ✅ CSV, TSV, JSON, Markdown, XLSX, SQL [X9] |
| Import | ⚠️ CSV with a mapping and preview [R7] | ✅ CSV; XLSX, XML PRO [D18][D20] | ✅ CSV, TSV, dumps [J16] | ✅ CSV, JSON, SQL dump [T8] | ⚠️ Indie and up: CSV, JSON, XLSX [B1][B11] | ✅ CSV, JSON, Excel, SQL; Parquet into DuckDB [X10] |
| Charts from results | ❌ planned (W12) | ❔ | ❔ | ✅ Chart view [T6] | ❔ | ❔ |

### Schema tools

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| Create and change tables without SQL | ✅ shows the DDL before it runs [R8] | ✅ [D2] | ✅ [J8] | ✅ [T9] | ✅ [B1] | ✅ reviewable DDL [X12] |
| Views, indexes, roles | ✅ roles on PostgreSQL [R8] | ✅ [D2] | ✅ [J1] | ⚠️ indexes [T9] | ✅ [B1] | ❔ |
| Copy DDL | ✅ [R8] | ✅ "Generate SQL" [D23] | ❔ | ❔ | ❔ | ❔ |
| Schema compare and migration script | ❌ planned, later (W13) | ⚠️ Enterprise and Ultimate; a simple compare in Community [D21][D22] | ✅ [J17] | ❔ "Sync Database" is new and undocumented [T3] | ❔ | ✅ schema diff and data compare, with a script [X11] |

### ER diagrams

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| Has them | ❌ planned, read-only first (ER1) | ✅ [D2] | ✅ [J18] | ✅ since June 2026 [T3][T11] | ⚠️ paid tiers only [B13] | ✅ [X1] |
| Selected tables, or whole schemas | — | ✅ both: one table, a schema, or a custom diagram of tables from several connections [D23][D28] | ✅ both: a data source, schema or table, and add more by dragging [J18] | ⚠️ a schema, with tables added and removed [T11] | ⚠️ the active database [B13] | ✅ both: a database, chosen schemas, or chosen tables with their related ones [X3][X14] |
| Edit the schema from the diagram | — | ⚠️ Edit mode in Enterprise, Ultimate and Team; it writes a script to review [D1][D23] | ⚠️ through the Modify dialog [J18] | ✅ columns, indexes, foreign keys, committed with ⌘S [T11] | ❔ not described [B13] | ✅ create, alter and drop, synced after showing the SQL; existing keys can't be edited [X14] |
| Export | — | ✅ PNG, GIF, BMP, GraphML; SVG in PRO [D23] | ✅ PNG, GraphML, draw.io, DOT, Mermaid, PlantUML [J18] | ❔ [T11] | ⚠️ an image, or the clipboard [B13] | ✅ SVG, PNG, JSON, DBML, Mermaid [X14] |
| Layout | — | automatic, kept only if asked [D23] | automatic layouts, saved as `.uml` [J18] | rearrange and sort [T11] | ❔ | automatic, saved per view [X3][X14] |

### Query history

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| Kept across restarts | ✅ [R10] | ⚠️ Lite and up; Community clears it at restart [D25] | ✅ [J19] | ✅ [T12] | ❔ [B9] | ✅ [X8] |
| Search | ✅ every data source at once, by text, kind and time [R10] | ⚠️ filters; keyword search ❔ [D25] | ✅ per data source [J19] | ✅ with regular expressions [T3] | ❔ | ✅ by date [X8] |
| Where each statement came from (console, grid, AI agent) | ✅ with the MCP client's name [R10] | ❔ | ⚠️ AI agents' queries are logged [J3] | ❔ | ❔ | ✅ AI or not; a log of MCP calls with the client [X8][X5] |
| A grid edit's old values, and the statement that undoes it | ✅ [R10] | ❔ | ❔ | ❔ | ⚠️ "Query Edit History" [B8] | ❔ |
| Retention and export | ✅ [R10] | ❔ | ❔ | ✅ 1–3 years [T3] | ❔ | ⚠️ retention by count [X8] |

### AI and MCP

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| AI chat inside the app | ❌ bring your own agent over MCP | ⚠️ OpenAI and Copilot; other providers PRO [D6] | ✅ several agents, own key or subscription [J1] | ✅ own key, many providers [T13] | ⚠️ Professional and up, own key [B1][B14] | ✅ own key, many providers, or a local Claude Code or Codex [X16] |
| MCP server | ✅ free, stdio, the app needn't be running [R11] | ⚠️ Lite to Ultimate, documented "since 26.3", not yet in release notes [D9][D24] | ✅ inside the running IDE [J9] | ✅ inside the running app [T14] | ❌ only a community plugin [B16] | ✅ free, a separate install; the app needn't be running [X5] |
| Read-only by default | ✅ [R11] | ❔ | ⚠️ asks consent; advises a read-only database user [J9][J22] | ⚠️ per token; "approval for unsafe query" on by default [T14] | — | ❔ |
| Read-only enforced by the database, not only by parsing | ✅ read-only transactions; SQLite opened read-only [R11] | ❔ | ❌ relies on the database user [J9] | ❔ | — | ❌ "depends on classifying SQL" [X6] |
| Scope | per project, optionally a list of schemas [R11] | per connection [D9] | ❔ | per token: all, or chosen connections or groups [T14] | — | per database, connection or group, with allow-lists of connections, databases and tools [X5] |
| Separate permissions for writes, schema changes and admin | ✅ [R11] | ❔ | ✅ four consents [J22] | ⚠️ read & write, or full access [T14] | — | ⚠️ read only, data read and write, or full access [X5] |
| A dedicated database login for agents | ✅ [R11] | ❔ | ⚠️ advised, not built in [J9] | ❔ | — | ⚠️ advised [X6] |
| Row limit and statement timeout | ✅ [R11] | ❔ | ❔ | ❔ | — | ❔ |

### Safety

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| Colour per connection | ✅ [R3] | ✅ the red "Production" type [D11] | ✅ [J3] | ✅ tags and a status-bar colour [T4] | ❔ | ✅ [X1] |
| Ask before changes | ✅ Careful: every change, naming the data source; consoles start in manual commit [R3] | ✅ Production type: confirm SQL and data changes, auto-commit off [D11] | ❔ | ✅ five Safe Mode levels, up to a password per query [T15] | ❔ | ✅ production protection: every write asks again [X6] |
| Read-only connection | ✅ enforced by the database where it can be [R3] | ✅ with finer restrictions [D12] | ✅ [J7] | ❔ beyond Safe Mode [T15] | ✅ free since 6.1 [B17] | ✅ by parsing; can be lifted for a few minutes [X6] |
| `UPDATE`/`DELETE` without `WHERE` | ✅ asks [R5] | ❔ | ✅ inspection [J6] | ❔ | ❔ | ❔ |
| Typed name before dropping on production | ✅ [R8] | ❔ | ❔ | ❔ | ❔ | ❔ |

### Team, platforms, licence and price

| | DbRover 0.13 | DBeaver | DataGrip | TablePlus | Beekeeper | DBX |
| --- | --- | --- | --- | --- | --- | --- |
| Account needed | ❌ none [R12] | ❌ none described for Community [D27] | ✅ to activate, even the free licence [J23] | ❌ a licence key [T2] | ❌ for Community [B1] | ❌ [X17] |
| Share connections | ⚠️ export and import files without secrets; plain TOML [R13] | ⚠️ Git sync in Enterprise and Ultimate; shared workspaces in Team [D1] | ⚠️ templates in your JetBrains Account [J22] | ⚠️ exported files, or a synced folder [T4] | ⚠️ cloud workspaces, Professional and up [B1] | ⚠️ sync over WebDAV or a private Git snippet, secrets left out by default [X18] |
| Settings as plain files you can version | ✅ keymap, actions, snippets, themes [R14] | ❔ | ❔ | ❔ | ❔ | ❔ |
| macOS, Windows, Linux | ✅ [R12] | ✅ [D27] | ✅ [J24] | ⚠️ Linux ❔ [T1][T16] | ✅ [B2] | ✅ and a web version [X19] |
| Built with | Rust and egui, drawn natively [R15] | Java, Eclipse RCP [D2] | IntelliJ platform, JVM [J24] | native [T1] | Electron [B2] | Rust, with a Vue interface in the system web view [X1] |
| Licence | MIT or Apache-2.0 [R12] | Apache-2.0; PRO commercial [D2] | proprietary [J23] | proprietary [T2] | GPL-3.0; paid parts source-available [B2] | Apache-2.0 [X1] |
| Price | free | free; PRO $113–510 a year [D29][D31][D32] | free for non-commercial use; else a subscription [J2][J23] | $99 perpetual, one year of updates [T2] | free; $9–18 a user a month [B1] | free [X1] |

### Start-up and size

Measured on one Mac, as [Measurements](measurements.md) describes: an Apple M4 Pro with 24 GB,
macOS 27.2, warm starts, median of five. TablePlus and Beekeeper Studio weren't measured. The
site quotes DbRover's own numbers as facts, not as a comparison.

| | DbRover 0.13 | DBeaver | DataGrip | DBX |
| --- | --- | --- | --- | --- |
| Main window up | 0.35 s | 3.6 s | 1.8 s | 0.38 s |
| Download for Apple silicon | 75 MiB, universal | 117 MiB | 980 MiB | 43 MiB |
| Installed | 174 MiB | 199 MiB | 2695 MiB | 77 MiB |

## Gaps worth closing soon

Ranked by how often DbRover's audience (developers who query PostgreSQL, MySQL and SQLite every
day, many of them on AWS) would miss each one, against the effort. Sizes are rough, in the
milestones' terms: **S** is a pull request or two, **M** a stack of three to five, **L** a
track of its own.

| # | Gap | Who has it | Hit how often | Size | Why this rank |
| --- | --- | --- | --- | --- | --- |
| 1 | **ER diagrams:** chosen tables or a whole schema with their foreign keys, laid out automatically, exported as PNG or SVG. Read-only first. | All five: DBeaver, DataGrip, TablePlus, DBX; Beekeeper in paid tiers | Weekly for anyone learning or explaining a schema | **M** read-only with export; **L** with editing | Every tool in the matrix has them, and they are the most visual thing a database tool shows: a diagram reads in a screenshot or a clip where a grid doesn't. The drivers already load each table's foreign keys both ways (W5), and egui's `Scene` pans and zooms, so the work is layout, drawing and export. **Decided on 2026-10-29:** planned, not a top priority; the roadmap's ER1 is placed after the near-term work, and Vision no longer lists ER diagrams as a non-goal. |
| 2 | **A password command:** a data source runs a command to get its password, so AWS RDS IAM, `gcloud` and Entra ID tokens, 1Password or Vault work today. | TablePlus [T4], Postico [L12] | Daily for teams on managed databases | **S** | Brings cloud sign-in long before C0 and C1 land, on H2's trusted-command machinery. C0's built-in sign-in stays the long-term answer. |
| 3 | **A visual explain plan:** PostgreSQL's JSON plan as a tree with each step's cost and rows, the costliest highlighted; MySQL and SQL Server next. | DBeaver, DataGrip, DBX | Weekly for anyone tuning a query | **M** | Helps with the slow query in front of you, and shows well in a clip. Each engine's plan format needs its own parser. |
| 4 | **Read-only on every engine:** DuckDB's and ClickHouse's own read-only modes, and SQL Server served to MCP clients through an MCP login. | — | Every agent session on those engines | **S–M** | Makes the safety pitch hold on all seven engines, not four. ClickHouse has a `readonly` setting per query and DuckDB opens a file read-only; SQL Server has no read-only transaction, so it needs the dedicated login. |
| 5 | **Schema compare:** two schemas or data sources, their differences, and a script that turns one into the other. | DataGrip, DBX; DBeaver Enterprise | Monthly, before a release | **L** | DBX includes it for free, so people will ask for it. Most teams keep migrations in code, which keeps it below the others. **Picked on 2026-10-29** as W13, later; no longer a non-goal in Vision. |
| 6 | **Charts from results:** a bar, line or scatter chart of a result's columns. | TablePlus | Monthly | **M** | Visual and easy to show, but most developers chart elsewhere. |
| — | **Export and report formats** | All five | Weekly | — | Planned separately, as W7 (exports) and W8 (your own formats) in [the roadmap](roadmap.md); this page keeps them to the matrix's rows. |
| — | **AI chat inside the app** | All five, some only in paid editions | — | — | Not proposed. DbRover serves the agents people already use over MCP rather than brokering a model. The comparison page says so. |
| — | **Rename refactoring across SQL** | DataGrip | Rarely | **L** | Not proposed: it needs a full SQL language server, a non-goal. |

All six are in [the roadmap](roadmap.md), picked on 2026-10-29: W9 the password command, W10
the visual explain plan, W11 read-only on every engine, W12 charts and W13 schema compare in the
well-rounded track, and ER1 in [the diagrams track](roadmap.md#diagrams-track).

## What DbRover brings

The ideas DbRover is built around, and the ones it shares with other tools. Each holds among
DBeaver, DataGrip, TablePlus, Beekeeper Studio and DBX as their own docs describe them.

1. **A native app, free and open source on all three systems, with no account.** DbRover draws
   its window itself, in Rust with egui, so it opens in about a third of a second and stays
   light. TablePlus is native too, and paid; DBX is free and quick to start, with its interface
   in the system's web view.
2. **AI agents whose reads the database itself keeps read-only.** DbRover's MCP server runs
   every read inside the database's own read-only transaction (and opens SQLite files
   read-only), so even a write hidden in a function fails. The server is part of the app, off
   until a project is allowed, read-only by default, with separate permissions for writes,
   schema changes and admin, a row limit and a timeout, and a login of its own set in the data
   source's dialog. DataGrip, TablePlus and DBX serve MCP too, with consent prompts and
   permission levels of their own [J9][T14][X5].
3. **Actions that run your own programs on the selected rows,** declared in a TOML file next to
   your keymap, snippets and theme. No other tool here documents an equivalent.

Shared with one or two of the others, and done well:

- **Safety levels.** A careful data source starts its consoles in manual commit, names itself in
  every confirmation, and waits for a table's name before dropping it. DBeaver's Production
  type, DBX's production protection and TablePlus's Safe Mode follow the same idea.
- **A history you can audit.** Every statement from every data source in one search, tagged by
  where it came from down to the AI client, a grid edit's old values with the statement that
  puts them back, and passwords redacted before they are stored. DBX keeps a log of MCP calls
  with the client; Beekeeper has a history of grid edits.
- **Settings as plain files.** Keymap, actions, snippets, themes and data sources (without their
  secrets) are files you can read, edit and keep in Git.

What DbRover doesn't do yet, said as plainly on the site: ER diagrams, schema compare, a visual
explain plan, cloud sign-in, Oracle, NoSQL databases, export beyond CSV and JSON (planned as
W7), charts, AI chat inside the app, and syncing settings between machines.

## Sources

Read on 2026-10-28 unless a line says otherwise. Each tag in the tables links here and to its
page.

**DbRover**

- R1: [Manual: Introduction](../book/src/introduction.md)
- R2: [More databases (and the roadmap's database track)](databases.md)
- R3: [Manual: Data sources (TLS, SSH tunnels, safety levels, placeholders, the keychain)](../book/src/data-sources.md)
- R4: [Cloud and big data (planned)](cloud-data.md)
- R5: [Manual: Query consoles (completion, formatting, problems, explain)](../book/src/consoles.md)
- R6: [Manual: Files, parameters and find](../book/src/files-parameters.md)
- R7: [Manual: Working with table data](../book/src/table-data.md)
- R8: [Manual: Creating and changing tables](../book/src/objects.md)
- R10: [Manual: The query history](../book/src/history.md)
- R11: [Manual: Connecting an AI assistant](../book/src/mcp.md)
- R12: [Manual: Install and first launch](../book/src/install.md)
- R13: [Manual: Moving data sources to another machine](../book/src/sharing.md)
- R14: [Manual: Actions, snippets and themes](../book/src/customising.md)
- R15: [ADR 0001: the UI toolkit is egui](adr/0001-ui-toolkit-egui.md)
- R16: [NoSQL databases (planned)](nosql.md)

**DBeaver**

- D1: <https://dbeaver.com/edition/> (read 2026-10-28)
- D2: <https://github.com/dbeaver/dbeaver> (2026-10-28)
- D3: <https://dbeaver.io/> (2026-10-28)
- D4: <https://dbeaver.com/databases/> (2026-10-28)
- D6: <https://dbeaver.com/docs/dbeaver/AI-Smart-Assistance/> (2026-10-28)
- D9: <https://dbeaver.com/docs/dbeaver/Model-Context-Protocol-Server/> (2026-10-28)
- D11: <https://dbeaver.com/docs/dbeaver/Connection-Types/> (2026-10-28)
- D12: <https://dbeaver.com/docs/dbeaver/Managing-security-restrictions-for-database-connection/> (2026-10-28)
- D13: <https://dbeaver.com/docs/dbeaver/SSH-Configuration/> (2026-10-28)
- D14: <https://dbeaver.com/docs/dbeaver/AWS-Credentials/> (2026-10-28)
- D15: <https://dbeaver.com/docs/dbeaver/SQL-Assist-and-Auto-Complete/> (2026-10-28)
- D16: <https://dbeaver.com/docs/dbeaver/SQL-Formatting/> (2026-10-28)
- D17: <https://dbeaver.com/docs/dbeaver/Query-Execution-Plan/> (2026-10-28)
- D18: <https://dbeaver.com/docs/dbeaver/Data-transfer/> (2026-10-28)
- D20: <https://dbeaver.com/docs/dbeaver/Data-import/> (2026-10-28)
- D21: <https://dbeaver.com/docs/dbeaver/Schema-compare/> (2026-10-28)
- D22: <https://dbeaver.com/docs/dbeaver/Simple-Structure-Compare/> (2026-10-28)
- D23: <https://dbeaver.com/docs/dbeaver/ER-Diagrams/> (2026-10-28, re-read 2026-10-29)
- D24: <https://dbeaver.com/dbeaver-lite-26-2/> (via official-site search listing, 2026-10-28; page not opened)
- D25: <https://dbeaver.com/docs/dbeaver/Query-Manager/> (2026-10-28)
- D27: <https://dbeaver.io/download/> (2026-10-28)
- D28: <https://dbeaver.com/docs/dbeaver/Custom-Diagrams/> (2026-10-29)
- D29: <https://dbeaver.com/dbeaver-lite/> (2026-10-28)
- D31: <https://dbeaver.com/dbeaver-enterprise/> (2026-10-28)
- D32: <https://dbeaver.com/dbeaver-ultimate/> (2026-10-28)

**DataGrip**

- J1: <https://www.jetbrains.com/datagrip/> (rendered page incl. FAQ, 2026-10-28)
- J2: <https://www.jetbrains.com/datagrip/buy/> (rendered; ?section=personal and ?section=commercial, billing=yearly/monthly, 2026-10-28)
- J3: <https://www.jetbrains.com/datagrip/whatsnew/> (2026.2, rendered, 2026-10-28)
- J5: <https://www.jetbrains.com/datagrip/whatsnew/2025-3/> (2026-10-28)
- J6: <https://lp.jetbrains.com/datagrip/sql-support/> (2026-10-28)
- J7: <https://www.jetbrains.com/help/datagrip/data-sources-and-drivers-dialog.html> (2026-10-28)
- J8: <https://lp.jetbrains.com/datagrip/working-with-data/> (2026-10-28)
- J9: <https://www.jetbrains.com/help/datagrip/mcp-server.html> (2026-10-28)
- J10: <https://www.jetbrains.com/help/datagrip/duckdb.html> (2026-10-28)
- J14: <https://www.jetbrains.com/help/datagrip/reformat-file-dialog.html> (2026-10-28)
- J15: <https://www.jetbrains.com/help/datagrip/query-execution-plan.html> (2026-10-28)
- J16: <https://www.jetbrains.com/help/datagrip/import-data.html> (2026-10-29)
- J17: <https://www.jetbrains.com/help/datagrip/schema-comparison-and-migration.html> (via official search results, 2026-10-28)
- J18: <https://www.jetbrains.com/help/datagrip/creating-diagrams.html> (2026-10-28, re-read 2026-10-29)
- J19: <https://www.jetbrains.com/help/datagrip/run-a-query.html> and https://www.jetbrains.com/help/datagrip/find-recent-queries-and-files.html (via official search results, 2026-10-28)
- J22: <https://www.jetbrains.com/datagrip/whatsnew/2026-1/> (rendered, 2026-10-28)
- J23: <https://blog.jetbrains.com/datagrip/2025/10/01/datagrip-is-now-free-for-non-commercial-use/> (via search; WebFetch returned 403, 2026-10-28)
- J24: <https://www.jetbrains.com/help/datagrip/installation-guide.html> (2026-10-28)

**TablePlus**

- T1: <https://tableplus.com/> (2026-10-28)
- T2: <https://tableplus.com/pricing> (2026-10-28)
- T3: <https://tableplus.com/osx/changelog> (2026-10-28)
- T4: <https://tableplus.com/docs/gui-tools/manage-connections> (2026-10-28)
- T5: <https://tableplus.com/docs/query-editor/autocomplete> (via official search results, 2026-10-28)
- T6: <https://tableplus.com/docs/query-editor/working-with-query-results> (2026-10-28)
- T7: <https://tableplus.com/docs/gui-tools/code-review-and-safemode> (2026-10-29)
- T8: <https://tableplus.com/docs/gui-tools/import-and-export> (2026-10-29)
- T9: <https://docs.tableplus.com/gui-tools/working-with-table/column> (via official search results, 2026-10-28)
- T11: <https://tableplus.com/docs/gui-tools/diagram> (2026-10-29)
- T12: <https://tableplus.com/docs/query-editor/query-history> (2026-10-28)
- T13: <https://tableplus.com/docs/llm-plugin> (2026-10-28)
- T14: <https://tableplus.com/docs/preferences/mcp> (2026-10-28)
- T15: <https://tableplus.com/docs/preferences/crash-and-security> (2026-10-28)
- T16: <https://tableplus.com/linux> (2026-10-28)

**Beekeeper Studio**

- B1: <https://www.beekeeperstudio.io/pricing/> (rendered comparison table and price cards, monthly and yearly, 2026-10-28)
- B2: <https://github.com/beekeeper-studio/beekeeper-studio> (2026-10-28)
- B3: <https://docs.beekeeperstudio.io/user_guide/connecting/connecting/> (2026-10-28)
- B8: <https://www.beekeeperstudio.io/category/release/> (2026-10-28)
- B9: <https://docs.beekeeperstudio.io/user_guide/sql_editor/editor/> (2026-10-28)
- B10: <https://docs.beekeeperstudio.io/user_guide/sql-query-formatter/> (listed in docs index, 2026-10-28; not opened)
- B11: <https://docs.beekeeperstudio.io/user_guide/importing-data-csv-json-etc/> (2026-10-28)
- B12: <https://docs.beekeeperstudio.io/user_guide/data-export/> (2026-10-28)
- B13: <https://docs.beekeeperstudio.io/user_guide/entity-relationship-diagrams-erd/> (2026-10-28, re-read 2026-10-29)
- B14: <https://www.beekeeperstudio.io/features/sql-ai> (2026-10-28)
- B16: <https://www.beekeeperstudio.io/plugins/gatekeeper/> (2026-10-28)
- B17: <https://github.com/beekeeper-studio/beekeeper-studio/releases/tag/v6.1.0> (2026-10-28)

**DBX**

- X1: <https://github.com/t8y2/dbx> (README, read via GitHub API, 2026-10-29)
- X3: <https://github.com/t8y2/dbx/releases> (latest v0.6.60, 2026-10-28T20:12Z; asset sizes and release notes v0.3.6–v0.6.60 via GitHub API, 2026-10-29)
- X4: <https://dbxio.com/en/docs/databases> (source: docs/content/docs/databases.mdx, 2026-10-29)
- X5: <https://dbxio.com/en/docs/mcp> (source: docs/content/docs/mcp.mdx) and https://github.com/t8y2/dbx/blob/main/packages/mcp-server/README.md (2026-10-29)
- X6: <https://dbxio.com/en/docs/production-safety> (source .mdx read, and live page spot-checked, 2026-10-29)
- X7: <https://dbxio.com/en/docs/ssh-tunnel> (source .mdx, 2026-10-29)
- X8: <https://dbxio.com/en/docs/query-editor> (source .mdx, 2026-10-29)
- X9: <https://dbxio.com/en/docs/data-grid> (source .mdx, 2026-10-29)
- X10: <https://dbxio.com/en/docs/table-import> (source .mdx, 2026-10-29)
- X11: <https://dbxio.com/en/docs/schema-diff> (source .mdx, 2026-10-29)
- X12: <https://dbxio.com/en/docs/what-is-dbx> and https://dbxio.com/en/docs/getting-started (source .mdx, 2026-10-29)
- X14: <https://github.com/t8y2/dbx/blob/main/apps/desktop/src/i18n/locales/en.ts> (official app UI strings, `diagram` and connection-form sections, 2026-10-29)
- X16: <https://dbxio.com/en/docs/ai-assistant> (source .mdx, 2026-10-29)
- X17: <https://dbxio.com/en/privacy> (effective 2026-09-27; read 2026-10-29)
- X18: <https://dbxio.com/en/docs/cloud-sync> (source .mdx, 2026-10-29)
- X19: <https://dbxio.com/> (2026-10-29)

**Other tools and surveys**

- L1: [Stack Overflow Developer Survey 2025, Technology](https://survey.stackoverflow.co/2025/technology/)
- L2: [JetBrains State of Developer Ecosystem 2025, Tools and trends](https://devecosystem-2025.jetbrains.com/tools-and-trends)
- L3: [Tabularis repository (stars and creation date via the GitHub API)](https://github.com/TabularisDB/tabularis)
- L4: [Tabularis MCP server docs](https://tabularis.dev/wiki/mcp-server)
- L5: [DbGate 7.2.3 release notes (MCP server)](https://www.dbgate.io/news/2026-07-21-7-2-3/)
- L6: [TablePro repository](https://github.com/TableProApp/TablePro)
- L7: [Chat2DB repository](https://github.com/OtterMind/Chat2DB)
- L8: [DBCode on the VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=DBCode.dbcode)
- L9: [Sequel Ace repository (MCP server in the README)](https://github.com/Sequel-Ace/Sequel-Ace)
- L10: [DBFlux; also https://github.com/feigeCode/gpui-kit (OnetCli), https://github.com/samurmaykrr/zqlz (ZQLZ), https://github.com/frkdrgt/ferox (Ferox), https://github.com/cmjcode/tabular (Tabular)](https://github.com/0xErwin1/dbflux)
- L11: [What's happening to Azure Data Studio (page updated 2026-10-19)](https://learn.microsoft.com/en-us/sql/tools/whats-happening-azure-data-studio?view=sql-server-ver17)
- L12: [Postico 2: pre-connect script (and https://www.eggerapps.at/postico2/)](https://eggerapps.at/postico2/documentation/connection-preconnect-script.html)

[R1]: ../book/src/introduction.md
[R2]: databases.md
[R3]: ../book/src/data-sources.md
[R4]: cloud-data.md
[R5]: ../book/src/consoles.md
[R6]: ../book/src/files-parameters.md
[R7]: ../book/src/table-data.md
[R8]: ../book/src/objects.md
[R10]: ../book/src/history.md
[R11]: ../book/src/mcp.md
[R12]: ../book/src/install.md
[R13]: ../book/src/sharing.md
[R14]: ../book/src/customising.md
[R15]: adr/0001-ui-toolkit-egui.md
[D1]: https://dbeaver.com/edition/
[D2]: https://github.com/dbeaver/dbeaver
[D3]: https://dbeaver.io/
[D4]: https://dbeaver.com/databases/
[D6]: https://dbeaver.com/docs/dbeaver/AI-Smart-Assistance/
[D9]: https://dbeaver.com/docs/dbeaver/Model-Context-Protocol-Server/
[D11]: https://dbeaver.com/docs/dbeaver/Connection-Types/
[D12]: https://dbeaver.com/docs/dbeaver/Managing-security-restrictions-for-database-connection/
[D13]: https://dbeaver.com/docs/dbeaver/SSH-Configuration/
[D14]: https://dbeaver.com/docs/dbeaver/AWS-Credentials/
[D15]: https://dbeaver.com/docs/dbeaver/SQL-Assist-and-Auto-Complete/
[D16]: https://dbeaver.com/docs/dbeaver/SQL-Formatting/
[D17]: https://dbeaver.com/docs/dbeaver/Query-Execution-Plan/
[D18]: https://dbeaver.com/docs/dbeaver/Data-transfer/
[D20]: https://dbeaver.com/docs/dbeaver/Data-import/
[D21]: https://dbeaver.com/docs/dbeaver/Schema-compare/
[D22]: https://dbeaver.com/docs/dbeaver/Simple-Structure-Compare/
[D23]: https://dbeaver.com/docs/dbeaver/ER-Diagrams/
[D24]: https://dbeaver.com/dbeaver-lite-26-2/
[D25]: https://dbeaver.com/docs/dbeaver/Query-Manager/
[D27]: https://dbeaver.io/download/
[D28]: https://dbeaver.com/docs/dbeaver/Custom-Diagrams/
[D29]: https://dbeaver.com/dbeaver-lite/
[D31]: https://dbeaver.com/dbeaver-enterprise/
[D32]: https://dbeaver.com/dbeaver-ultimate/
[J1]: https://www.jetbrains.com/datagrip/
[J2]: https://www.jetbrains.com/datagrip/buy/
[J3]: https://www.jetbrains.com/datagrip/whatsnew/
[J5]: https://www.jetbrains.com/datagrip/whatsnew/2025-3/
[J6]: https://lp.jetbrains.com/datagrip/sql-support/
[J7]: https://www.jetbrains.com/help/datagrip/data-sources-and-drivers-dialog.html
[J8]: https://lp.jetbrains.com/datagrip/working-with-data/
[J9]: https://www.jetbrains.com/help/datagrip/mcp-server.html
[J10]: https://www.jetbrains.com/help/datagrip/duckdb.html
[J14]: https://www.jetbrains.com/help/datagrip/reformat-file-dialog.html
[J15]: https://www.jetbrains.com/help/datagrip/query-execution-plan.html
[J16]: https://www.jetbrains.com/help/datagrip/import-data.html
[J17]: https://www.jetbrains.com/help/datagrip/schema-comparison-and-migration.html
[J18]: https://www.jetbrains.com/help/datagrip/creating-diagrams.html
[J19]: https://www.jetbrains.com/help/datagrip/run-a-query.html
[J22]: https://www.jetbrains.com/datagrip/whatsnew/2026-1/
[J23]: https://blog.jetbrains.com/datagrip/2025/10/01/datagrip-is-now-free-for-non-commercial-use/
[J24]: https://www.jetbrains.com/help/datagrip/installation-guide.html
[T1]: https://tableplus.com/
[T2]: https://tableplus.com/pricing
[T3]: https://tableplus.com/osx/changelog
[T4]: https://tableplus.com/docs/gui-tools/manage-connections
[T5]: https://tableplus.com/docs/query-editor/autocomplete
[T6]: https://tableplus.com/docs/query-editor/working-with-query-results
[T7]: https://tableplus.com/docs/gui-tools/code-review-and-safemode
[T8]: https://tableplus.com/docs/gui-tools/import-and-export
[T9]: https://docs.tableplus.com/gui-tools/working-with-table/column
[T11]: https://tableplus.com/docs/gui-tools/diagram
[T12]: https://tableplus.com/docs/query-editor/query-history
[T13]: https://tableplus.com/docs/llm-plugin
[T14]: https://tableplus.com/docs/preferences/mcp
[T15]: https://tableplus.com/docs/preferences/crash-and-security
[T16]: https://tableplus.com/linux
[B1]: https://www.beekeeperstudio.io/pricing/
[B2]: https://github.com/beekeeper-studio/beekeeper-studio
[B3]: https://docs.beekeeperstudio.io/user_guide/connecting/connecting/
[B8]: https://www.beekeeperstudio.io/category/release/
[B9]: https://docs.beekeeperstudio.io/user_guide/sql_editor/editor/
[B10]: https://docs.beekeeperstudio.io/user_guide/sql-query-formatter/
[B11]: https://docs.beekeeperstudio.io/user_guide/importing-data-csv-json-etc/
[B12]: https://docs.beekeeperstudio.io/user_guide/data-export/
[B13]: https://docs.beekeeperstudio.io/user_guide/entity-relationship-diagrams-erd/
[B14]: https://www.beekeeperstudio.io/features/sql-ai
[B16]: https://www.beekeeperstudio.io/plugins/gatekeeper/
[B17]: https://github.com/beekeeper-studio/beekeeper-studio/releases/tag/v6.1.0
[X1]: https://github.com/t8y2/dbx
[X3]: https://github.com/t8y2/dbx/releases
[X4]: https://dbxio.com/en/docs/databases
[X5]: https://dbxio.com/en/docs/mcp
[X6]: https://dbxio.com/en/docs/production-safety
[X7]: https://dbxio.com/en/docs/ssh-tunnel
[X8]: https://dbxio.com/en/docs/query-editor
[X9]: https://dbxio.com/en/docs/data-grid
[X10]: https://dbxio.com/en/docs/table-import
[X11]: https://dbxio.com/en/docs/schema-diff
[X12]: https://dbxio.com/en/docs/what-is-dbx
[X14]: https://github.com/t8y2/dbx/blob/main/apps/desktop/src/i18n/locales/en.ts
[X16]: https://dbxio.com/en/docs/ai-assistant
[X17]: https://dbxio.com/en/privacy
[X18]: https://dbxio.com/en/docs/cloud-sync
[X19]: https://dbxio.com/
[L1]: https://survey.stackoverflow.co/2025/technology/
[L2]: https://devecosystem-2025.jetbrains.com/tools-and-trends
[L3]: https://github.com/TabularisDB/tabularis
[L4]: https://tabularis.dev/wiki/mcp-server
[L5]: https://www.dbgate.io/news/2026-07-21-7-2-3/
[L6]: https://github.com/TableProApp/TablePro
[L7]: https://github.com/OtterMind/Chat2DB
[L8]: https://marketplace.visualstudio.com/items?itemName=DBCode.dbcode
[L9]: https://github.com/Sequel-Ace/Sequel-Ace
[L10]: https://github.com/0xErwin1/dbflux
[L11]: https://learn.microsoft.com/en-us/sql/tools/whats-happening-azure-data-studio?view=sql-server-ver17
[L12]: https://eggerapps.at/postico2/documentation/connection-preconnect-script.html
