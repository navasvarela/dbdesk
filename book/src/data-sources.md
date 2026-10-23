# Data sources

A *data source* is a saved connection: which database to connect to and how. dbdesk keeps the
list in a file (see [Where dbdesk keeps its files](files.md)) and passwords in your system's
keychain, never in that file.

## Adding one

Click **+** at the top of the explorer (or **New data source** on the Welcome tab) and choose
**PostgreSQL** or **SQLite**.

![The New PostgreSQL data source dialog](images/data-source-dialog.png)

Both kinds have:

- **Name** — how it appears in the explorer. Leave it empty and dbdesk names it for you:
  `database@host` for PostgreSQL, the file name for SQLite.
- **Colour** — optional. The colour tints the data source's row in the explorer, the tabs of its
  consoles and a band along the top of each console. Giving production a red and your local copy
  nothing makes it hard to run something in the wrong place.

**PostgreSQL** adds:

| Field | Default | Notes |
| --- | --- | --- |
| Host | `localhost` | Name or IP address of the server. |
| Port | `5432` | A number from 1 to 65535. |
| Database | `postgres` | The database to connect to. The explorer lists the server's other databases too. |
| User | `postgres` | |
| Password | none | Stored in the keychain when you save. See below. |

dbdesk connects to PostgreSQL **without TLS** for now, so a server that requires an encrypted
connection will refuse it. See [Troubleshooting](troubleshooting.md#the-server-requires-tlsssl).

**SQLite** adds:

- **File** — the full path to the database file. The file must already exist: dbdesk never
  creates one, so a typo is an error rather than a new, empty database.
- **Read-only** — opens the file read-only. Statements that would change it fail with SQLite's
  error.

## Testing the connection

**Test connection** connects with what is in the form right now, checks the connection works,
and disconnects. The result appears under the form ("Connected to PostgreSQL 17.6", or the
error) and in the log console. Nothing is saved by testing, not even the password.

## Saving

**Save** checks the form and saves the data source. Required fields left empty, or a port that
isn't a number, show a message under the form instead. The dialog stays open until the save has
gone through; if it fails, the reason is shown in the dialog.

**Cancel**, **Esc** or a click outside the dialog closes it without saving.

## Passwords and the keychain

PostgreSQL passwords are stored in your operating system's credential store: the Keychain on
macOS, Credential Manager on Windows, the Secret Service (GNOME Keyring, KWallet) on Linux.
Each entry has the service name `dbdesk` and, as its account, the data source's ID, a long
string that never changes even if you rename the data source.

- **When you save**, a typed password is written to the keychain. When you edit a data source
  the password field is empty and says *unchanged*: leave it empty to keep the saved password,
  or type a new one to replace it.
- **When dbdesk connects** — the explorer, each console, and Test connection with an empty
  password field — it reads the password from the keychain. On macOS the system may ask whether
  to let dbdesk use it; see [Troubleshooting](troubleshooting.md#the-keychain-keeps-asking).
- **When you delete** a data source, its keychain entry is removed too.

SQLite data sources have no password, so dbdesk never asks the keychain for them.

## Editing, connecting and deleting

Right-click a data source in the explorer for:

| Menu item | What it does |
| --- | --- |
| **Connect** / **Disconnect** | Opens or closes the explorer's connection. Expanding a data source also connects it. |
| **Refresh** | Reloads everything under the data source (while connected). |
| **New query console** | Opens a console on this data source. See [Query consoles](consoles.md). |
| **Edit…** | Opens the dialog with the saved settings. |
| **Delete…** | Asks for confirmation, then removes the data source, closes its consoles and deletes its saved password. |

Hover over a data source to see its driver, where it connects to and its connection state. The
dot after its name shows the state of the explorer's connection: amber while connecting, green
when connected, red when the last attempt failed, and no dot when disconnected. A failed
connection's error is in the tooltip and in the log console.
