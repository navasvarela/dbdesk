# Data sources

A *data source* is a saved connection: which database to connect to and how. DbRover keeps the
list in a file (see [Where DbRover keeps its files](files.md)) and passwords in your system's
keychain, never in that file.

## Adding one

Click **+** at the top of the explorer (or **New data source** on the Welcome tab) and choose
**PostgreSQL**, **SQLite**, **MySQL / MariaDB**, **SQL Server**, **DuckDB** or **ClickHouse**.

![The New PostgreSQL data source dialog](images/data-source-dialog.png)

Every kind has:

- **Name** — how it appears in the explorer. Leave it empty and DbRover names it for you:
  `database@host` for a server, the file name for SQLite.
- **Colour** — optional. The colour tints the data source's row in the explorer, the tabs of its
  consoles and a band along the top of each console. Giving production a red and your local copy
  nothing makes it hard to run something in the wrong place.
- **Safety** — how careful DbRover is with the data. See [Safety levels](#safety-levels).

**PostgreSQL** adds:

| Field | Default | Notes |
| --- | --- | --- |
| Host | `localhost` | Name or IP address of the server. |
| Port | `5432` | A number from 1 to 65535. |
| Database | `postgres` | The database to connect to. The explorer lists the server's other databases too. |
| User | `postgres` | |
| TLS | `prefer` | Whether to encrypt the connection; see below. |
| SSH | off | Connect through an SSH server; see [SSH tunnels](#ssh-tunnels). |
| Password | none | Stored in the keychain when you save. See below. |
| MCP login | none | Another user, only for AI assistants; see [The MCP login](mcp.md#the-mcp-login). |
| MCP password | none | Shown once there is an MCP login. Stored in the keychain apart from the password. |

### TLS

**TLS** encrypts the connection, which hosted databases usually require. The modes are the
ones PostgreSQL's own tools call `sslmode`, so a provider's connection guide reads the same:

| Mode | Encrypts | Checks the server's certificate |
| --- | --- | --- |
| `disable` | Never | — |
| `prefer` | If the server offers it, otherwise connects unencrypted | No |
| `require` | Always; fails if the server can't | No |
| `verify-full` | Always | Yes: signed by an authority you trust, and naming the host you typed |

`prefer` and `require` keep anyone on the network from reading what you send, but not from
pretending to be the server. Use `verify-full` for anything that matters.

With `verify-full`, a **CA file** row appears. Leave it empty for servers whose certificate is
signed by a public authority (the same ones web browsers trust). For a private or self-signed
certificate, give the authority's certificate as a PEM file, typed or picked with **Browse…**:
for example the bundle your cloud provider publishes for its managed databases. The **Host**
must be a name (or IP address) the certificate lists.

DbRover encrypts with [rustls](https://github.com/rustls/rustls), so there is no OpenSSL to
install. Client certificates aren't supported yet.

**MySQL / MariaDB** has the same fields as PostgreSQL, with these defaults:

![The New MySQL / MariaDB data source dialog](images/mysql-data-source-dialog.png)

| Field | Default | Notes |
| --- | --- | --- |
| Host | `localhost` | Name or IP address of the server. |
| Port | `3306` | |
| Database | `mysql` | The database unqualified names refer to, as `USE` would set. The explorer lists every database you may see. |
| User | `root` | |
| TLS | `prefer` | As for PostgreSQL. MySQL 8 and MariaDB 11.4 encrypt by default, with a self-signed certificate unless the server is given one, so `verify-full` needs that server's CA file. |
| Password | none | Stored in the keychain when you save. |
| MCP login | none | Another user, only for AI assistants; see [The MCP login](mcp.md#the-mcp-login). |
| MCP password | none | Shown once there is an MCP login. Stored in the keychain apart from the password. |

One data source type serves both: DbRover tells MySQL and MariaDB apart once it connects, and
**Test connection** says which it found ("Connected to MariaDB 11.4.4").

**SQL Server** has the same fields too, for SQL Server and Azure SQL, with these defaults. Its MCP
login is required for [MCP clients](mcp.md) to use it at all, since SQL Server has no read-only
mode: see [How the permissions are enforced](mcp.md#how-the-permissions-are-enforced).

![The New SQL Server data source dialog](images/sqlserver-data-source-dialog.png)

| Field | Default | Notes |
| --- | --- | --- |
| Host | `localhost` | Name or IP address of the server. For Azure SQL, the server's name, such as `myserver.database.windows.net`. |
| Port | `1433` | Named instances aren't looked up: give the instance's port. |
| Database | `master` | The database the console works in. The explorer lists every database the login may open. |
| User | `sa` | A SQL Server login. Windows and Microsoft Entra ID sign-in aren't supported yet. |
| TLS | `prefer` | SQL Server always offers encryption, with a certificate of its own making unless it is given one, so `prefer` and `require` encrypt without checking it, and `verify-full` needs that server's CA file (or, for Azure SQL, nothing: its certificate is publicly signed). |
| Password | none | Stored in the keychain when you save. |

**Test connection** says which server it reached ("Connected to SQL Server 16.0.4305.5"; 16 is
SQL Server 2022).

**ClickHouse** has the same fields, with these defaults:

![The New ClickHouse data source dialog](images/clickhouse-data-source-dialog.png)

| Field | Default | Notes |
| --- | --- | --- |
| Host | `localhost` | Name or IP address of the server. For ClickHouse Cloud, the service's host name. |
| Port | `8123` | ClickHouse's HTTP port: DbRover speaks HTTP to ClickHouse, not its native protocol on 9000. |
| Database | `default` | The database unqualified names refer to, as `USE` would set. The explorer lists every database you may see. |
| User | `default` | |
| TLS | `disable` | `disable`, `require` or `verify-full`; see below. |
| Password | none | Stored in the keychain when you save. |
| MCP login | none | Required for AI assistants, since DbRover doesn't use ClickHouse's read-only setting yet; see [The MCP login](mcp.md#the-mcp-login). |
| MCP password | none | Shown once there is an MCP login. Stored in the keychain apart from the password. |

ClickHouse serves HTTPS on a port of its own (8443) rather than offering encryption on the HTTP
one, so there is no `prefer`: choosing `require` or `verify-full` moves the port to `8443`, and
`disable` moves it back, unless you typed a port of your own. ClickHouse Cloud only accepts
HTTPS, with a publicly signed certificate, so `verify-full` with no CA file works there.

**Test connection** says which server it reached ("Connected to ClickHouse 25.8.33.6").

**SQLite** adds:

![The New SQLite data source dialog](images/sqlite-data-source-dialog.png)

- **File** — the full path to the database file. Type or paste it, or use the buttons under the
  field:
  - **Browse…** opens your system's file dialog to pick an existing database. It shows every
    file, because SQLite databases end in `.db`, `.sqlite`, `.sqlite3` or nothing at all.
  - **New…** opens your system's Save dialog and creates an empty database file where you
    choose, then fills in the path and clears **Read-only**. The message under the form says
    whether it worked; click **Save** to keep the data source. DbRover never overwrites an
    existing file, even if the Save dialog asked whether to replace it and you said yes: pick
    another name instead.

  Connecting never creates a file, so a typo in the path is an error rather than a new, empty
  database. **New…** is the only way DbRover creates one.
- **Read-only** — opens the file read-only. Statements that would change it fail with SQLite's
  error.

**DuckDB** has two kinds of database, chosen under **Database**:

![The New DuckDB data source dialog](images/duckdb-data-source-dialog.png)

- **File** — a DuckDB database file (`.duckdb`), with **File**, **Browse…**, **New…** and
  **Read-only** as for SQLite. Only one program at a time may have a DuckDB file open for
  writing, so connecting fails while another program (the `duckdb` command, say) has it open.
- **In memory** — an empty database that lives as long as the connection, for querying files
  rather than storing data: DuckDB reads CSV, Parquet and JSON files directly. Each console has
  its own, and everything created in it is gone when it disconnects.

DuckDB is part of DbRover, so there is nothing to install. **Test connection** says which
version it is ("Connected to DuckDB 1.5.6").

### SSH tunnels

Many databases only accept connections from inside their own network, through a *jump host*
(or *bastion*) you reach with SSH. Tick **Connect through an SSH server** and DbRover opens an
SSH tunnel for you before connecting: no `ssh -L` to run first, and nothing to install. Every
server engine has it: PostgreSQL, MySQL and MariaDB, SQL Server and ClickHouse.

![A PostgreSQL data source behind a bastion, signing in with a key file](images/ssh-data-source-dialog.png)

| Field | Default | Notes |
| --- | --- | --- |
| SSH host | none | The SSH server's name or IP address. |
| SSH port | `22` | |
| SSH user | none | Whom to sign in to the SSH server as. |
| SSH login | SSH agent | How to sign in; see below. |
| Key file | none | For **Key file**: your private key, in OpenSSH's format. **Browse…** opens `~/.ssh`. |
| Passphrase | none | For an encrypted key. Stored in the keychain when you save. |
| SSH password | none | For **Password**. Stored in the keychain when you save. |

**Host**, **Port** and **Database** above stay the database's own, as the SSH server sees it:
often a private address such as `10.0.4.12`, or `localhost` when the database runs on the SSH
server itself. TLS still checks the database's certificate against that host, so `verify-full`
works through a tunnel too.

The ways to sign in:

- **SSH agent** uses the keys your running agent holds, as `ssh` does: the agent `SSH_AUTH_SOCK`
  names on macOS and Linux, and Windows' OpenSSH agent or Pageant on Windows. Nothing is stored.
- **Key file** signs in with a private key. An encrypted key's passphrase is asked for the first
  time it is needed, like a password, and kept in the keychain; an unencrypted key never touches
  the keychain.
- **Password** signs in with the SSH user's password, kept in the keychain.

The SSH settings take [placeholders](#placeholders-variable) like the server's, and they travel
in an [export](sharing.md); the passphrase and password stay in each machine's keychain.

**The SSH server's host key** is checked against `~/.ssh/known_hosts`, the file `ssh` itself
keeps, so a server you already reach with `ssh` is trusted by DbRover too. The first time DbRover
meets one it doesn't know, it shows the key's SHA-256 fingerprint and asks. Compare it with the
fingerprint the server's administrator gives you (`ssh-keygen -lf` on the server's host key):
**Trust and connect** adds it to `known_hosts` and connects. A server whose key has changed
since is refused, with the line of `known_hosts` to remove if the change was expected; see
[Troubleshooting](troubleshooting.md#ssh-tunnels).

![Connecting to an SSH server for the first time shows its fingerprint](images/host-key-prompt.png)

A data source's consoles and its explorer share one tunnel, which closes with the last of them.

## Testing the connection

**Test connection** connects with what is in the form right now, checks the connection works,
and disconnects. The result appears under the form ("Connected to PostgreSQL 17.6", or the
error) and in the log console. Nothing is saved by testing, not even the password.

## Saving

**Save** checks the form and saves the data source. Required fields left empty, or a port that
isn't a number, show a message under the form instead. The dialog stays open until the save has
gone through; if it fails, the reason is shown in the dialog.

**Cancel**, **Esc** or a click outside the dialog closes it without saving.

## Safety levels

A colour only marks a data source. Its **safety level** decides what its consoles may change:

| Level | What happens |
| --- | --- |
| **Normal** | Changes run when asked. Only an `UPDATE` or `DELETE` without `WHERE` asks first (see [Query consoles](consoles.md#statements-that-change-every-row)). |
| **Careful** | Every change asks first, naming the data source: statements that change data or the schema, and edits in the grid. New consoles start in **Manual commit** mode. |
| **Read-only** | Changes are refused. DbRover reads each statement before sending it and refuses anything but reading, then runs it in the database's own read-only mode, which also stops a write hidden in a function. The grid can't be edited. |

![Red suggests Careful in the data source dialog](images/safety-level.png)

Picking red for a data source whose level is Normal suggests **Careful**, since red usually marks
production; **Make it careful** takes the suggestion. The level is its own setting, so a red data
source can stay Normal, and a careful one can be any colour.

Read-only uses the same checks as an AI assistant's read-only projects (see
[Connecting an AI assistant](mcp.md)):

- **PostgreSQL** runs each statement in `BEGIN READ ONLY`, **MySQL and MariaDB** in
  `START TRANSACTION READ ONLY`, rolled back once its rows are read. **SQLite** opens the file
  read-only.
- **SQL Server**, **DuckDB** and **ClickHouse** have no read-only mode DbRover uses yet:
  statements are still checked, but a write inside a procedure or function isn't stopped. The
  dialog says so. On SQL Server and ClickHouse, sign in as a user with read-only grants to have
  the database refuse it; [The MCP login](mcp.md#the-mcp-login) shows how to make one. (A
  DuckDB file can also be opened with its own **Read-only** box ticked.)
- Anything that isn't a plain read is refused, including `SET`, `BEGIN` and statements DbRover
  can't parse, since what they do can't be told.

## Placeholders: `${VARIABLE}`

Host, port, database, user, a SQLite or DuckDB file and a CA file may hold `${NAME}`
placeholders, which DbRover replaces with the environment variable `NAME` each time it connects.
One shared list of data sources then works on machines whose settings differ: each machine sets
`PGHOST`, say, and the data source's host is `${PGHOST}`. A placeholder can be part of a value
(`db-${ENV}.internal`), and a path starting with `~` is in your home folder.

![Placeholders in the host, port and user, with what each resolves to](images/data-source-placeholders.png)

Under each field with a placeholder, the dialog shows what it resolves to now, or which variable
isn't set. Connecting with an unset variable stops with an error that names the setting:
*could not resolve the host: ${PGHOST} is not set*.

Variables come from the environment DbRover was started in. On macOS, an app opened from the
Finder or the Dock doesn't see what your shell profile sets; start it from a terminal
(`open -a dbrover`, after exporting the variables), or set them with `launchctl setenv`.

## Passwords and the keychain

PostgreSQL, MySQL and SQL Server passwords are stored in your operating system's credential
store: the Keychain on
macOS, Credential Manager on Windows, the Secret Service (GNOME Keyring, KWallet) on Linux.
Each entry has the service name `dbrover` and, as its account, the data source's ID, a long
string that never changes even if you rename the data source.

- **When you save**, a typed password is written to the keychain. When you edit a data source
  the password field is empty and says *unchanged*: leave it empty to keep the saved password,
  or type a new one to replace it.
- **When DbRover connects** — the explorer, each console, and Test connection with an empty
  password field — it reads the password from the keychain. On macOS the system may ask whether
  to let DbRover use it; see [Troubleshooting](troubleshooting.md#the-keychain-keeps-asking).
- **When there is none**, or the keychain can't be read (locked, declined, or missing on this
  system), connecting asks for the password. **Save to keychain** is ticked, so you are asked
  once. Whatever you type is also kept in memory until DbRover quits, so the data source still
  connects if the keychain refuses it or you untick the box. A server that needs no password
  (local *trust* login, say) works with the field left empty.
- **When you delete** a data source, its keychain entry is removed too.
- **An MCP login's password** has an entry of its own, with the data source's ID followed by
  `/mcp` as its account. Only `dbrover mcp` reads it.
- **An SSH password or key passphrase** has one too, with `/ssh` after the ID. It is asked for,
  and saved, the same way as the database password.

![Connecting asks for the password a data source has none saved for](images/password-prompt.png)

SQLite data sources have no password, so DbRover never asks the keychain for them.

## Editing, connecting and deleting

Right-click a data source in the explorer for:

| Menu item | What it does |
| --- | --- |
| **Connect** / **Disconnect** | Opens or closes the explorer's connection. Expanding a data source also connects it. |
| **Refresh** | Reloads everything under the data source (while connected). |
| **New query console** | Opens a console on this data source. See [Query consoles](consoles.md). |
| **Open saved console** | Reopens a console you closed, by its file name. Shown when there are any. |
| **New › Schema…** | Creates a database, on MySQL, MariaDB and ClickHouse, whose schemas are databases. See [Views and schemas](views-schemas.md#new-schema). |
| **Edit…** | Opens the dialog with the saved settings. |
| **Move to project ›** | Puts the data source in a project, in **No project**, or in a new project. See [Projects](explorer.md#projects). |
| **Delete…** | Asks for confirmation, then removes the data source and deletes its saved password. |

Hover over a data source to see its driver, where it connects to and its connection state. The
dot after its name shows the state of the explorer's connection: amber while connecting, green
when connected, red when the last attempt failed, and no dot when disconnected. A failed
connection's error is in the tooltip and in the log console.
