# A native look on macOS

Status: **planned** (2026-10-27). This is the design of D5 in the
[roadmap](roadmap.md#d5--a-native-look-on-macos).

dbdesk draws everything with egui, so on macOS it looks like an egui app in a Mac window: a
menu bar inside the window, a grey title bar above it with the word "dbdesk", Inter instead of
the system font, and teal selection whatever accent colour the user picked. D5 makes it read as
a Mac app while keeping egui for everything inside the window, and leaves Windows and Linux as
they are.

## What stays the same

- **egui draws the window's contents** ([ADR 0001](adr/0001-ui-toolkit-egui.md)). Nothing here
  replaces a panel with a native control; the menu bar and the title bar are the window's
  chrome, not its contents.
- **One set of commands.** A menu item does exactly what the same item does in the egui menu
  and what its shortcut does, through one function, so the two menus can't drift apart.
- **The keymap still decides shortcuts** ([keymap](features/keymap.md)). A menu item for a
  keymap action shows, and answers to, the keys `keymap.toml` gives it, in either menu.
- **Windows and Linux keep the egui menu bar** and today's fonts and colours. macOS-only code is
  behind `cfg(target_os = "macos")`, and CI still builds and tests all three systems.
- **Tests and the manual's pictures don't depend on the machine.** The native menu, the
  integrated title bar, the system font and the accent colour are switched on by `main` for the
  real window only. UI tests and `screenshots::regenerate_screenshots` build the app without
  them, so they render the same bundled fonts and palette on every system, and the existing UI
  tests that click through the egui menu keep working on macOS.

## D5a — One list of commands

A `commands` module lists what the menus can do: `AppCommand` (Open…, Save, Export data
sources…, Theme › Light, Format, About dbdesk, and so on), each with

- a stable id (`file.open`, `view.theme.dark`), for the native menu's item ids and for tests;
- its label, as the egui menu shows it today;
- its shortcut: the keymap action it runs (`keymap::Action::Save`), so a rebinding applies to
  the menus too, or none;
- whether it is enabled and checked, from a small `CommandState` (is a console focused, are
  there data sources, which theme is picked) that the app fills in once per frame;
- its tooltip and the reason it is disabled.

The menu bar itself is data too: `commands::egui_menu_bar` returns the menus, their entries,
separators and submenus (with Open recent's files), and `app.rs` walks it to draw egui's
`MenuBar`. Every click, and the keymap's file shortcuts, go through one
`DbDeskApp::run_command`. This layer changes no behaviour: the egui menu has the same items,
shortcuts, tooltips, enabled states and icons as before, which kittest tests pin down.

## D5b — The macOS menu bar

On macOS dbdesk installs a real menu bar with [`muda`](https://crates.io/crates/muda) 0.21.2,
the menu crate Tauri uses, and hides the egui one. `commands::macos_menu_bar` arranges the same
commands the way Mac apps do:

| Menu | Items |
| --- | --- |
| **dbdesk** | About dbdesk, Check for updates…, Settings… ⌘,, Services, Hide dbdesk ⌘H, Hide Others ⌥⌘H, Show All, Quit dbdesk ⌘Q |
| **File** | Open… ⌘O, Open recent ›, Save ⌘S, Save as… ⇧⌘S, Export data sources…, Import data sources… |
| **Edit** | Undo ⌘Z, Redo ⇧⌘Z, Cut ⌘X, Copy ⌘C, Paste ⌘V, Select All ⌘A |
| **View** | Theme ›, Keymap…, Reset layout, Enter Full Screen ⌃⌘F |
| **Code** | Format ⌥⌘L, Format all ⇧⌥⌘L |
| **Window** | Minimize ⌘M, Zoom, Bring All to Front |
| **Help** | the manual |

A unit test checks that every command in the egui menu is somewhere in the macOS one.

- **Events.** muda calls a handler on the main thread when an item is chosen; it forwards the
  item's id over a channel and asks egui for a repaint, and the next frame runs the command.
- **Edit goes to the focused egui widget.** muda's predefined Copy and Paste send `copy:` and
  `paste:` to the first responder, which is winit's view, and it doesn't implement them, so they
  would always be greyed out. dbdesk's own Edit items instead add to the frame's input what the
  keys would have: `Event::Copy`, `Event::Cut`, `Event::Paste` with the pasteboard's text, and
  ⌘Z, ⇧⌘Z or ⌘A key presses (in `raw_input_hook`, before any widget runs). The editor, text
  fields and the results grid handle them as they handle the keys today.
- **Quit asks first.** Quit is dbdesk's own item and closes the window the usual way, so open
  transactions and unsaved files are still confirmed. winit's default menu sends `terminate:`,
  which skips that.
- **State stays in sync.** Each frame the app updates enabled and checked marks that changed,
  the shortcuts after `keymap.toml` is reloaded, and Open recent when the list changes.
- **Settings… moves to the app menu with ⌘,** (Ctrl+, in the egui menu on Windows and Linux),
  as a new keymap action `app.settings`.
- **The dependency.** muda is a macOS-only dependency without default features, so Linux
  doesn't pull in GTK. Its macOS dependencies (`objc2` 0.6, `objc2-app-kit` 0.3,
  `crossbeam-channel`, `dpi`) are already in the lockfile through winit and rfd; it adds
  `keyboard-types`. Writing the menu against `objc2-app-kit` by hand would mean our own
  `NSMenuItem` target class and key-equivalent mapping, which is the part muda gets right.

## D5c — An integrated title bar

On macOS the window's content runs under a transparent title bar
(`with_fullsize_content_view`, `with_titlebar_transparent`, `with_title_shown(false)`), so the
traffic-light buttons sit on dbdesk's own top strip, as in Finder, Xcode or TablePlus:

- The strip is as tall as a title bar (28 points), leaves 78 points on the left for the
  buttons, and shows the window title centred in weak text.
- Dragging the strip moves the window (`ViewportCommand::StartDrag`), and double-clicking it
  zooms (`ViewportCommand::Maximized`), as the title bar did.
- In full screen the buttons hide, so the inset goes too.
- Without a native menu (tests) the egui menu sits in the strip after the inset, so the strip
  can be tested with kittest on any system.

## D5d — The system font and accent colour

- **SF Pro and SF Mono**, read at runtime from `/System/Library/Fonts/SFNS.ttf` and
  `SFNSMono.ttf`, never bundled: Apple's licence allows using them as the system font, not
  shipping them. SF Pro is a variable font; the semibold family is the same file at weight 590
  (`FontTweak::coords`). If either file is missing or doesn't parse, dbdesk keeps Inter and
  JetBrains Mono, and says so in the log.
- **The accent colour** (`NSColor.controlAccentColor`, through `objc2-app-kit`) replaces teal
  for selection, focus rings and the text cursor. The selection fill is the accent faded into
  the panel, as the palette's own is, and a test holds text on it to WCAG AA for each of
  macOS's accent colours, in both themes. The brand colours stay for icons, tabs and buttons.
  dbdesk reads the accent again when the window regains focus or the theme changes, so a change
  in System Settings shows without a restart.

## D5e — The manual

The book describes the macOS menu bar, where Settings… and Check for updates… are on a Mac,
the Edit menu, and that dbdesk uses the system font and accent colour there. Pictures stay
rendered with the bundled fonts.

## Decisions

- **muda, not a hand-written `NSMenu`.** See D5b. egui has no native menu of its own, and
  eframe's winit only offers its default app menu.
- **Edit items are dbdesk's, not muda's predefined ones,** because winit's view doesn't answer
  `copy:`/`paste:` (see D5b).
- **The native chrome is opt-in from `main`,** not decided inside `DbDeskApp::new`, so tests on
  a Mac see the same app as on Linux, and the manual's pictures don't change with the machine
  that renders them.
- **Accent for selection and focus only.** Tinting every primary-coloured mark with the accent
  would make a pink-accented dbdesk lose its brand; selection and focus are what macOS itself
  tints.

## Risks

- **Key equivalents swallow keys.** A menu item's shortcut is handled by the menu before the
  window sees the key, so egui no longer gets ⌘S, ⌘C or ⌘Z directly. That is why every menu item
  runs exactly what egui would have, and why the Edit items feed egui the same events. A keymap
  binding that can't be written as a menu key equivalent is left off the item and keeps working
  through egui.
- **Paste reads `NSPasteboard` itself,** since egui-winit's clipboard isn't reachable from the
  app. Only plain text is pasted, as before.
- **System font files can move.** Apple has renamed them before (`SFNSDisplay.ttf`,
  `SFNSText.ttf`); the fallback keeps the app usable, and the log names the missing file.
- **Double-click on the strip always zooms,** ignoring the "double-click a window's title bar
  to minimize" preference.
- **No macOS UI tests.** CI's macOS runner has no window server session to drive a menu bar;
  the native parts are checked by running the app, and the pure parts (the command list, the
  menu layout, shortcut conversion, accent contrast) by unit tests on every system.
