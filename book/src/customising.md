# Actions, snippets and themes

Three files let you add to dbdesk without rebuilding it: `actions.toml` adds your own SQL to the
explorer's and the grid's menus, `snippets.toml` adds snippets to completion, and `theme.toml`
changes the colours. They sit next to `keymap.toml` (see
[Where dbdesk keeps its files](files.md)), and none of them exists until you write it.

They are read when dbdesk starts. After editing one, click **Reload** in **View → Actions…**,
or in the window the status bar opens for the file: that reads all three again.

A mistake costs only the entry it is in: an action, snippet or colour that can't be used is
skipped, and the status bar shows **actions.toml: N problems** (or **snippets.toml**,
**theme.toml**). Click it to see each problem. A file that isn't valid TOML adds nothing, and
says on which line it went wrong.

## Actions

An action is SQL you attach to what you right-click:

```toml
[[action]]
id = "row-count"
name = "Row count"
applies_to = ["table", "view"]
drivers = ["postgres", "mysql"]
sql = "SELECT count(*) FROM {qualified_name}"

[[action]]
id = "latest"
name = "Last 100 rows"
applies_to = ["table"]
sql = "SELECT * FROM {qualified_name} ORDER BY {column:created_at} DESC LIMIT 100"
run = false
```

- **`id`** names the action for good: lowercase letters, digits, `-` and `_`. It is how
  `keymap.toml` binds it.
- **`name`** is what the menus show.
- **`applies_to`** says where the action appears:
  - explorer nodes by kind: `database`, `schema`, `table`, `view`, `materialized_view`,
    `function`, `procedure`, `sequence`, `event`, `synonym`, `dictionary`, `column`, `index`,
    `constraint`, `trigger`, `role`;
  - `data_source`, a data source's own row;
  - `grid.selection`, the grid's selected cells, acted on once;
  - `grid.row`, the grid's selected rows, acted on once each.
- **`drivers`** limits it to some databases: `postgres`, `sqlite`, `mysql`, `sqlserver`,
  `duckdb`, `clickhouse`. Leave it out for every one.
- **`run = false`** opens the SQL in a new console for you to edit and run. Otherwise it runs at
  once.

Choosing an action opens a new console on the same data source with the SQL filled in. Running
it is the same as running SQL you typed: `:name` parameters are asked for, a `DELETE` without
`WHERE` asks first, a careful data source asks before any change, and a read-only one refuses
them. The history marks the statements **Action**, with the action's id.

### Placeholders

| Placeholder | Becomes |
| --- | --- |
| `{qualified_name}` | The object's name, with its schema: `"public"."orders"` |
| `{database}`, `{schema}`, `{table}` | The database, schema or table the object is in, or is |
| `{column}` | The column right-clicked, or the first selected column in the grid |
| `{column:created_at}` | The same, or `created_at` where there is no column |
| `{data_source}` | The data source's name in dbdesk, as a string: `'shop'` |
| `{value}` | The first selected cell, as a literal: `'42'`, or `NULL` |
| `{values}` | Every selected cell, as literals separated by commas, for `IN (…)` |
| `{row.email}` | The `email` column of the first selected row (of each row, with `grid.row`) |

A placeholder is never pasted in as text. Names are quoted as the database quotes them
(`"…"`, `` `…` `` or `[…]`), and values become string literals, so a table called
`x; DROP TABLE y` stays one name. Don't put quotes around a placeholder yourself.

A placeholder the action could never fill is a problem with the file: `{value}` on a table, or
`{column}` on a table without a default. One that can't be filled this time, such as `{table}`
on grid cells that don't come from one table, is said in the status bar when you choose the
action. A `{` that doesn't start a placeholder, as in a JSON literal, stays as written; write
`{{` for one that would.

### Keys and the Actions menu

Every action is in the **Actions** menu, and can have a shortcut in `keymap.toml` as
`custom.<id>`:

```toml
[bindings]
"custom.row-count" = "cmd+shift+c"
```

From the menu or a key, an action acts on the focused console's selected cells, or else on the
explorer's selected node: click a node to select it. The item is greyed out when what is
selected isn't something the action applies to. A shortcut that one of dbdesk's own actions
already has stays dbdesk's, and is listed as a problem in **View → Keymap…**, which also lists
your actions' ids.

**View → Actions…** lists every action with its id, what it applies to, its drivers, its keys
and its SQL, and the file's problems. **Create actions.toml** writes the examples above,
commented out, to start from; it never replaces a file you have.

## Snippets

```toml
[[snippet]]
trigger = "sel"
description = "Select with a filter"
body = "SELECT ${1:*}\nFROM ${2:table}\nWHERE ${3:condition};"
drivers = ["postgres"]
```

While you type a word in the editor, completion lists the snippets whose trigger starts with
it, marked with braces, after the names that start with it. **Enter** or **Tab** inserts the
body with its first field selected: type over it, then **Tab** goes to the next field and
**⇧Tab** back. After the last field the cursor goes to `$0`, or to the end, and **Tab** is the
editor's again. **Esc**, or clicking or typing outside the field, leaves the fields.

The syntax is the one other editors use, so snippets can be copied from them: `${1:default}` or
`$1` for a field, `$0` for where the cursor ends, and `\$`, `\}` and `\\` for the characters
themselves. A `$` that starts no field, as in PostgreSQL's `$$`, stays as written; write `\$1`
for a positional parameter. Fields inside fields aren't supported. `trigger` takes letters,
digits and `_`; `description` and `drivers` are optional.

## Themes

```toml
[light]
primary = "#5b3fe0"

[dark]
primary = "#9a8bff"
base = "#14121a"
```

`[light]` and `[dark]` replace colours of the light and dark themes, so **View → Theme** still
offers Match system, Light and Dark, each with your colours. Colours are written `#rrggbb`:

| Name | What it colours |
| --- | --- |
| `primary` | Selection, focus, and the main button on a screen |
| `secondary` | SQL keywords, links |
| `success`, `warning`, `error` | What went right, what to watch, what went wrong |
| `base` | Behind the panels, and the menu and status bars |
| `panel` | The panels, windows and menus |
| `field` | Text fields, the editor and the grid |
| `control`, `control_hover` | Buttons at rest and under the pointer |
| `border` | Lines around panels and windows |
| `text`, `text_weak` | Text, and details, hints and comments |
| `selection` | Behind selected text and rows |
| `on_primary` | Text on a `primary` button |

A colour that leaves text too faint to read on what it is drawn on (below WCAG AA's 4.5:1) still
applies, and is listed as a problem, naming the text and the background.

On a Mac, selection and focus take your accent colour from System Settings, as they do without a
theme, unless `theme.toml` sets that theme's `primary` or `selection`.
