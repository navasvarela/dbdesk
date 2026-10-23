# Where dbdesk keeps its files

dbdesk keeps everything in one folder per platform:

| Platform | Settings | Window state and logs |
| --- | --- | --- |
| macOS | `~/Library/Application Support/dbdesk/` | same folder |
| Linux | `~/.config/dbdesk/` | `~/.local/share/dbdesk/` |
| Windows | `%APPDATA%\dbdesk\config\` | `%APPDATA%\dbdesk\data\` |

| File | What it holds |
| --- | --- |
| `data_sources.toml` (settings folder) | Your data sources. No passwords. |
| `app.ron` (window state folder) | The window's size and position, the panel layout, the theme you picked. |
| `logs/dbdesk.YYYY-MM-DD.log` (window state folder) | One log file per day. The last seven are kept. |

Passwords are not in any of these files: they are in your system's keychain, under the service
name `dbdesk`. See [Data sources](data-sources.md#passwords-and-the-keychain).

## data_sources.toml

You can edit this file by hand **while dbdesk is closed**. dbdesk reads it at start-up and
rewrites it every time you save or delete a data source, so changes made while it runs are
overwritten, and comments are not kept.

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

- `id` identifies the data source for good, including its keychain entry. Don't change it; for
  a new data source by hand, any unique UUID will do.
- `color` is optional: `red`, `orange`, `yellow`, `green`, `teal`, `blue`, `violet` or `pink`.
- `read_only` is optional and defaults to `false`.

If the file can't be read — say, a typo while editing it — the explorer shows the error and
dbdesk refuses to save or delete anything, so it never overwrites your file with an empty list.
Fix the file and restart dbdesk.

## Starting over

To reset the layout, use **View → Reset layout**. To forget everything, quit dbdesk and delete
its folders; the passwords stay in the keychain until you delete the `dbdesk` entries there
too.
