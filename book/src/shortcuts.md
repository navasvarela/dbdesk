# Keyboard shortcuts

On Linux and Windows, read Ctrl wherever this page says ⌘.

## Defaults

| Shortcut | Action | Works while |
| --- | --- | --- |
| ⌘↵ | Run the selection, or the statement at the cursor | the console's editor has focus |
| ⇧⌘↵ | Run every statement in the console | the editor has focus |
| ⌥⌘E | Explain the statement at the cursor | the editor has focus |
| ⌘. | Cancel the running statement, or discard rows not yet loaded | the editor has focus |
| ⌃Space | Suggest names and keywords at the cursor | the editor has focus |
| Space | Show the selected cell's whole value | no text field has focus |
| F2, Enter | Edit the selected cell | no text field has focus |
| ⌘C | Copy the selected cells | no text field has focus |

Console shortcuts are taken before the editor sees them, so ⌘↵ runs a statement instead of
inserting a new line. **History**, **Commit** and **Rollback** have no shortcut by default.

## Changing them

Shortcuts live in `keymap.toml`, next to `data_sources.toml` (see
[Where dbdesk keeps its files](files.md)). **View → Keymap…** lists every action with its id,
its shortcuts and what it does, and has two buttons:

- **Create keymap.toml** writes a file listing every action with its default, commented out. It
  never replaces a file that exists.
- **Reload** applies your edits without restarting dbdesk.

In the file, list only the actions you want to change:

```toml
[bindings]
"console.run" = "cmd+r"                    # one shortcut
"console.explain" = ["cmd+alt+e", "f5"]    # or several
"console.cancel" = []                      # or none
```

A shortcut is modifiers and a key joined by `+`, ignoring case. `cmd` is ⌘ on macOS and Ctrl
elsewhere; `ctrl` is the Control key everywhere; `alt` (or `option`) and `shift` are what they
say. Keys are letters, digits, `enter`, `space`, `tab`, `escape`, `f1` to `f35`, `period`,
`comma`, `pagedown` and so on, or the character itself (`cmd+.`).

A mistake doesn't cost you the rest of the file: an unknown action, a shortcut that can't be
read, or one that two actions share, is skipped and listed as a problem in the Keymap window,
and the status bar shows **keymap.toml: N problems**. A file that isn't valid TOML keeps every
default and shows why.

## Editing text

The SQL editor and the other text fields use the usual editing keys:

| Shortcut | Action |
| --- | --- |
| ⌘A | Select all |
| ⌘C, ⌘X, ⌘V | Copy, cut, paste |
| ⌘Z, ⇧⌘Z | Undo, redo |
| ⇧ with arrow keys | Extend the selection |
| ⌥← / ⌥→ (Ctrl on Linux and Windows) | Move by word |

## Dialogs

**Esc** closes the data source dialog, the delete confirmation and the "Run anyway?" check
without doing anything.
