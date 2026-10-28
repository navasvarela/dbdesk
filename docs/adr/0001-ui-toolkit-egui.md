# 0001. UI toolkit: egui with egui_dock

- Status: Accepted
- Date: 2026-10-22

## Context

DbRover needs dockable panels, a tree, a tabbed SQL editor, and a data grid that stays smooth
with tens of thousands of rows. The project is also a Rust learning exercise, so most of the code
should be Rust, and the toolkit should be well documented with many examples.

## Decision

Use `egui` through `eframe`, with `egui_dock` for the docking layout and `egui_extras` /
`egui_table` for the virtualized result grid. Highlighting via `syntect`.

## Consequences

- Fast iteration: immediate mode means UI code is a function of `AppState`, with no widget tree
  to keep in sync.
- Docking, tabs and virtualized tables exist as maintained crates.
- `egui_kittest` gives headless UI tests.
- The text editor is the weak spot: `TextEdit` lacks multi-cursor, folding, and efficient
  handling of very large files. Accepted for v1; the editor is isolated behind its own widget so
  it can be replaced (custom widget, or a port of an editor core such as `ropey` + own rendering).
- Immediate mode redraws the whole UI on interaction; layout cost must stay proportional to
  what is visible (virtualize grid and tree).
- Native look is not a goal; egui looks like egui.

## Alternatives considered

- **gpui** (Zed): the best text editing and performance, and `gpui-component` already has dock,
  table and editor widgets. Lost on documentation and API stability, which matter more while
  learning. Worth revisiting if the editor becomes the main pain point.
- **iced**: clean Elm-style architecture with `pane_grid` and `text_editor` built in. Lost
  because a large custom data grid is more work in retained mode, and the ecosystem of ready
  widgets for this kind of app is smaller.
- **Tauri + web UI**: Monaco and a mature JS grid would give the best UX for the least effort,
  but half the code base would be TypeScript, defeating the learning goal.
- **ratatui (TUI)**: light and fun, but a result grid and mouse-driven tree browsing are a worse
  fit for the desktop database IDE being aimed at, in the line of DBeaver, DataGrip and
  TablePlus.
