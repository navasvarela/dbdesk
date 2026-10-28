# Troubleshooting

## Where to look first

The **log console**, under the consoles, shows what dbdesk did and what went wrong, with
the newest entries at the bottom. The drop-down on its left picks how much detail to show
(Error, Warn, Info, Debug, Trace); the search box filters entries by text; the bin icon clears
the list. It keeps the latest 10,000 entries.

The same entries, from dbdesk itself and warnings from the libraries it uses, go to a log file
per day: see [Where dbdesk keeps its files](files.md). Attach the day's file when you report a
problem; it never contains passwords.

## TLS/SSL errors

The data source's **TLS** setting decides whether dbdesk encrypts the connection (see
[Data sources](data-sources.md#tls)). The usual errors, and what to change:

- ***pg_hba.conf rejects connection … no encryption*** (or *no pg_hba.conf entry … SSL off*):
  the server only accepts encrypted connections and **TLS** is `disable`. Pick `require` or,
  better, `verify-full`.
- ***server does not support TLS***: **TLS** is `require` or `verify-full`, but the server
  has encryption turned off. Pick `prefer` or `disable` if the network is one you trust, such
  as a database in a local container, or turn TLS on in the server.
- ***invalid peer certificate: UnknownIssuer***: with `verify-full`, the certificate isn't
  signed by an authority dbdesk trusts. Set **CA file** to your provider's or your company's
  authority certificate (a PEM file).
- ***certificate not valid for name …; certificate is only valid for …***: with
  `verify-full`, the certificate doesn't list the **Host** you typed. Use one of the names the
  message lists, often the one in the provider's connection string, instead of an IP address
  or an alias.
- ***could not read the CA file …*** or ***… holds no usable certificate***: the **CA file**
  is missing, or isn't a PEM file with a certificate in it (`-----BEGIN CERTIFICATE-----`).

Don't turn off encryption on a server just for dbdesk.

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

## SSH tunnels

- **"the SSH server … host key has changed … refusing to connect"**: the key the server presents
  isn't the one `~/.ssh/known_hosts` holds for it. That is what someone impersonating the server
  would look like, so dbdesk never connects. If the server's key really changed (it was
  reinstalled, say), ask its administrator for the new fingerprint, remove the line the message
  names from `~/.ssh/known_hosts` (or run `ssh-keygen -R '[host]:port'`), and connect again to be
  asked about the new key.
- **"the SSH server refused to sign in … with the keys in the SSH agent"**: the agent holds no
  key the server accepts. `ssh-add -l` lists the agent's keys; `ssh-add` adds yours. On Windows,
  start the *OpenSSH Authentication Agent* service, or Pageant.
- **"could not use the SSH agent"**: no agent is running where dbdesk can find it. An app started
  from the Dock or the Start menu may not see the `SSH_AUTH_SOCK` your terminal has; choose
  **Key file** instead.
- **"the SSH server could not forward to host:port"**: you signed in, but the SSH server can't
  reach the database at that address, or isn't allowed to forward there (`AllowTcpForwarding`
  and `PermitOpen` in its `sshd_config`). **Host** and **Port** are the database as the SSH
  server sees it.

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

## "actions.toml: N problems" in the status bar

The same for `actions.toml`, `snippets.toml` or `theme.toml`: an entry that couldn't be used was
skipped, and the rest of the file applies. Click the message to see each problem, fix the file
and click **Reload**. For `theme.toml` it can also be a colour that leaves some text too faint to
read; that colour still applies. See [Actions, snippets and themes](customising.md).

## The panels are in a mess

**View → Reset layout** puts every panel back where it started. If you closed the Welcome tab,
this brings it back too.
