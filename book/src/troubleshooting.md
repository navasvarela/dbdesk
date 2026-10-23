# Troubleshooting

## Where to look first

The **log console** at the bottom of the window shows what dbdesk did and what went wrong, with
the newest entries at the bottom. The drop-down on its left picks how much detail to show
(Error, Warn, Info, Debug, Trace); the search box filters entries by text; the bin icon clears
the list. It keeps the latest 10,000 entries.

The same entries, from dbdesk itself and warnings from the libraries it uses, go to a log file
per day: see [Where dbdesk keeps its files](files.md). Attach the day's file when you report a
problem; it never contains passwords.

## The server requires TLS/SSL

dbdesk doesn't support encrypted PostgreSQL connections yet: it always connects without TLS.
A server that only accepts encrypted connections (as many hosted databases do) refuses, and the
error usually mentions SSL or `pg_hba.conf`, for example *no pg_hba.conf entry for host …, SSL
off*.

Until TLS support arrives, dbdesk can only reach servers that accept unencrypted connections
from where you are, such as a database in a local container. Don't turn off encryption on a
server just for dbdesk.

## The keychain keeps asking

On macOS, the system asks whether dbdesk may use the password it saved in your keychain the
first time dbdesk reads it. **Always Allow** stops the questions, but macOS ties that answer to
the exact program. Because you build dbdesk yourself, every rebuild is a new program to macOS,
and it asks again.

dbdesk reads the password each time it opens a connection: when the explorer connects, when a
console runs its first statement, and when you test a connection without typing the password.
If you deny access, the connection fails with *could not read the saved password*.

To see or delete the saved passwords, open **Keychain Access** and search for `dbdesk`. Each
entry's account is the ID of a data source in `data_sources.toml`.

On Linux, dbdesk needs a running Secret Service (GNOME Keyring or KWallet) to save passwords.
Without one, saving a PostgreSQL data source with a password fails with *could not store the
password*.

## A connection takes long, then fails

dbdesk gives up on a connection attempt after 15 seconds and reports *could not connect within
15s*. Check the host and port, and that nothing (a VPN, a firewall) is in the way. Test
connection in the data source dialog is the quickest way to try again.

## "unable to open database file" for SQLite

The file in the data source's **File** field doesn't exist, or dbdesk can't read it. Connecting
never creates a SQLite file, so check the path, or pick the file with **Browse…** in the data
source dialog. To start a new database, use **New…** there (see
[Data sources](data-sources.md#adding-one)).

## "Can't read data sources"

`data_sources.toml` exists but isn't valid, usually after editing it by hand. dbdesk shows the
error in the explorer and refuses to change the file until it is fixed, so nothing is lost.
Correct the file (the error says which line), then restart dbdesk.

## The status bar says the engine stopped

The engine is the part of dbdesk that talks to databases. If it stops, nothing can connect or
run, and the reason is in the log console and the log file. Restart dbdesk; if it happens
again, please report it with the day's log file.

## "keymap.toml: N problems" in the status bar

Some lines of `keymap.toml` couldn't be used: an unknown action, a shortcut that can't be read,
or a shortcut two actions share. Click the message to open **View → Keymap…**, which lists each
problem; fix the file and click **Reload**. Everything else in the file still applies.

## The panels are in a mess

**View → Reset layout** puts every panel back where it started. If you closed the Welcome tab,
this brings it back too.
