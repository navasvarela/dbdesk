# Connecting an AI assistant

dbdesk can serve a [project](explorer.md#projects)'s databases to an AI assistant or coding
agent through the [Model Context Protocol](https://modelcontextprotocol.io) (MCP). The
assistant can then list the project's data sources, browse their tables, describe them and run
queries, using the data sources, drivers and saved passwords dbdesk already has.

For now, an assistant can **only read**. Every statement runs in the database's own read-only
mode and is rolled back, and anything that would change data or the schema is refused before it
is sent.

> **Data leaves your machine.** What a query returns goes to the assistant, and from there to
> the company that runs its model. Only allow projects whose data you are happy to share that
> way, and keep sensitive databases in projects of their own.

## 1. Allow a project

A new project is hidden from MCP clients. To allow one, quit dbdesk, open `projects.toml` (see
[Where dbdesk keeps its files](files.md#projectstoml)) and set `mcp_enabled = true` on it:

```toml
[[project]]
id = "5b0c7d1e-2f3a-4b5c-8d6e-7f8091a2b3c4"
name = "Analytics"
slug = "analytics"
mcp_enabled = true
```

Note the project's `slug`; hovering over the project in the explorer shows it too. A later
version will have a project settings window for this.

## 2. Add dbdesk to the client

The client starts dbdesk itself, as `dbdesk mcp --project <slug>`, and talks to it over standard
input and output; dbdesk doesn't need to be open. Point the client at the dbdesk program:

| System | Program |
| --- | --- |
| macOS | `/Applications/dbdesk.app/Contents/MacOS/dbdesk` |
| Linux, `.deb` | `/usr/bin/dbdesk` |
| Linux, AppImage | wherever you keep the `.AppImage` file |
| Built from source | the `dbdesk` that `cargo install` put in `~/.cargo/bin` |

Most clients take a JSON entry like this one, in their own settings file:

```json
{
  "mcpServers": {
    "dbdesk-analytics": {
      "command": "/Applications/dbdesk.app/Contents/MacOS/dbdesk",
      "args": ["mcp", "--project", "analytics"]
    }
  }
}
```

Clients with a command line add the same thing in one go, for example Claude Code:

```bash
claude mcp add dbdesk-analytics -- /Applications/dbdesk.app/Contents/MacOS/dbdesk mcp --project analytics
```

One entry serves one project. For several projects, add one entry per project, each with its
own `--project`.

On macOS, the first time the server reads a saved password, the system asks whether `dbdesk`
may use it, as it does for the app; choose **Always Allow**. Until dbdesk's builds are signed,
it asks again after each update.

## What the assistant can do

| Tool | What it does |
| --- | --- |
| `list_data_sources` | The project's data sources: name, engine, colour, and the server version once connected. Also the limits below. |
| `list_objects` | One level of a data source's catalog, as the explorer shows it. Each object comes with the `path` that lists its children. |
| `describe_table` | A table's columns with their types, its primary key, indexes and constraints. |
| `get_definition` | The source of a view, function, trigger or index. |
| `query` | Runs one statement that only reads, and returns its rows as text. |
| `explain` | Shows how the database would run one read statement, without running it. |

Every tool is marked read-only, so clients that ask before running tools can let these run
freely.

Each call is limited:

- **At most 500 rows** per query. The reply says `"truncated": true` when there were more.
- **30 seconds** per statement; a slower one is cancelled.

Both can be changed for every project in the `[mcp_defaults]` table of `projects.toml`
(`max_rows`, `statement_timeout = "1m"`), or for one project in a `[project.mcp]` table under
it.

## How reading is enforced

Two checks, so one catching what the other misses:

1. **dbdesk reads the statement first.** Anything other than a read is refused before it reaches
   the database: `INSERT`, `UPDATE`, `DELETE` (even inside a `WITH`), `SELECT … INTO`, `SELECT
   … FOR UPDATE`, DDL, `SET`, `COPY`, transaction control and so on. A statement dbdesk can't
   parse is refused too, since what it would do is unknown, and so is more than one statement
   at a time.
2. **The database refuses writes on its own.** Every call runs inside a read-only transaction
   that is then rolled back: `BEGIN READ ONLY` on PostgreSQL, `START TRANSACTION READ ONLY` on
   MySQL and MariaDB, and SQLite files are opened read-only. This is what stops a `SELECT` that
   calls a function which writes, since dbdesk can't see inside the function.

SQL Server has no read-only transaction, so its data sources are left out of what the server
offers for now; the log says so when the server starts.

For airtight limits, also give the data source a database user that can only read.

## What dbdesk records

- **The log.** Every call is a line with the project, data source, tool and the kind of
  statement; refusals are logged with their reason. The server logs to its own daily files,
  `dbdesk-mcp.YYYY-MM-DD.log`, next to the app's (see
  [Where dbdesk keeps its files](files.md)), and to standard error, which most clients keep
  in their own logs.
- **The query history.** Every statement the server sends to a database is in the history,
  marked as coming from MCP, beside your own.

## When it doesn't work

The client's log for the server shows what dbdesk said when it stopped:

- **"there is no project with the slug …"**: the `--project` doesn't match any project's `slug`;
  the message lists the slugs there are.
- **"… is not allowed for MCP clients"**: set `mcp_enabled = true` on it, as in step 1.
- **"unknown argument"**: the client's `args` have a typo; each one is checked.

The server reads `projects.toml` and `data_sources.toml` when it starts, so after changing
either, restart the server (most clients have a button for it, or restart the client).
