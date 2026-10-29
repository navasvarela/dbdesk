# Keymap

Every action a key can trigger has a stable id, and `keymap.toml` in the config directory can
rebind it. Status: implemented in M5.

## The file

```toml
[bindings]
"console.run" = "cmd+r"                    # one shortcut
"console.explain" = ["cmd+alt+e", "f5"]    # or several
"console.cancel" = []                      # or none: unbound
```

- Only the actions listed change; every other action keeps its default.
- A shortcut is modifiers and a key joined by `+`, ignoring case. Modifiers: `cmd` (⌘ on macOS,
  Ctrl elsewhere, egui's `COMMAND`), `ctrl` (the Control key everywhere), `alt` (`option`),
  `shift`. Keys use egui's names: letters, digits, `enter`, `space`, `tab`, `escape`, `f1`…`f35`,
  `period`, `comma`, `pagedown`, or the character itself (`cmd+.`, `cmd++`).
- **View → Keymap…** lists every action with its id, shortcuts and description, and has
  **Create keymap.toml** (a file with every action and its default, commented out; an existing
  file is never replaced) and **Reload** (edits apply without restarting).
- Mistakes don't cost the other bindings: an unknown action id or a shortcut that doesn't parse
  is skipped and listed as a problem, as is a shortcut two actions share (the first in the list
  below keeps it). The status bar says "keymap.toml: N problems" and opens the window. A file
  that isn't valid TOML leaves every default in place and shows the parse error.

## Actions

| Id | Default | What it does |
| --- | --- | --- |
| `console.complete` | ⌃Space | Suggest names and keywords at the cursor |
| `console.run` | ⌘↵ | Run the selection or the statement at the cursor |
| `console.run-all` | ⇧⌘↵ | Run every statement in the console |
| `console.explain` | ⌥⌘E | Explain the statement at the cursor |
| `console.cancel` | ⌘. | Stop the running statement or discard unread rows |
| `console.history` | — | Open the data source's query history |
| `console.commit` | — | Commit the console's transaction |
| `console.rollback` | — | Roll back the console's transaction |
| `console.format` | ⌥⌘L | Format the selection, or the statement at the cursor |
| `console.format-all` | ⇧⌥⌘L | Format every statement in the console |
| `console.next-problem` | F8 | Go to the next syntax error or warning |
| `console.previous-problem` | ⇧F8 | Go to the previous syntax error or warning |
| `grid.view-cell` | Space | Show the selected cell in the cell viewer |
| `grid.edit-cell` | F2, Enter | Edit the selected cell, if its table can be edited |
| `grid.copy-with-headers` | ⇧⌘C | Copy the selected cells with their column names |
| `grid.copy-as.csv`, `.tsv`, `.markdown`, `.sql-insert`, `.json`, `.jsonl`, `.html`, `.sql-update` | — | Copy the selected cells in that format, as **Copy as ›** does |
| `app.settings` | ⌘, | Open the settings |

Console actions work while the console's editor has focus, and are taken before the editor
sees the key. The `grid.*` actions work while no text field has focus. Toolbar buttons show their
action's current shortcut in their tooltip, and menu items theirs, in the egui menu bar and in
macOS's (see [A native look on macOS](../macos-look.md)).

## Design

- The engine reads the file (at start-up, before the data sources, and on Reload) and sends
  the raw strings as `Event::KeymapLoaded`; the app parses them into egui shortcuts. The UI
  thread never touches the file ([ADR-0003](../adr/0003-ui-engine-message-passing.md)), and the
  store doesn't depend on egui.
- egui's shortcut matching lets extra Shift and Alt through, so ⌘↵ would also fire on ⇧⌘↵.
  The keymap tries shortcuts with more modifiers first, whatever order the file lists them in.
- Toolbar buttons and shortcuts go through the same function, so a key does exactly what its
  button does, and is ignored when the button is disabled.

*Planned*: bindings for the explorer and the app (new console, close tab), and chords.
