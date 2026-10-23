# Creating and changing database objects

Status: **planned**, nothing built yet. Proposed on 2026-10-23.

Today the explorer only reads: it browses the catalog, opens a table's data and shows
definitions. Creating or changing anything means writing the SQL by hand in a console. This
plan adds helpers like those in desktop database tools (pgAdmin, DBeaver, DataGrip,
TablePlus) for the three kinds of object people manage most: **tables**, **views** and
**users**. Each one is a dialog that writes the SQL, shows it, and runs it.

## What the user sees

Right-clicking in the explorer gains a **New** submenu and object actions, depending on what
was clicked and what the database supports:

| Clicked on | New | Object actions |
| --- | --- | --- |
| A schema, or its tables folder | Table | — |
| A schema, or its views folder | View, Materialized view (PostgreSQL) | — |
| A table | — | Modify table…, Rename…, Truncate…, Drop…, Copy DDL |
| A view | — | Edit view…, Rename…, Drop…, Copy DDL |
| A materialized view | — | Refresh, Edit…, Rename…, Drop…, Copy DDL |
| The data source, or its roles folder (PostgreSQL) | User, Role | — |
| A user or role | — | Edit…, Rename…, Drop… |

Every dialog ends the same way: **the SQL it will run, shown before it runs.** The user can
**Execute** it, **Open in console** to edit it first, or **Copy** it. Nothing reaches the
database until one of the three is clicked. This keeps the helpers honest and teaches the SQL
behind them, which matches the project's learning goal.

After a change succeeds, the explorer refreshes the parent node and selects the new or changed
object. A failure shows the database's error in the dialog, which stays open to fix it.

## Shared groundwork (E0)

Everything the three kinds of object have in common, built once.

- **Object specs and a DDL builder in `dbdesk-sql`.** Plain data types describe what the user
  asked for: `TableSpec` (columns, primary key, unique and foreign keys, checks), `ViewSpec`
  and `RoleSpec`. Pure functions turn a spec into statements for a dialect, so the builder is
  tested without a database.
  - Identifiers are quoted by the dialect's rules, which DB0 gathered in one place.
  - Each change is described as an operation (`AddColumn`, `RenameTable`, `DropView`, …), so
    one dialog can produce several statements in a known order.
- **Capabilities per dialect.** What each engine can do (materialized views, roles,
  transactional DDL, which `ALTER TABLE` forms) is a table of facts next to the dialect. The
  menus and dialogs read it, instead of checking driver names. SQLite gets no users or roles,
  and its menus don't show them.
- **A "review and run" panel** reused by every dialog:
  - the generated SQL, highlighted and read-only;
  - Execute, Open in console and Copy;
  - the database's error inline when it fails.
- **An engine command for changes**, `Command::ApplyDdl { data_source, statements, sensitive }`,
  answered by `Event::DdlApplied`. It runs on the explorer's session of that data source.
  - **On PostgreSQL and SQLite**, whose DDL is transactional, the statements run inside one
    transaction, so a failure leaves nothing half done.
  - **On MySQL and SQL Server (from DB1/DB2)**, where DDL commits as it goes, the panel says
    that a failure part way leaves the earlier statements applied.
  - **Sensitive statements** (anything holding a password) are kept out of the query history
    and the log. The log records only the statement's kind, e.g. "CREATE ROLE".
- **Confirmations for destructive actions.** Drop and Truncate name the object and what goes
  with it (`CASCADE` drops dependent views). On a data source coloured red, the user types the
  object's name to confirm. The colour is how the M1 follow-up marks production.

**Exit:** a UI test builds a one-column table spec, runs it from the review panel, and sees the
table appear in the refreshed tree, on both engines.

## E1 — Tables

- **New table** dialog:
  - name and schema;
  - a grid of columns: name, type, nullable, default, and primary key;
  - tabs for unique keys, foreign keys and checks.
  - The type field offers the dialect's common types (from the dialect facts) and accepts any
    text, so arrays, domains and extension types still work.
  - A foreign key picks its target table and columns from the cached catalog, the same
    source completion uses.
- **Rename…**: `ALTER TABLE … RENAME TO …`.
- **Truncate…**, and **Drop…** with an optional `CASCADE` (PostgreSQL), both behind the
  confirmations above.
- **Copy DDL**: the `CREATE TABLE` statement for an existing table.
  - SQLite stores it, so it comes straight from the catalog.
  - PostgreSQL has no `pg_get_tabledef`, which is an open question in the explorer doc. The
    statement is rebuilt from `describe_table` plus the constraints the catalog already
    loads, through the same builder. This settles that M2 follow-up.

**Exit:**
- I create a table with a foreign key to another one, rename it, and drop it, on PostgreSQL
  and SQLite.
- A conformance check builds a table from a spec, then compares `describe_table` with the spec.

## E2 — Views

- **New view**: name, schema, and the query in the console's own editor (highlighting,
  completion, problems while typing). The dialog writes `CREATE VIEW`; it offers
  `CREATE OR REPLACE` when the name is taken and the dialect allows it.
- **New materialized view** (PostgreSQL): the same, plus "with data" or `WITH NO DATA`.
- **Edit view…**: opens the current definition, which `Show definition` already loads, in the
  dialog.
  - PostgreSQL saves it with `CREATE OR REPLACE VIEW`.
  - SQLite has no replace, so it drops and recreates the view inside one transaction.
- **Refresh** for materialized views: `REFRESH MATERIALIZED VIEW`, with `CONCURRENTLY` offered
  when the view has a unique index.
- **Rename…** and **Drop…** as for tables.

**Exit:** I create a view from a query, edit its query, and drop it, on both engines. On
PostgreSQL I also create and refresh a materialized view.

## E3 — Users and roles (PostgreSQL)

PostgreSQL has one kind of object for both: a **user** is a role that may log in. The menu
offers both words, and **New user** starts with LOGIN ticked. pgAdmin likewise uses one
"Login/Group Role" dialog for both.

- **Catalog:** a **Roles** folder at the top of a PostgreSQL data source, next to its
  databases, because roles belong to the whole server. Each role shows whether it can log in
  and whether it is a superuser. It lists the roles it belongs to. This needs two new node
  kinds, `NodeKind::Role` and `FolderKind::Roles`.
- **New user / New role** dialog:
  - name and password;
  - attributes: LOGIN, SUPERUSER, CREATEDB, CREATEROLE, INHERIT, REPLICATION, BYPASSRLS;
  - connection limit and expiry date;
  - the roles it is a member of.
- **The password never travels or gets stored in clear.**
  - The dialog hashes it on the client into a SCRAM-SHA-256 verifier, which PostgreSQL accepts
    in place of a password (`PASSWORD 'SCRAM-SHA-256$…'`). The hashing function to use is the
    one in `postgres-protocol`, which tokio-postgres already depends on.
  - The statement is marked sensitive, so it stays out of the history and the log.
  - The review panel shows the password as `••••••`.
- **Edit…**: the same dialog, producing `ALTER ROLE` for what changed, plus `GRANT`/`REVOKE`
  for changed memberships.
- **Rename…** and **Drop…**. PostgreSQL refuses to drop a role that still owns objects, and
  the error says so. The dialog then offers `REASSIGN OWNED BY … TO …`, then `DROP OWNED BY …`,
  as a separate, confirmed step.

Privileges on tables and schemas (a grants editor) are not part of E3. Each new engine adds its
own users when its driver lands:
- **MySQL** (DB1) has `'name'@'host'` accounts.
- **SQL Server** (DB2) has logins plus database users.

**Exit:**
- I create a login user with a password and a membership, log in as it from a new data
  source, change its attributes, and drop it.
- Its password appears in no history entry, log line or review panel.

## E4 — Modify table

Changing an existing table is the hardest helper, so it comes last.

- **Modify table…** opens the New table dialog on the existing table. The dialog compares what
  the user changed with what `describe_table` reported, and builds the operations from that:
  - add, drop and rename columns;
  - change a column's type, nullability and default;
  - add and drop keys and checks.
- **PostgreSQL** supports all of it with `ALTER TABLE`. A type change that needs a conversion
  gets a `USING` expression field.
- **SQLite** only renames and adds or drops simple columns in place. Anything else needs
  SQLite's documented twelve-step rebuild:
  1. Create the new table.
  2. Copy the rows into it.
  3. Drop the old table.
  4. Rename the new one.
  5. Recreate the indexes, triggers and views.

  This all runs in one transaction with foreign keys switched off. It is risky, so it ships
  last and only once the tests below pass.
- **Tests:** a round trip per operation (build, apply, `describe_table`, compare), on both
  engines. SQLite rebuilds also check that rows, indexes and triggers survive.

**Exit:** I change a column's type and add a foreign key on a table with rows, on both engines,
and nothing is lost.

## Later

- **New schema** and **Drop schema**, with the same panel.
- **Indexes:** New index… on a table, and Drop index.
- **A privileges editor** for PostgreSQL.
- **Users** for MySQL (DB1) and SQL Server (DB2).
- **Virtual views** (a saved query shown as if it were a view, never created on the
  server): a candidate once consoles can be pinned to the tree.

## Decisions

Proposed defaults; each is confirmed when its milestone starts.

| Question | Proposed | Consequence |
| --- | --- | --- |
| Run straight away, or always review first | **Always review first**, with Execute in the review panel. | One extra click per change; nothing runs unseen. |
| Where the DDL builder lives | **`dbdesk-sql`**, beside the dialects. | Pure and testable without a database or the UI. |
| Users and roles in one dialog | **One dialog**: New user is New role with LOGIN ticked. | Matches PostgreSQL's model, and pgAdmin's single Login/Group Role dialog. |
| Passwords | **Hashed into a SCRAM verifier on the client**, and kept out of the history and logs. | Needs a server with `password_encryption = scram-sha-256`, the default since PostgreSQL 14. Older servers get the password sent as typed, with a warning in the panel. |
| SQLite table rebuilds | **Last, in E4**, behind round-trip tests. | Until then, Modify table on SQLite offers only what `ALTER TABLE` does in place. Other options are disabled, with a tooltip saying why. |

## Risks

- **DDL breadth.** Every engine has many forms of `CREATE TABLE` and `ALTER TABLE`. The
  dialogs cover the common ones. Anything else goes through **Open in console**, so the user is
  never blocked.
- **Losing data in a rebuild.** SQLite's rebuild copies every row. It runs in one transaction
  and must recreate everything that pointed at the old table. E4 ships only once its tests
  show nothing is lost.
- **Dropping the wrong thing.** Typing the name for red (production) data sources,
  `CASCADE` spelled out in the confirmation, and the review panel all guard against it.
- **Passwords leaking.** The sensitive flag has to be honoured everywhere a statement is
  recorded: the history, the log and the console's error messages. A test runs a sensitive
  statement and then searches every one of them for the password.
