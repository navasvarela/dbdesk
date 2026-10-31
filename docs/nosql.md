# NoSQL databases

Status: **N0a and N0b built** (2026-10-30), in the layers listed under
[N0a](#n0a--groundwork-languages-safety-and-the-explorer) and
[N0b](#n0b--groundwork-documents); N1 is next. W7 (exports) has landed.

DbRover speaks seven SQL engines. Many of the teams that use them keep a Redis beside their
PostgreSQL, documents in MongoDB, or logs in Elasticsearch, and today they open another tool for
those. This plan adds the most used non-SQL stores to the same explorer, console and grid, with
the same safety levels, history and MCP server. It starts with groundwork that every one of them
needs, because DbRover's console, grid and safety checks all assume SQL today.

[More databases](databases.md) left Redis and MongoDB out on purpose until 2026-10-29: their data
doesn't fit a SQL console with a grid of results. This plan is how it can be made to fit.

## Which databases, and why in this order

Two popularity measures, as of October 2026:

| Database | Kind | DB-Engines rank (Sep 2026) | Stack Overflow survey 2025, share of developers using it |
| --- | --- | --- | --- |
| MongoDB | documents | 5 | 24% |
| Redis | keys and values | 8 | 28% |
| Elasticsearch | search | 11 | 16.7% |
| Apache Cassandra | wide columns | 12 | 2.9% |
| Amazon DynamoDB | keys and documents, cloud only | 18 | 9.8% |
| Neo4j | graph | 20 | 2.6% |
| InfluxDB | time series | 29 | 3.7% |
| OpenSearch | search | 30 | not in the survey's list |
| Couchbase | documents | 45 | not in the survey's list |
| ScyllaDB | wide columns | 65 | not in the survey's list |
| Valkey | keys and values | 102 | 2.4% |

Sources: [Bytebase's snapshot](https://www.bytebase.com/database-periodic-table.md) of the
[DB-Engines ranking](https://db-engines.com/en/ranking) (taken on 2026-09-10), and the
[Stack Overflow Developer Survey 2025](https://survey.stackoverflow.co/2025/technology/).

The order, with the reason for each place:

1. **Redis, with Valkey and Dragonfly.** The most used non-SQL store among developers, and the
   cheapest to add after the groundwork. Its values are flat (strings, hashes, lists, sets,
   sorted sets), so they fit the grid DbRover has; and the server says which of its commands
   write, so the safety checks have something exact to go on. Valkey (the Linux Foundation's
   fork) and Dragonfly speak the same protocol, so one driver serves all three.
2. **MongoDB.** Fifth on DB-Engines and close behind Redis in the survey. It needs the larger
   half of the groundwork: results that are documents, nested and without fixed columns.
3. **Elasticsearch and OpenSearch.** Far more used than Cassandra (16.7% against 2.9%), and
   they return JSON documents too, so they reuse what MongoDB builds. OpenSearch is the fork
   Amazon's managed service runs; one driver serves both.
4. **Cassandra and ScyllaDB.** CQL looks like SQL and returns rows, so it fits the console and
   grid with the fewest changes. It comes last because few developers use it.

Later:

- **Amazon DynamoDB** belongs in the [cloud track](cloud-data.md): it exists only on AWS, signs
  in with an AWS account (C0), and charges for what a query reads, so it needs C0's cost guards.
  It reuses this track's document results, and its PartiQL fits the console.
- **Neo4j** returns nodes and relationships, which only a graph view shows well. That view could
  share ER1's canvas (pan, zoom, boxes and lines), so Neo4j waits for ER1.

Left out for now:

- **InfluxDB.** Its current major version, InfluxDB 3, is queried in SQL (and InfluxQL), so it
  would be one more SQL engine rather than a NoSQL one. It is small in both measures, and time
  series want charts first (W12).
- **Couchbase.** 45th on DB-Engines and not in the survey's list. Its Rust SDK reached 1.0 in
  March 2026, and its query language (SQL++) is close to SQL, so it could follow MongoDB's
  documents later, if people ask.

## What the shared code assumes today

Every engine DbRover has speaks SQL, and the code says so in many places:

- **Every driver has a SQL dialect.** `Dialect::from(DriverKind)` is how the console, the engine,
  the MCP server and the object dialogs learn what they are talking to: about twenty source files
  in `dbrover-sql`, `dbrover-engine`, `dbrover-app` and `dbrover-mcp`. Splitting a script,
  highlighting, completion, formatting, the linter, explain and parameters are all SQL.
- **Results are text with fixed columns** ([ADR 0004](adr/0004-results-as-text.md)). A cell is
  `Option<String>`, and a statement's columns are known before its first row
  (`ResultCursor::columns`, `Event::StatementStarted`). A MongoDB result has neither: each
  document has its own fields, and a field can hold an object or an array. The cell viewer
  pretty-prints JSON, but there is no way to browse a nested value or turn its fields into
  columns.
- **Safety rests on a SQL parser.** `dbrover_sql::classify` parses each statement with
  `sqlparser` and calls it a read, a write, a schema change, transaction control or
  administration. Three things depend on it: the read-only safety level
  (`dbrover-engine/src/read_only.rs`), the MCP server's permissions (`dbrover-mcp`) and the
  warnings before a risky statement (`dbrover-app/src/hazard_dialog.rs`). A statement it can't
  parse is refused. Behind the parser, the database's own read-only transaction (PostgreSQL,
  MySQL, MariaDB) or read-only connection (SQLite) catches what it misses. None of the NoSQL
  engines here has a read-only session; only a user with read-only grants is enforced by the
  database.
- **The explorer is schemas and tables.** `Connection::children` returns one level of the tree
  at a time, all of it at once. A Redis database can hold millions of keys, which no tree can
  load whole.
- **Editing and generated statements are SQL.** Grid edits become `UPDATE`s (`dbrover-sql`'s
  `dml`), **Open data** runs a `SELECT`, the object helpers write DDL, CSV import writes
  `INSERT`s, and W7 adds SQL `INSERT` and `UPDATE` export formats.

## Goals

- **Browse, query and edit** Redis, MongoDB, Elasticsearch, OpenSearch, Cassandra and ScyllaDB in
  the explorer, console and grid DbRover already has, with each engine's own query language in
  the console.
- **Keep every guarantee:** ⌘. cancels, results page, safety levels hold, every command goes to
  the history, and the MCP server never runs a write a project doesn't allow.
- **Nested results that are easy to read:** documents flattened into columns in the grid, and a
  tree for the whole document. SQL engines' JSON columns get the tree too.
- **Nothing to install:** each client is pure Rust and uses rustls, as the SQL engines' do.

Not goals: monitoring, profiling, slow-query analysis, cluster and shard administration, or
backups. The vendors' own tools (RedisInsight, MongoDB Compass, Kibana and OpenSearch
Dashboards) do those well. DbRover browses, queries and edits.

## When to start

Nothing here was to be built until W7's changes to the grid and results landed, because N0b
changes the same code: what a result holds, how the grid draws it, and how exports read it. W7
landed on 2026-10-30, and N0a started the same day.

## Milestones

### N0a — Groundwork: languages, safety and the explorer

Changes every non-SQL engine needs, made against the SQL engines DbRover already has, so each can
be tested with no new driver:

- **Query languages beside SQL.** A closed `QueryLanguage` enum replaces `Dialect` as what a
  driver speaks: `Sql(Dialect)` for the seven engines, and one variant per new language as each
  engine arrives. Each language provides:
  - splitting a script into commands, and the command at the cursor (⌘↵);
  - highlighting tokens;
  - completion words;
  - classifying a command (below);
  - optionally a formatter, an explain prefix and a local syntax check.

  Features with no meaning outside SQL (formatting with SQL rules, parameters, the object
  helpers, CSV import, SQL export formats) ask the language for its `Dialect` and are hidden when
  it has none, rather than each checking the engine. Same rule as DB0: one place to look a fact
  up, and the compiler pointing at every `match` when a language is added.
- **Every language classifies its commands.** `StatementKind` (read, write, schema,
  transaction, admin) stays the one vocabulary for the read-only level, MCP permissions and
  the warnings, and each language fills it in its own way: SQL with `sqlparser`, as now; Redis
  from the server's command flags; MongoDB from a table of shell methods and pipeline stages;
  Elasticsearch from the request's method and path. The rule that an unclassified command is
  refused (unless the project allows `admin`) holds for every language. Each language gets a
  classification corpus, as each SQL dialect has in `crates/dbrover-sql/tests/classify/`.
- **Warnings per language.** The "`UPDATE` without `WHERE`" check generalises to "this command
  may touch everything": `FLUSHALL`, `FLUSHDB` and `KEYS` (which blocks the server while it
  reads every key) on Redis; `deleteMany({})` and `updateMany({}, …)` on MongoDB;
  `_delete_by_query` with `match_all` on Elasticsearch. Each language lists its own.
- **The database's guard where there is no read-only mode.** For an engine without a read-only
  transaction, a read-only data source and an MCP project rely on the command check plus a
  login with read-only grants. The dialog says so, as W11 plans for SQL Server: the console's
  read-only level refuses writes with the check alone, and the MCP server serves such a data
  source only when it has an MCP login.
- **An explorer that pages.** `children` can return part of a level with a way to ask for the
  next part, and can take a filter the server applies. The tree shows **Load more…** at the end
  of a partial level, and the explorer's filter box passes its text to the server when the level
  is partial. Redis keys need it first; a SQL Server or PostgreSQL schema with tens of thousands
  of tables benefits too.
- **Node kinds for what isn't a table:** `Key` (with its Redis type), `KeyPrefix` (a group of
  keys sharing a prefix), `Collection`, `SearchIndex`, `Alias`, `DataStream` and `Keyspace`,
  added to `dbrover-core/src/catalog.rs` as engines arrive, with their icons and the actions
  each offers. **Open data** opens what fits the node: a key viewer for a key, a `find()` for a
  collection.

**Exit:** every SQL feature works as before through `QueryLanguage::Sql`, with its tests
unchanged; a PostgreSQL schema of 20,000 tables loads in the explorer 1,000 at a time, with the
filter applied by the server; and a test language, built only for the tests, has its commands
split, highlighted, classified and refused on a read-only data source.

#### Layers

Each layer is one pull request in a stack, useful and tested on its own. None adds a driver; each
is exercised by the SQL engines and by the test language.

- **N0a·1 — Plan:** this list, the decisions N0a takes (in [Decisions](#decisions)), and the
  roadmap's status.
- **N0a·2 — Query languages** (built): a new crate, `dbrover-language`, holds `QueryLanguage`:
  `Sql(Dialect)` for the seven engines, and `Test`, a language of one command per line built
  only for the tests, behind the crate's `test-language` feature. Each language splits a script
  into commands, finds the command at the cursor, yields tokens to highlight, lists completion
  words, classifies a command into `StatementKind` (and names its schemas, for the MCP server's
  allow-list), and says which `Dialect` it has, if any. `QueryLanguage::from(DriverKind)`
  replaces `Dialect::from(DriverKind)` everywhere. The console keeps a language rather than a
  dialect, and runs, highlights and completes through it; the read-only check and the MCP
  server classify through it. Formatting, the linter, parameters, explain, manual commit mode,
  the object helpers, CSV import, **Open data**, and what the grid does with a result's rows
  (edits, sorting in the database, copying and exporting) ask for the dialect and are hidden or
  refused without one. The test language has a classification corpus, as each SQL dialect has,
  and console tests that split, highlight and complete its commands.
- **N0a·3 — Safety for every language** (built): the warning before a risky command becomes "this
  command may touch everything", listed by each language (`UPDATE` and `DELETE` without `WHERE`
  for SQL). An engine without a read-only session is guarded by its login: the console's
  read-only level refuses writes with the check alone and the dialog says to use a login with
  read-only grants, and the MCP server serves such a data source for reading when, and only when,
  it has an MCP login. SQL Server and ClickHouse are the first to use it; `execute` is refused on
  them, since DbRover has no transaction to run changes in there. Tests: the test language
  refused by the read-only check and planned by the MCP server through its own
  classification, its warnings, and, on the SQL Server test server, MCP serving a data source
  only through its MCP login and reading as that login.
- **N0a·4 — An explorer that pages** (built): `Connection::children_page` returns part of a
  level, with a token for the next part, and takes a filter the server applies; its default
  lists the whole level with `children` and filters it in DbRover. PostgreSQL and SQLite page
  the tables, views, materialized views and sequences of a schema 1,000 at a time, by name
  (keyset paging), with the filter as a case-insensitive `ILIKE` (`LIKE` on SQLite). The tree
  shows **Load more…** at the end of a partial level, and the explorer's filter box asks the
  server again for a partial level once typing pauses for 300 ms; a level loaded whole is still
  filtered by the explorer alone. Tests: a conformance check that the pages add up to the whole
  level and the server's filter matches DbRover's (SQLite, 2,500 tables); engine tests on the
  PostgreSQL test server, 2,500 tables in three pages and the filter, and an ignored one with
  20,000; and a kittest on SQLite that finds a table not yet loaded through the filter, then
  loads more. SQLite pages too, beyond the plan, so the explorer is tested end to end without a
  server. The MCP server's `list_objects` still lists a level whole; N1 pages it for keys.

The manual changes in the layer that changes what a user sees: N0a·3 (read-only data sources and
MCP logins on SQL Server and ClickHouse) and N0a·4 (**Load more…** and the filter).

### N0b — Groundwork: documents

Results that are documents rather than rows, for MongoDB, Elasticsearch and OpenSearch, Redis's
JSON values, and the JSON columns SQL engines already return. Can be built alongside N1.

- **[ADR 0006](adr/0006-documents-as-extended-json-text.md), documents as JSON text,**
  extending ADR 0004 rather than replacing it:
  - A result is either **rows** (today) or **documents**. A document travels as text, one
    Extended JSON string per document (MongoDB's JSON form for every BSON type), so, as with
    ADR 0004, every type shows correctly with no decoding code for each.
  - The **canonical** form is kept, which records each value's type (`{"$numberLong": "7"}`),
    so an edited document goes back with its types unchanged. It is shown the way the shell
    shows it: `ObjectId("…")`, `ISODate("…")`, `NumberLong(7)`.
  - ADR 0004 stays accepted, with a line pointing to 0006.
- **The grid flattens documents into columns:**
  - the columns are the top-level fields of the documents fetched so far, `_id` first and the
    rest in the order they first appear; a field first seen on a later page adds a column at
    the right;
  - a missing field shows as empty and greyed, which is different from `null`;
  - an object or array shows compactly (`{3 fields}`, `[12 items]`), and **Expand column**
    turns an object's fields into columns of their own (`address.city`, `address.zip`);
  - past 200 columns, the rest stay in the tree, and the grid says how many there are.
- **A tree view of the whole document,** in a tab beside the grid (**Grid**, **Tree**, **JSON**):
  fields folded and unfolded, each value with its type, and copying a value, a path
  (`items[2].sku`) or the whole document. The cell viewer gains the same tree for any JSON value,
  so a PostgreSQL `jsonb`, MySQL `JSON` or SQLite JSON text column can be browsed too.
- **Exports** (W7): JSON and JSON Lines write the documents themselves; CSV, TSV, Excel,
  Markdown and HTML write the flattened columns; the SQL formats aren't offered for documents.
  Copying and exporting the rows of a language without a SQL dialect, which N0a leaves hidden,
  come with them.
- **Paging** is as for rows: 500 documents a page through `FetchMore`.

**Exit:** a PostgreSQL query returning a `jsonb` column opens each value as a tree, and a
result of documents from a test cursor shows in the grid with a column added by a later page,
an expanded column, missing fields told apart from `null`, and a JSON Lines export that reads
back to the same documents.

#### Layers

Each layer is one pull request in a stack, useful and tested on its own, as N0a's were. None
adds a driver: the SQL engines' JSON columns and results of documents made up in the tests
exercise each.

- **N0b·1 — Plan:** this list, [ADR 0006](adr/0006-documents-as-extended-json-text.md), the
  decisions N0b takes (in [Decisions](#decisions)), and the roadmap's status.
- **N0b·2 — A tree for JSON values** (built): a new crate, `dbrover-document`, with no engine in it,
  reads JSON text: its own reader, which keeps an object's fields in the order written and each
  number as written. It knows Extended JSON's types and shows each as the shell does
  (`ObjectId("…")`, `ISODate("…")`, `NumberLong(7)`), names each value's type, and writes a
  value's path (`items[2].sku`). The cell viewer shows a JSON object or array as a tree, beside
  the text it shows today: fields folded and unfolded, each value with its type, and **Copy
  value**, **Copy path** and **Copy all** on each. That is the tree for PostgreSQL `json` and
  `jsonb`, MySQL `JSON`, SQLite JSON text, and DuckDB and ClickHouse `JSON`, and the one Redis's
  JSON strings and nested replies will use in N1. Tests: the reader (order, numbers, escapes,
  depth, errors), each Extended JSON type and plain JSON beside it, paths, and a kittest on
  SQLite that opens a JSON value from a query as a tree and copies a path.
- **N0b·3 — Documents in the grid** (built): a result's shape, rows or documents, from the cursor
  (`ResultCursor::shape`) to the console (`Event::StatementStarted`), with each document a row
  of one cell, as ADR 0006 says. The console flattens the documents into columns as the
  milestone describes: `_id` first, a column added at the right by a later page, a missing
  field greyed and told apart from `null`, objects and arrays shown compactly, **Expand
  column** on an object's column, and at most 200 columns, with a note of how many more the
  tree holds. A **Grid**, **Tree** and **JSON** switch above a result of documents shows the
  selected row's document whole. Tests: the flattening (order, later pages, missing and `null`,
  expanding, the cap) in `dbrover-document`, and a console fed a result of documents in two
  pages, as the engine sends a cursor's, in the test language.
- **N0b·4 — Copying and exporting documents and other languages** (built): what N0a left hidden for a
  language without a SQL dialect: **Copy as** and **Export…** of its rows, in every format but
  the SQL ones. For documents, JSON and JSON Lines write the documents themselves, as they came;
  CSV, TSV, Excel, Markdown, HTML and Parquet write the flattened columns. Copying JSON or JSON
  Lines from a selection copies its rows' documents whole. A result of documents exports what
  is loaded; **All rows** and **Run to file** wait for N2 (see [Decisions](#decisions)), and
  Run to file stays SQL's until a language without SQL has a driver. Tests: a JSON Lines
  export of documents that reads back to the same documents, a CSV of the flattened columns,
  and the test language's rows copied and exported without the SQL formats.

The manual changes in N0b·2, the one layer whose change a user sees with today's engines: the
cell viewer's tree. N0b·3 and N0b·4 change what a user sees only once an engine without SQL
arrives, and its milestone's manual chapter describes them.

### N1 — Redis, Valkey and Dragonfly

- **Client: the [`redis`](https://crates.io/crates/redis) crate** (1.8, October 2026), on
  tokio, with rustls through `tokio-rustls-comp` and no crypto provider of its own, so it uses
  ring like the other drivers. It is a community project (BSD-3-Clause) that releases every few
  weeks, and its CI runs against Redis 6 and later and Valkey 7 and later. Redis Inc lists it as
  best-effort rather than supported. `fred`, the alternative, has had no release since
  February 2025; Valkey's own `valkey-glide` has a Rust client, but it is pre-1.0 and tested on
  Linux only. Its 2.0 is in release candidates; take whichever is released when N1 starts.
- **Data source:** the shared server form (port 6379), with the user optional (ACL users; the
  `default` user otherwise), a password, the database number (0 by default) and TLS. SSH tunnels
  work as for the others. Test connection reports Redis, Valkey or Dragonfly and the version,
  from `INFO server`.
- **Explorer:**
  - the databases that hold keys, with their key counts, from `INFO keyspace`;
  - under each, keys grouped into a tree by a separator (`:` by default, set per data source),
    so `user:42:cart` sits under `user` › `42`;
  - keys are read with `SCAN` in pages of 1,000 (N0a's paging), never with `KEYS`; the filter
    box becomes `SCAN`'s `MATCH` pattern, and a type filter its `TYPE` option;
  - each key shows its type's icon, and its TTL when it has one.
- **A viewer per type**, opened by **Open data** on a key, in a tab like a table's data:

  | Type | Shown as | Edits |
  | --- | --- | --- |
  | String | Text; a tree when it parses as JSON (N0b); hex when it isn't UTF-8 | Set the value, keeping the TTL |
  | Hash | A grid of field and value, read with `HSCAN` | Change, add and delete fields |
  | List | A grid of index and value, read with `LRANGE` in pages | Change an item, push, remove |
  | Set | A grid of members, read with `SSCAN` | Add and remove members |
  | Sorted set | A grid of member and score, read with `ZRANGE … WITHSCORES` | Change a score, add, remove |
  | Stream | A grid of entry id and fields, newest first (`XREVRANGE`) | Add an entry, delete one |
  | JSON (the JSON module) | A tree (N0b), from `JSON.GET` | Set a path's value |
  | Anything else | Its type name, size and TTL | None |

  Every viewer's header has the key's TTL (set it, or remove it with `PERSIST`), its memory use
  where the server reports it, **Rename…** and **Delete…**, which asks first. An edit is one
  command, shown in the log and recorded in the history, and runs only after the safety level
  allows it.
- **Console:** one command per line, quoted as `redis-cli` reads them. Completion offers
  command names and their arguments from `COMMAND DOCS`, which include the server's modules.
  Replies show by shape: a list as one column, a map (RESP3) as field and value, a number or
  text as a single cell, and nested replies as a tree (N0b; indented text until it lands).
- **Classification** from the server's own command flags: each command in `COMMAND INFO`
  says whether it is `readonly`, `write`, `admin` or `dangerous`. A table generated from those
  flags ships with DbRover, so the check works before connecting; the session adds modules'
  commands from the server's answer. `MULTI`, `EXEC` and `DISCARD` are transaction control.
  `EVAL` and `FCALL` are writes; `EVAL_RO` and `FCALL_RO` are reads. `SELECT`, `CONFIG`,
  `CLIENT`, `FLUSHALL` and `SHUTDOWN` are admin.
- **Cancel:** Redis runs one command at a time and can't stop most of them once started; a
  command over a large key or `KEYS` blocks until done. So ⌘.:
  - unblocks a blocking command (`BLPOP`, `XREAD BLOCK`) with `CLIENT UNBLOCK` from a second
    connection;
  - stops a script that hasn't written yet with `SCRIPT KILL` or `FUNCTION KILL`;
  - otherwise stops waiting, closes the console's connection and opens a new one, and says the
    server may still be finishing the command.

  This is the limit of the server, not the client, and the manual says so.
- **No transactions, explain or validation.** `transaction_status` is always idle and manual
  commit mode isn't offered: `MULTI` queues commands rather than opening a transaction that can
  be rolled back. There is nothing to explain, and `validate` checks each command's argument
  count against `COMMAND INFO` without running anything.
- **Safety and MCP:** a read-only data source refuses commands the flags call writes, and the
  dialog suggests an ACL user with only `+@read`. The MCP server serves a Redis data source only
  with an MCP login, for example a user made with `ACL SETUSER mcp on >… ~* +@read`. Its
  `query` tool takes a command; `list_objects` pages keys.
- **Dragonfly:** `SCAN`, `TYPE` and `TTL` work as on Redis; `OBJECT` and `SCRIPT KILL` don't,
  and `MEMORY USAGE` partly. The viewers leave out what the server doesn't answer.
  ([Dragonfly's command list](https://www.dragonflydb.io/docs/command-reference/compatibility))
- **CI:** `redis:8-alpine` (40 MB) and `valkey/valkey:9-alpine` (19 MB), both starting in about
  a second, through a conformance suite for key-value engines: the parts of the shared suite
  that apply (lifecycle, connect failure, catalog walk, cancel) plus key viewers and
  classification. Dragonfly (52 MB, from its own registry; Docker Hub's copy is out of date) is
  tried by hand before release.

**Exit:** I connect to Redis 8 and Valkey 9, over TLS too; browse a database of a million keys
grouped by `:` without the server stalling; open a hash, a list, a sorted set and a stream; edit
a field and set a TTL; run `HGETALL` in the console; ⌘. a `BLPOP`; a read-only data source
refuses `SET`; and an MCP client reads a key through an MCP login.

### N2 — MongoDB

- **Client: the official [`mongodb`](https://crates.io/crates/mongodb) driver** (3.9,
  Apache-2.0), on tokio, with rustls on ring by default, and `bson` for documents, which reads
  and writes canonical and relaxed Extended JSON. Replica sets, sharded clusters and
  `mongodb+srv://` addresses (its `dns-resolver` feature) all work through it.
- **Data source:** the shared server form (port 27017) with the authentication database, or a
  connection string pasted whole, which is what MongoDB Atlas hands out. SCRAM passwords first;
  X.509 certificates next; AWS IAM sign-in comes with the cloud track's C0.
- **Explorer:** databases, then collections, views and time-series collections, with their
  indexes. **Show definition** gives `db.createCollection(…)` with the collection's options and
  validator, and a `createIndex(…)` for each index. An estimated document count comes from
  `estimatedDocumentCount`, never a full count.
- **Console: a subset of the shell's syntax**, parsed by DbRover:
  - `db.orders.find({status: "open"}).sort({_id: -1}).limit(50)`, with `projection`, `skip`,
    `hint`, `collation` and `explain()` chained;
  - `aggregate([…])`, `countDocuments`, `distinct`, `findOne`;
  - writes: `insertOne`, `insertMany`, `updateOne`, `updateMany`, `replaceOne`, `deleteOne`,
    `deleteMany`, `findOneAndUpdate`, `bulkWrite`;
  - collections and indexes: `createCollection`, `drop`, `createIndex`, `dropIndex`,
    `renameCollection`;
  - `db.getCollection("name")` for names that aren't identifiers, `db.runCommand({…})`,
    `use <database>`, `show dbs`, `show collections`;
  - arguments are the shell's relaxed JSON (unquoted keys, single quotes, trailing commas,
    comments) with `ObjectId()`, `ISODate()`, `new Date()`, `NumberLong()`,
    `NumberDecimal()`, `UUID()`, `Timestamp()` and `/regular expressions/`.

  No variables, functions or loops: anything else is an error that names what isn't supported.
  No crate parses the shell's syntax, so DbRover writes a small parser, with the
  community `mongosh` crate's parser as a reference rather than a dependency. Commands are
  separated by `;` or by a new line once a call is complete.
- **Results** are documents (N0b), read through the driver's cursor in batches of 500; a cursor
  dropped before its end is closed on the server (`killCursors`) by the driver.
- **Classification:** a table of the methods above, plus the pipeline: an `aggregate` with
  `$out` or `$merge` writes. `db.runCommand` is read only for an allow-list (`ping`,
  `buildInfo`, `listCollections`, `collStats` and similar) and admin otherwise. Users, roles,
  `$function` and `$where` are admin.
- **Cancel:** the driver warns that dropping one of its calls part-way can leave it
  inconsistent, so each command runs in a task of its own, tagged with a `comment` unique to the
  run. ⌘. finds the operation with `$currentOp` on that comment and stops it with `killOp`, from
  the explorer's connection; a user may always kill their own operations.
- **Explain:** ⌥⌘E runs the command through the `explain` command (the driver has no helper),
  with `executionStats`, and shows the plan as a tree.
- **Validate:** for reads, `explain` with `queryPlanner` verbosity plans without running, so it
  catches an unknown operator or a malformed pipeline; writes get the local parse only.
- **Editing:** a field changed in the grid or the tree becomes
  `updateOne({_id: …}, {$set: {"address.city": …}})`, its value written with the type it had
  (canonical Extended JSON keeps it). **Insert document…** and **Delete document** on a
  collection's data. Shown in the log and recorded in the history, like grid edits.
- **Transactions:** none at first: each command commits on its own, and manual commit mode
  isn't offered. Multi-document transactions need a replica set and are left for later.
- **Safety and MCP:** a read-only data source refuses writes by the check, and the dialog
  suggests a user with the `read` role. The MCP server serves a MongoDB data source only with an
  MCP login; its `query` tool takes one shell command, and `list_objects` lists collections.
- **Compatible servers** (Amazon DocumentDB, Azure Cosmos DB for MongoDB, FerretDB) may connect
  through the same driver. Trying each and noting what differs is a small task for later, as
  C1 does for PostgreSQL-compatible databases.
- **CI:** `mongo:8` (302 MB compressed, starts in a few seconds), as a single server, through the
  shared suite's parts that apply plus document results, classification, editing and cancel.

**Exit:** I connect to MongoDB 8 locally and to an Atlas cluster through its connection string;
browse databases and collections; run a `find` with a sort and an `aggregate` and page through
them; expand a nested field into columns and open a document as a tree; edit a field and see it
keep its type; ⌘. a slow aggregation; a read-only data source refuses `deleteMany`; and an MCP
client queries a collection through an MCP login.

### N3 — Elasticsearch and OpenSearch

- **Client: their REST API over HTTP, with the hyper client and `dbrover-tls`'s rustls
  configuration** the ClickHouse driver already uses, so `verify-full` works with a CA file. The
  official `elasticsearch` crate is still an alpha whose last release (August 2025) predates the
  current 9.x servers, and the `opensearch` crate's rustls feature brings in aws-lc-rs and lags
  OpenSearch 3. A console sends requests as typed, so the clients' typed APIs add little.
- **Data source:** the shared server form (port 9200) with a user and password, or an API key;
  an Elastic Cloud address. Signing in to Amazon OpenSearch Service with AWS credentials comes
  with the cloud track's C0. Test connection reports Elasticsearch or OpenSearch and the
  version, from `GET /`.
- **Explorer:** indices (system and hidden ones only with **System** ticked), aliases, data
  streams and index templates; under an index, its fields from the mapping. **Show definition**
  gives the index's mapping and settings as JSON.
- **Console:** requests as Kibana's Dev Tools and OpenSearch Dashboards write them: a line with
  the method and path, then an optional JSON body.

  ```text
  GET /orders/_search
  { "query": { "match": { "status": "open" } }, "sort": [{ "created": "desc" }] }
  ```

  Each request runs on its own, as statements do. Completion offers the common paths and the
  index names. The SQL endpoints (`POST /_sql` on Elasticsearch, in its free Basic licence, and
  `POST /_plugins/_sql` on OpenSearch) are requests like any other, and their answers show as
  rows.
- **Results:** a search's hits are documents (N0b), with `_index`, `_id` and `_score` as columns
  before `_source`'s fields; aggregations and every other answer show as a tree. Paging past the
  first page uses a point in time and `search_after`, which both servers have.
- **Classification** by method and path: `GET`, and `POST` to `_search`, `_count`,
  `_msearch`, `_sql` and similar reads, are reads; `_doc`, `_update`, `_bulk`,
  `_update_by_query` and `_delete_by_query` are writes; creating and deleting indices, mappings
  and templates is a schema change; `_cluster`, `_nodes`, `_snapshot` and security paths are
  admin.
- **Cancel:** each request carries an `X-Opaque-Id` unique to the run; ⌘. finds its task with
  the tasks API and cancels it (`POST /_tasks/<id>/_cancel`), then drops the request.
  Elasticsearch also stops a search when its client disconnects.
- **Editing:** a `_source` field changed in the grid or tree becomes
  `POST /<index>/_update/<id>` with that field, and refreshes so the result shows the change.
- **No transactions, and explain** is the search's `profile` and `_explain`, shown as a tree.
- **Safety and MCP:** as for MongoDB: the check, plus a user or API key with read-only
  privileges, which the MCP server requires as its MCP login.
- **CI:** these servers run on the JVM and are the largest images in this plan:
  `elasticsearch:9` 896 MB and `opensearch:3` 1,160 MB compressed, and each wants a few
  gigabytes of memory, `vm.max_map_count` raised, and its security set up or switched off. One
  of them (Elasticsearch, the more used) runs with the pull request suite if it keeps the Linux
  job under about 15 minutes with its heap held to 512 MB; otherwise both run in a separate job
  or nightly. Measure before deciding.

**Exit:** I connect to Elasticsearch 9 and OpenSearch 3; browse indices and their fields; run a
search with a sort and page past its first hundred hits; see an aggregation as a tree; run a
`_sql` query as rows; edit a field of a document; ⌘. a slow search; and a read-only data source
refuses a `_delete_by_query`.

### N4 — Cassandra and ScyllaDB

- **Client: the [`scylla`](https://crates.io/crates/scylla) crate** (1.9, ScyllaDB's own,
  MIT OR Apache-2.0), which works with Apache Cassandra too. Its rustls feature (`rustls-023`)
  compiles in aws-lc-rs beside ring, as `tiberius`'s does; the driver hands it `dbrover-tls`'s
  ring configuration and installs ring as rustls's process-wide provider, as the SQL Server
  driver does. Ask upstream for a ring-only feature at the same time.
- **Data source:** the shared server form (port 9042) with the local datacenter, which the
  client needs to route requests. Password authentication first.
- **Explorer:** keyspaces, then tables, materialized views, user-defined types, functions and
  aggregates, with columns (partition and clustering keys marked) and secondary indexes.
  **Show definition** uses the server's own `DESCRIBE` where it has one (Cassandra 4.0 and
  later), and is rebuilt from `system_schema` otherwise.
- **Dialect:** a `Dialect` of `dbrover-sql` (see the decisions): `--`, `//` and `/* */`
  comments, `$$` strings, double-quoted names, `BEGIN BATCH … APPLY BATCH` kept as one statement,
  and CQL's function list for completion.
- **Results** are rows with typed columns from the result's metadata, formatted as `cqlsh`
  does; lists, sets, maps and user-defined types show as JSON, browsable in N0b's tree. Paging
  uses the protocol's own, 500 rows a page.
- **Classification** by a small classifier of CQL's statements: `SELECT` reads; `INSERT`,
  `UPDATE`, `DELETE`, `BATCH` and `TRUNCATE` write; `CREATE`, `ALTER` and `DROP` change the
  schema; roles, users, permissions and `USE` of a system keyspace are admin.
- **Validate** prepares the statement, which checks names in the protocol without running it.
- **Cancel:** the protocol has none. ⌘. stops waiting and frees the console; Cassandra finishes
  the statement or stops at its own timeout. On ScyllaDB, every statement runs with
  `USING TIMEOUT` from the data source's statement timeout, so the server stops it too.
- **Editing:** a cell becomes an `UPDATE` by the full primary key, as on the SQL engines.
- **No transactions** (lightweight transactions are a per-statement condition), so manual commit
  mode isn't offered.
- **Safety and MCP:** no read-only session; a role with only `SELECT` is the guard, required as
  the MCP login.
- **CI:** one of the two runs with the pull request suite. `cassandra:5` is the smaller image
  (168 MB) but starts in about a minute on the JVM, with `MAX_HEAP_SIZE` and `HEAP_NEWSIZE`
  set or a small runner kills it; `scylladb/scylla` (275 MB) starts faster with `--smp 1
  --memory 1G --overprovisioned 1`. Measure both; the other runs nightly.

**Exit:** I connect to Cassandra 5 and ScyllaDB; browse keyspaces and tables; page through a
large table; see a map column as a tree; edit a cell; a misspelt column is caught before
running; and a read-only data source refuses an `INSERT`.

### Later: DynamoDB and Neo4j

- **DynamoDB**, in the [cloud track](cloud-data.md) after C0: `aws-sdk-dynamodb`, with its
  ring option rather than its default aws-lc-rs; PartiQL in the console (a subset of SQL, where
  a `SELECT` without an equality on the partition key scans the whole table, which C0's cost
  guards must catch), items as documents (N0b), and tables and indexes in the explorer.
- **Neo4j**, after ER1: Cypher in the console, results as rows of nodes and relationships, and a
  graph view on ER1's canvas. Its Rust client, `neo4rs`, is a Neo4j Labs project, marked
  experimental and unsupported, which speaks Bolt 4.0 to 4.3 only and has been in release
  candidates since 2025. Check it again before planning this.

## Decisions

The first four were agreed on 2026-10-29; the rest are proposed, and each is confirmed when its
milestone starts.

| Question | Decision | Consequence |
| --- | --- | --- |
| The order | **Redis, MongoDB, Elasticsearch and OpenSearch, then Cassandra and ScyllaDB.** Agreed. | Redis needs only N0a; N0b can be built beside it, ready for MongoDB. Elasticsearch comes before Cassandra because many more people use it. |
| MongoDB's console syntax | **A subset of the shell's, parsed by DbRover** with no JavaScript engine. Agreed. | Snippets from MongoDB's documentation run as they are, and every call can be classified. Scripts with variables, loops or functions don't run, and the error says so. |
| Documents and ADR 0004 | **Extend it with ADR 0006**: documents as canonical Extended JSON text, flattened in the grid, with a tree view. Agreed. | No driver's results change; SQL engines' JSON columns get the tree. Typed sorting and filtering of documents stay text-based, as for rows. |
| MCP from the first milestone | **Yes, through an MCP login only**: a Redis ACL user limited to reading, a MongoDB user with the `read` role. Agreed. | An assistant can read Redis and MongoDB data from N1 and N2, and the database enforces what the command check might miss. Without an MCP login, these data sources aren't served, and the log says why. |
| Valkey and Dragonfly | **The Redis driver**, one data source type named "Redis / Valkey". The connection reports which server it reached. | No separate crates. Dragonfly is tried by hand, not in CI. |
| Redis Cluster and Sentinel | **After N1**: a single server, with TLS, ACL users and SSH tunnels, first. | Most local and many managed servers are single; a cluster needs keys scanned on every node. |
| One search driver or two | **One**, for Elasticsearch and OpenSearch, with their differences keyed off the server's answer to `GET /`. | One crate and data source type, "Elasticsearch / OpenSearch". |
| Cassandra's language | **A `Dialect` of `dbrover-sql`**, since CQL lexes and splits like SQL, with its own small classifier, as `sqlparser` has no CQL dialect. | The SQL tooling (splitting, highlighting, completion) works for CQL with a new set of rules, not a new language. |
| Where languages live (N0a) | **A crate of their own, `dbrover-language`,** on top of `dbrover-sql`. It owns `QueryLanguage`; Redis's and MongoDB's languages become modules of it. `StatementKind` and `Unclassified` stay in `dbrover-sql` and are re-exported, and highlighting keeps `dbrover-sql`'s token kinds. | `dbrover-sql` stays about SQL. Code outside the crate asks a language through its methods and never matches on its variants, so a variant added for a new engine, or the test language, changes no other crate. |
| The test language (N0a) | **A variant behind a `test-language` feature,** turned on only in dev-dependencies. | It is compiled into tests only, yet it runs through the same console, check and warnings as a real language. |
| How a document travels (N0b) | **As canonical Extended JSON text, one row of one cell per document,** with the result's shape (rows or documents) from the cursor. See [ADR 0006](adr/0006-documents-as-extended-json-text.md). | Paging, cancelling, the history and the engine's events stay as they are. The console flattens documents, so no driver repeats it. |
| Where documents are read (N0b) | **A crate of their own, `dbrover-document`,** with DbRover's own JSON reader, which keeps fields in order and numbers as written, rather than `serde_json::Value`. | One tree for documents, SQL JSON columns and Redis's JSON. `serde_json` reads numbers into `f64` or 64-bit integers, which would change `1.10` to `1.1`, and its `arbitrary_precision` would change every crate's JSON numbers. |
| Run to file and table exports of documents (N0b) | **Left to N2:** they write rows as the cursor gives them, before any column of a document is known. | N0b exports documents from the grid: what is loaded. N2 adds writing every document to JSON and JSON Lines straight from the cursor. |
| Engines without a read-only session (N0a) | **The login is the guard:** SQL Server and ClickHouse data sources are served to MCP clients, for reading, only with an MCP login. Changes through `execute` wait for a transaction DbRover can open on them. | W11's plan for SQL Server, here for every such engine with logins. W11 still gives ClickHouse its `readonly` setting and DuckDB its read-only file, which are stronger. |

## Risks

- **A shell subset that feels broken.** People paste mongosh scripts with `const`, loops and
  `printjson`. The parser names what it doesn't support and where, and the manual lists what
  runs. If the subset proves too small, a sandboxed JavaScript engine can be weighed again, with
  its cost to the binary and to classification.
- **Commands the check can't see into.** Redis's `EVAL` and `FCALL` run scripts, MongoDB's
  `$function` and `$where` run JavaScript, and an Elasticsearch ingest pipeline can change what
  it indexes. Scripts count as writes unless they use the read-only forms (`EVAL_RO`,
  `FCALL_RO`), and only a login with read-only grants is airtight, as for the SQL engines beyond
  reading. The docs say so plainly.
- **Large keyspaces and collections.** A careless `KEYS *` or a count over a whole collection
  stalls a production server. The explorer only ever scans in pages, counts come from the
  server's own estimates, and the warnings name the commands that read everything.
- **Documents with thousands of fields.** Flattening caps the columns, and the tree shows the
  rest; the grid never grows past what it can draw.
- **CI time.** Redis, Valkey and MongoDB are small and start in seconds. Elasticsearch,
  OpenSearch and Cassandra run on the JVM and take longer; each milestone measures its
  container, and anything that pushes the Linux job past about 15 minutes runs in a separate
  job or nightly instead (see each milestone's CI line).
- **Scope creep toward the vendors' tools.** Each engine's own tool has monitoring, profiling
  and administration. This track stops at browsing, querying and editing, so that each engine
  stays a milestone, not a product.
