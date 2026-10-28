# Hackability

Status: **H1 built** (2026-10-28, in layers H1a–H1f in the
[roadmap](roadmap.md#hackability-track)); H2 and H3 planned. Proposed on 2026-10-24. The
decision behind it is [ADR 0005](adr/0005-customisation-files-commands-then-wasm.md).

Personalisation is one of the two reasons dbdesk exists (see [Vision](vision.md)). Today you can
rebind keys and change how SQL is formatted. This plan makes dbdesk hackable in three steps,
from cheap and safe to powerful:

1. **H1 — Files:** custom actions that run templated SQL, snippets, and themes, all in
   hand-editable files like `keymap.toml`.
2. **H2 — Commands:** actions that run your own programs, with what you selected handed to them.
3. **H3 — WebAssembly plugins, later:** sandboxed code from any language. They are built only
   once real uses show H1 and H2 falling short.

Levels 1 and 2 cover most of what people want from a hackable tool, at a fraction of the cost of
a plugin system. Level 3 stays possible because the extension points it would use are the same
ones H1 and H2 define.

## Extension points

What can be extended, and which level opens it up:

| Extension point | H1 files | H2 commands | H3 plugins |
| --- | --- | --- | --- |
| Actions in the explorer's right-click menu | SQL actions | command actions | ✓ |
| Actions on the grid's selected cells or rows | SQL actions | command actions | ✓ |
| Snippets in the console | `snippets.toml` | — | ✓ |
| Colours | `theme.toml` | — | — |
| Key bindings, for every built-in and custom action | `keymap.toml` (exists) | — | — |
| Cell viewers (JSON exists; geometry, images, …) | — | output to the viewer | ✓ |
| Export formats | — | command actions | ✓ |
| Lint rules | — | — | ✓ |
| Where data sources come from (1Password, Vault, a team file) | — | — | ✓ |

Plugins never draw their own panels. egui has no stable widget boundary to hand to outside code,
so every extension adds entries to lists the app already draws: menus, viewers, rules. This keeps
the UI consistent and the interface small.

## H1 — Files

### `actions.toml`

Custom actions that run SQL, attached to what you right-click:

```toml
[[action]]
id = "row-count"                     # stable; bindable in keymap.toml as "custom.row-count"
name = "Row count"
applies_to = ["table", "view"]       # node kinds, or "data_source", "grid.selection", "grid.row"
drivers = ["postgres", "mysql"]      # optional; every driver when left out
sql = "SELECT count(*) FROM {qualified_name}"
run = true                           # run at once; false opens it in a console to edit first

[[action]]
id = "latest"
name = "Last 100 rows"
applies_to = ["table"]
sql = "SELECT * FROM {qualified_name} ORDER BY {column:created_at} DESC LIMIT 100"
```

- **Placeholders:**
  - Names: `{qualified_name}`, `{database}`, `{schema}`, `{table}`, `{column}`.
  - `{data_source}`: the data source's name in dbdesk, as a string literal, since it names
    nothing in the database.
  - Values from the grid: `{value}` (the first selected cell), `{values}` (the selection, as
    a comma-separated list of literals) and `{row.<column>}`.
  - A brace that doesn't start a placeholder, as in a JSON literal, is left alone; `{{` stands
    for a `{` that would.
- **Placeholders are never pasted in as raw text.** Names are quoted as identifiers by the
  dialect's own rules, which DB0 gathered in one place. Values become escaped literals. A table
  called `x; DROP TABLE y` stays one quoted name, not a second statement.
- **`{column:created_at}`** names a column, with a default the action can fall back on.
- **A placeholder an action can never fill is a problem with the file,** found when it is read:
  `{value}` on a table, or `{column}` without a default on anything but a column or the grid.
- **`grid.selection` runs once, `grid.row` once per selected row,** all in one run.
- **From a key or the Actions menu,** an action acts on the focused console's selected cells,
  or on the explorer's selected node.
- **Statements still go through the existing safety checks.** An action's `DELETE` without
  `WHERE` asks first, like one typed by hand.
- **Problems with the file work like `keymap.toml`:** an action that doesn't parse is skipped
  and listed, the status bar counts problems, and **View → Actions…** lists every action with a
  **Reload** button.
- **Keys:** every custom action gets an id in the keymap (`custom.<id>`), so it can have a
  shortcut.

### `snippets.toml`

```toml
[[snippet]]
trigger = "sel"
description = "Select with a filter"
body = "SELECT ${1:*}\nFROM ${2:table}\nWHERE ${3:condition};"
drivers = ["postgres"]               # optional
```

- **Completion offers snippets** whose trigger matches what you typed, marked as snippets.
- **Accepting one inserts the body.** Tab moves through `${n:…}` fields in order, as in VS Code.
- **The syntax is the common one** (`${1:default}`, `$0` for the final cursor), so snippets can
  be copied from other editors.

### `theme.toml`

```toml
[light]
primary = "#5b3fe0"

[dark]
primary = "#7c5cff"
base = "#14121a"
```

- **It overrides the palette tokens** the theme module already uses (see
  [Look and feel](features/look-and-feel.md)), each theme its own: `primary`, `secondary`,
  `success`, `warning`, `error`, `base`, `panel`, `field`, `control`, `control_hover`,
  `border`, `text`, `text_weak`, `selection`, `on_primary`, as `#rrggbb`.
- **Whichever theme shows takes its overrides,** under Match system, Light and Dark alike.
- **A token with too little contrast still applies,** and is listed as a problem, using the same
  contrast check the palette's test runs.
- **On macOS the accent colour still tints selection and focus,** unless the file sets that
  theme's `primary` or `selection`.

**H1 exit:**
- I add a "Row count" action and a "Last 100 rows" action that uses a column default, and bind
  one to a key.
- I add two snippets and a theme with my own primary colour.
- A table named with a quote and a semicolon still works with every action.

## H2 — Commands

An action can run a program instead of SQL:

```toml
[[action]]
id = "rows-to-jq"
name = "Send rows to jq"
applies_to = ["grid.selection"]
command = ["jq", "-r", ".[] | .email"]
stdin = "json"                       # the selected rows: json, csv, tsv, or none
output = "viewer"                    # where stdout goes: viewer, console, log, or none

[[action]]
id = "pgcli"
name = "Open in pgcli"
applies_to = ["data_source"]
drivers = ["postgres"]
command = ["open", "-a", "Terminal", "--args", "pgcli", "postgres://{user}@{host}:{port}/{database}"]
output = "none"
```

- **No shell.** `command` is a list of arguments, run directly. Each placeholder fills exactly
  one argument, so a table name full of spaces and quotes can't become extra arguments or a
  second command. A shell is still possible when someone writes `["sh", "-c", …]` deliberately.
- **Never secrets.** There is no `{password}` placeholder, and no password reaches a command
  through its environment either. A program that needs one gets it from its own configuration,
  or from the keychain itself.
- **Context in the environment:**
  - `DBDESK_DATA_SOURCE`, `DBDESK_DRIVER`, `DBDESK_DATABASE`, `DBDESK_SCHEMA`, `DBDESK_TABLE`;
  - `DBDESK_FORMAT` for what is on stdin.
- **Runs in the engine,** on its blocking pool, so the UI never waits:
  - with a timeout (`timeout = "30s"`, default 60 s);
  - stoppable with ⌘. like a query;
  - with output capped at a few megabytes.
- **Output:**
  - **viewer** shows stdout in the cell viewer, JSON pretty-printed;
  - **console** opens it in a new query console, for a program that writes SQL;
  - **log** sends it and stderr to the log console;
  - a non-zero exit shows stderr in an error.
- **Trust.** Command actions come only from the user's own `actions.toml`. Nothing else may
  add them:
  - Shared or imported configuration (the planned data source export and import) never carries
    actions.
  - **View → Actions…** shows each command in full before you first run it.

**H2 exit:**
- I send selected rows to `jq` and see the result in the viewer.
- I open a data source in `pgcli`.
- A program that writes SQL opens its output in a console.
- A runaway command is stopped by its timeout and by ⌘..
- A table named with a quote, a space and a semicolon reaches the command as one argument.

## H3 — WebAssembly plugins (later)

Started only when real uses that H1 and H2 can't express have piled up: at least three, written
down in this file as they come. Likely ones are a geometry cell viewer, a Parquet exporter, a
lint rule for a team's conventions, and data sources read from 1Password.

The shape it would take, so H1 and H2 don't close the door:

- **Runtime:** [wasmtime](https://wasmtime.dev) with the WebAssembly component model and a WIT
  interface, or [Extism](https://extism.org), which wraps it with a simpler interface. Choose
  when starting, after comparing how each handles host functions and how much they add to the
  downloads.
- **Interface:** the extension points in the table above, as WIT functions:
  - an action gets the context and returns statements, text or rows;
  - a viewer gets a cell and returns text or simple markup;
  - an exporter gets rows and returns bytes;
  - a lint rule gets tokens and returns problems.
- **Capabilities.** A plugin declares what it needs in its manifest: read the catalog, run
  read-only queries, run changes, reach the network, read files. You grant each one when
  installing it. Running queries reuses the read/write/schema/admin model and its enforcement
  from the [MCP plan](mcp-server.md), so there is one permission model, not two.
- **Installing:** a folder per plugin under `plugins/` in the config directory, with a manifest
  and a `.wasm` file. There is no registry at first; installing is copying a folder or picking a
  file. Updates and a registry come only if people actually share plugins.
- **Drivers stay native.** Database drivers need sockets, TLS and long-lived connections, which
  WASI only partly provides, so a new engine is still a crate (see
  [Adding a database](adding-a-database.md)).

## Decisions

Recorded in [ADR 0005](adr/0005-customisation-files-commands-then-wasm.md). In short:

| Question | Decision |
| --- | --- |
| Runtime plugins at all | Yes, eventually, as sandboxed WebAssembly, and only once H1 and H2 prove too small. |
| Native plugins (`dylib`) | No: Rust has no stable ABI, and the workspace forbids `unsafe`. |
| An embedded scripting language (Lua, Rhai) | No: a second API to maintain, with less isolation than WASM and fewer languages to write in. |
| Who may add command actions | Only the user's own `actions.toml`; never shared or imported files. |
| A fourth theme from `theme.toml` | No: the file overrides the light and dark themes, so Match system keeps working and View → Theme keeps egui's three choices. |
| What a key-bound action acts on | The focused console's selected cells, or the explorer's selected node, which clicking a node now sets. |

## Risks

- **A command action running something unintended.** There is no shell, each placeholder fills
  exactly one argument, commands come only from the user's own file, and they are shown in full
  before their first run.
- **Placeholders injecting SQL.** Names are quoted as identifiers and values escaped as
  literals, never pasted in as text, and a test runs every placeholder with hostile names.
- **A plugin interface that is hard to change once people depend on it.** H3 starts with a
  version in its WIT package, and only after H1 and H2 have shown which extension points matter.
- **Two ways to do the same thing.** Each extension point says which level serves it (the table
  above), and a plugin is the answer only where files and commands can't be.
