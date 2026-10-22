# Persistence

All state lives in the platform config directory from the `directories` crate
(`~/Library/Application Support/dbdesk` on macOS).

```
dbdesk/
├── data_sources.toml     # data source configs, no secrets
├── settings.toml         # app settings: page size, theme, fonts
├── keymap.toml           # M5
├── actions.toml          # M5, custom catalog actions
├── layout.ron            # egui_dock state, panel sizes, open tabs
├── consoles/
│   └── <data-source-id>/<console-name>.sql
├── history.sqlite        # query history
└── logs/dbdesk.log.*     # rolling file log
```

## Data sources

- TOML because it is the file I'll most often edit by hand.
- Each entry has a stable `id` (UUID, written once) so renames don't break consoles, history or
  keychain entries.
- Written atomically (write to temp file, then rename) so a crash never truncates it.
- Unknown fields are preserved on save, so a newer config doesn't lose data when opened by an
  older build. (Implemented by keeping the parsed `toml_edit::DocumentMut` and editing it in place.)

## Secrets

- Passwords go to the OS keychain via `keyring`, keyed by `dbdesk/<data-source-id>`.
- If the keychain is unavailable or the user declines, fall back to asking for the password on
  connect and keeping it in memory for the session only. Never write secrets to disk.
- Later: support `~/.pgpass` and `PG*` environment variables for Postgres, which is what other
  tools on the machine already use.

## Query history

SQLite table:

```sql
CREATE TABLE history (
    id            INTEGER PRIMARY KEY,
    data_source   TEXT NOT NULL,
    executed_at   TEXT NOT NULL,      -- RFC 3339
    sql           TEXT NOT NULL,
    duration_ms   INTEGER,
    rows          INTEGER,
    outcome       TEXT NOT NULL       -- ok | error | cancelled
);
CREATE INDEX history_by_source ON history (data_source, executed_at DESC);
```

Searchable from a history dialog per data source. Results themselves are not stored.

## Layout

The `egui_dock` `DockState` is serde-serializable; it is saved on exit and on a timer, and
restored at start-up. Tabs refer to consoles by id, so a console whose file was deleted is
dropped from the layout instead of failing to start.
