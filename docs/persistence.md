# Persistence

All state lives in the platform config directory from the `directories` crate
(`~/Library/Application Support/dbdesk` on macOS).

```
dbdesk/
├── data_sources.toml     # data source configs, no secrets
├── settings.toml         # app settings: page size, theme, fonts
├── keymap.toml           # M5
├── actions.toml          # M5, custom catalog actions
├── app.ron               # eframe storage: window size, egui_dock layout, open tabs
├── consoles/
│   └── <data-source-id>/<console-name>.sql
├── history.sqlite        # query history
└── logs/dbdesk.<date>.log # daily rolling file log, last 7 kept
```

## Data sources

- TOML because it is the file I'll most often edit by hand.
- Each entry has a stable `id` (UUID, written once) so renames don't break consoles, history or
  keychain entries.
- Written atomically (write to temp file, then rename) so a crash never truncates it.
- Owned by the engine: loaded once at start-up, rewritten on every save or delete.
- If the file exists but can't be parsed, the engine refuses all changes and the explorer shows
  the error. A broken file is never overwritten with an empty list.
- `color` is optional (one of `red`, `orange`, `yellow`, `green`, `teal`, `blue`, `violet`,
  `pink`) and only written when set, so files from before it existed load unchanged.
- Comments and unknown fields are **not** preserved when dbdesk rewrites the file (it is written
  with `serde` + `toml`), and the file header says so. Switching to `toml_edit` to edit the
  document in place is a follow-up if hand-editing becomes common.

```toml
[[data_source]]
id = "6f1c0d9e-3b8a-4c3e-9f57-2d1e8a7b5c40"
name = "local"
color = "teal"
driver = "postgres"
host = "localhost"
port = 5432
database = "app"
user = "juan"

[[data_source]]
id = "0b6a4f8e-91d2-4e7a-a1c3-5f0e2d9b7c18"
name = "chinook.db"
driver = "sqlite"
path = "/Users/juan/data/chinook.db"
read_only = true
```

## Secrets

- Passwords go to the OS keychain via `keyring`: service `dbdesk`, account `<data-source-id>`.
  The engine only reads the keychain for drivers that use passwords (not SQLite), so opening a
  SQLite file never triggers a keychain prompt.
- Behind the `SecretStore` trait, so tests use an in-memory store. A real-keychain round trip is
  an opt-in test: `cargo test -p dbdesk-store -- --ignored`.
- Deleting a data source removes its keychain entry. If that fails, the failure is only logged,
  since a leftover entry is harmless.
- Not yet: if the keychain is unavailable or the user declines, fall back to asking for the
  password on connect and keeping it in memory for the session only. Never write secrets to disk.
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

The `egui_dock` `DockState` is serde-serializable. It is stored under the `dock_layout` key in
eframe's own storage (`app.ron`, written on exit and every 30 seconds) and restored at start-up.
A saved layout that no longer deserializes (for example after a `Tab` variant is removed) is
replaced by the default layout; **View → Reset layout** does the same on demand.

Tabs will refer to consoles by id, so a console whose file was deleted is dropped from the
layout instead of failing to start.
