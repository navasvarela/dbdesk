# Cloud and big data

Status: **planned**, not scheduled. Proposed on 2026-10-27; the order and the decisions below are
confirmed when the track starts.

dbdesk connects to databases you run or reach by host and port. Much of the data people query
today lives elsewhere: managed databases signed into with a cloud account instead of a password,
Parquet and table formats in object storage, and warehouses such as Snowflake and BigQuery that
answer over HTTPS and bill by the work a query does. This plan brings those in, starting with
what costs least: the engines dbdesk already speaks, hosted in the cloud.

## What changes when the data is in the cloud

Each warehouse is a new engine, built like any other (see [Adding a database](adding-a-database.md)).
What is new is shared by all of them, and is built once:

- **Signing in with a cloud account.** Managed databases accept short-lived tokens from AWS IAM,
  Google Cloud IAM or Microsoft Entra ID instead of a stored password, and warehouses accept
  only tokens, key pairs or OAuth. dbdesk should reuse the sign-in the user already has, never
  store a cloud secret itself, and refresh tokens before they expire.
- **Queries that cost money.** BigQuery bills by bytes scanned, Snowflake and Databricks by
  warehouse time, Athena by bytes scanned. Things dbdesk does without asking on a local
  database (counting a table's rows, **Open data**, completion's catalog reads, W5's
  **Export every row**) can cost real money there. The user needs to see the cost before
  running, a limit per data source, and defaults that never scan a whole table unasked.
- **Queries that run as jobs.** Warehouses run a statement as a job: submit, poll, page through
  the results, cancel by the job's id. The engine's cursor and cancel model needs that shape,
  including results that arrive in many partitions.
- **Catalogs too large to load at once.** A BigQuery organisation or a Unity Catalog can have
  thousands of projects, datasets or schemas. The explorer must load lazily, page, and search on
  the server instead of walking everything.
- **No container to test against.** Snowflake, BigQuery and Databricks have no official local
  server, so CI can't run the conformance suite against them the way it does for the engines in
  `docker-compose.yml`.

## Which services, and in what order

1. **Managed versions of the engines dbdesk already has.** Amazon RDS and Aurora, Google Cloud
   SQL, Azure Database for PostgreSQL and MySQL, Azure SQL, Neon, Supabase, ClickHouse Cloud
   and Amazon Redshift (which speaks PostgreSQL's protocol with its own catalog). They
   need no new protocol, only cloud sign-in and fixes for what each catalog does differently.
   The cheapest step, and it reaches the most users.
2. **Object storage through DuckDB.** DuckDB, already compiled into dbdesk, reads Parquet, CSV
   and JSON from S3, Google Cloud Storage and Azure Blob Storage, and Iceberg and Delta tables,
   through its own extensions. That is a data lake console with no new driver, plus MotherDuck
   (hosted DuckDB) through the same engine.
3. **Snowflake.** The most used cloud warehouse among the tools dbdesk is compared with.
   Its SQL API is HTTPS with JSON or Arrow results, which fits the ClickHouse driver's shape.
4. **BigQuery.** Google's warehouse, with a REST jobs API, GoogleSQL, and a dry run that says
   how many bytes a query will scan before it runs.
5. **Databricks SQL.** A SQL warehouse over Unity Catalog's three-level names, through the
   Statement Execution API (HTTPS, like Snowflake's).
6. **Trino and Athena.** Trino's HTTP protocol (submit, then follow `nextUri`) covers Trino,
   Starburst and Presto; Athena speaks Trino's dialect through AWS's API, with results written
   to S3.

The order between Snowflake, BigQuery and Databricks is a proposal; it is checked against
current usage numbers, as [More databases](databases.md) did for its engines, when the track
starts.

Left out on purpose:

- **Hadoop-era engines** (Hive through Thrift, Impala, Spark Thrift Server). Fewer new users,
  Thrift and SASL protocols, and the same data usually reachable through Trino or Databricks.
- **NoSQL cloud stores** (DynamoDB, Cosmos DB, Bigtable, Firestore), for the reason
  [More databases](databases.md) leaves out Redis and MongoDB: their data doesn't fit a SQL
  console with a grid of results. DynamoDB's PartiQL could be reconsidered later.
- **Running or managing cloud resources** (creating instances, scaling warehouses, IAM policy).
  dbdesk queries data; the cloud consoles and infrastructure-as-code tools manage it.

## Milestones

### C0 — Cloud sign-in, cost guards and jobs

The groundwork every later milestone needs.

- **Credential providers** in a new `dbdesk-cloud` crate, chosen per data source:
  - AWS: an IAM authentication token for RDS and Aurora (PostgreSQL and MySQL), from the
    user's AWS profile (including SSO);
  - Google Cloud: an access token from Application Default Credentials, for Cloud SQL's IAM
    database authentication;
  - Microsoft Entra ID: an access token for Azure SQL and Azure Database for PostgreSQL and
    MySQL, which also completes SQL Server's planned Entra ID sign-in (see
    [More databases](databases.md#decisions)).
  Tokens are fetched when connecting and refreshed before they expire; none is stored,
  logged or written to the history.
- **Cost guards**, as a per-data-source setting next to the safety level (W2): a limit (bytes
  scanned, or a time budget) above which a statement asks first, and quiet defaults for the
  explorer and the grid, so that no automatic action scans a whole table.
- **Jobs in the engine:** a cursor that submits, polls and pages, with cancel by job id, and a
  status line showing what the job has done so far (bytes, rows, elapsed time). ClickHouse's
  `query_id` cancel is the first user.
- **Lazy, searchable catalogs** in the explorer, for catalogs with thousands of children.

**Exit:** an RDS PostgreSQL database connects with an AWS profile and no password, and keeps
working past the token's 15-minute lifetime.

### C1 — Managed databases

- Try each managed service in [Which services](#which-services-and-in-what-order), item 1, and
  fix what its catalog or defaults do differently (Redshift's system tables, Azure SQL's gateway
  redirect, Neon's and Supabase's poolers, ClickHouse Cloud's TLS).
- The data source dialog offers each as a preset: the right driver, port, TLS mode and sign-in.
- **Exit:** each service on the list connects, browses and runs the conformance suite where an
  account is available; a short page in the manual per provider.

### C2 — Object storage and table formats (DuckDB)

- An **object storage** data source (or a DuckDB option): S3, Google Cloud Storage or Azure Blob
  Storage, with C0's credentials passed to DuckDB as secrets, and the bucket browsable in the
  explorer.
- Iceberg and Delta tables, through DuckDB's extensions.
- MotherDuck, through DuckDB's `md:` databases.
- **Exit:** query `s3://bucket/path/*.parquet` and an Iceberg table from a MinIO container in CI
  with no AWS account.

### C3 — Snowflake

- A driver over Snowflake's SQL API: key-pair (JWT) and OAuth sign-in, asynchronous statements,
  partitioned results, cancel.
- The Snowflake dialect, catalog (databases, schemas, tables, views, stages), and the warehouse
  and role per console.

### C4 — BigQuery

- A driver over the REST jobs API: projects, datasets and tables in the explorer, GoogleSQL,
  a dry run before every statement that shows the bytes it will scan, and
  `maximumBytesBilled` from C0's limit.

### C5 — Databricks SQL

- A driver over the Statement Execution API, with Unity Catalog's catalog, schema and table
  levels, and a personal access token or OAuth.

### C6 — Trino and Athena

- A driver over Trino's HTTP protocol, with catalogs as the top level; then Athena, through
  AWS's API with C0's credentials and its workgroup's output location.
- **Exit:** the conformance suite against Trino's container, in a job that runs nightly if the
  container is too large for every pull request.

## Decisions

Proposed defaults; each is confirmed when its milestone starts.

| Question | Proposed | Consequence |
| --- | --- | --- |
| How dbdesk gets cloud credentials | **From the provider's own sign-in on the machine first:** AWS profiles and SSO cache, Google Application Default Credentials, Azure CLI's sign-in, read or invoked (`aws`, `gcloud`, `az`) without a shell. A browser OAuth flow later, for warehouses that need it. | Uses the login people already have, and no cloud secret ever lives in dbdesk's files. Users without the CLIs need them installed, or a key file. |
| Cloud SDKs | **No full SDKs.** Small HTTP clients on the hyper and rustls stack the ClickHouse driver already uses, plus request signing (AWS SigV4) where needed. | Keeps CI build times and download size close to today's; each new API call is a little more code to write and test. |
| Warehouse protocols | **Their HTTPS APIs** (Snowflake SQL API, BigQuery REST, Databricks Statement Execution, Trino HTTP), not ODBC or JDBC. | Pure Rust, no drivers to install, works through proxies; some services' fastest result paths (BigQuery's Storage Read API, Arrow Flight) come later if needed. |
| Testing services with no container | **Recorded HTTP exchanges replayed by a fake server in CI**, plus live tests run by hand (or nightly) with credentials kept in repository secrets. | Every pull request stays fast and needs no cloud account; the live tests catch API drift. |
| DuckDB extensions | **Downloaded by DuckDB on first use**, signed by DuckDB, rather than compiled in. | Downloads stay the same size; the first use of S3 or Iceberg needs network access. |
| What runs without asking | **Nothing that scans a whole table** on a data source with a cost limit: no row counts, **Open data** with a `LIMIT`, export and import asking first. | A few conveniences are slower to reach on warehouses, in exchange for no surprise bills. |

## Risks

- **Surprise bills.** A missed automatic query on a large table costs the user money. Every
  automatic statement goes through one place that checks the data source's cost guard, with a
  test per engine.
- **APIs that change.** Cloud APIs version and deprecate on their own schedules. Recorded
  exchanges pin the behaviour tested; the live tests show when they drift.
- **CI time.** New containers (MinIO, Trino) add to the Linux job; anything over a minute or two
  moves to a nightly job, as [More databases](databases.md#risks) plans for large engines.
- **Accounts to test with.** Some services need a paid account to try at all. A milestone ships
  only what was tried against the real service, and says what wasn't.
