# Look and feel

dbdesk's interface takes its layout and density from DataGrip's new UI, and its colours from
the brand palette ([brand.md](../brand.md)): a calm base, a few accents used sparingly. The code
is in `crates/dbdesk-app/src/theme.rs`, `fonts.rs`, `icons.rs`, `widgets.rs` and `welcome.rs`.

## Themes

There is a light and a dark theme. dbdesk follows the OS setting until the user picks one in
**View → Theme** (Match system, Light, Dark). egui stores that choice with the rest of its
state in `app.ron`, so there is no setting of dbdesk's own to keep in sync.

Both themes are built by one function from a `Palette`, a struct of colours by role:

| Role | Used for |
| --- | --- |
| `primary` | Selection, focus rings, the active tab, the main button on a screen, table icons |
| `secondary` | SQL keywords, links, database and schema icons |
| `success`, `warning`, `error` | Status only: connection dots, run results, log levels |
| `base` | The window behind the panels: the gaps between them, the menu and status bars |
| `panel` | Panels, windows and menus |
| `field` | Text fields, the SQL editor and the results grid |
| `control`, `control_hover` | Buttons and other controls at rest and under the pointer |
| `border`, `text`, `text_weak`, `selection`, `on_primary` | What the names say |

Widgets ask for a role, never a fixed colour, through `Palette::for_visuals(ui.visuals())`. A
colour that only works in one theme can't sneak in that way.

**Contrast is tested.** `theme::tests::text_colours_meet_wcag_aa` checks every text colour
against every background it is drawn on, in both themes, for WCAG AA (4.5:1). It caught the
first selection colours: SQL strings and keywords on selected text fell to 3.5:1. When adding a
role or a new text-on-background combination, add the pair to `text_pairs`.

## Panels as islands

The dock's background is the darker `base`, and each dock node (its tab bar and body) is a
`panel`-coloured rectangle with rounded corners and a 6-point gap around it, so tool windows
read as separate "islands". egui_dock draws this from the style `theme::dock_style` builds;
the gap is the separator, painted in `base`.

- Tool-window tabs (Explorer, Welcome, Log console) have a coloured icon and a semibold title.
  Console tabs use the regular weight, like editor tabs.
- The active tab is a faint primary tint rather than a frame or underline.
- egui_dock's per-node "collapse" and "close all" buttons are hidden: they were the heaviest
  marks in the tab bar and the actions they offer are rarely what you want.
- Below each tab bar is a compact row of small, monochrome icon buttons (`widgets::toolbar`,
  `IconButton`). An icon button's tooltip and accessible name are its label, so screen readers
  and UI tests find "Run all" rather than a private-use character.
- The status bar is a single line of small, weak text with a dot for the engine's state.

## Typography

Three typefaces ship in `assets/fonts/`, each with its OFL licence:

- **Inter** (regular and semibold) for the interface. egui has no font weights, so semibold is
  a font family of its own (`fonts::semibold`), used for headings and tool-window titles.
- **JetBrains Mono** for SQL, result cells and the log console.
- **Phosphor** (from the `egui-phosphor` crate) for icons.

Body text is Inter at 13 points; spacing is tighter than egui's defaults so more tree and grid
rows fit on screen.

**Icons have their own font family** (`fonts::icon`, `widgets::glyph`), and are deliberately
not a fallback of the text families, which is how `egui-phosphor` installs them by default.
Inter has its own glyphs at over 300 of the private-use code points Phosphor uses, so behind
Inter many icons came out as accented letters (a database icon became "⇔", a folder "ṡ").
Phosphor can't go in front of Inter either, because it maps a–z to the blank glyphs its
ligatures are built from. So: every icon goes through the icon family, and a raw icon character
in an ordinary label shows as a missing-glyph box, which is easy to spot.

## Icons and colour

`icons.rs` maps each driver and each catalog node kind to a glyph and a palette role:

| Kind | Icon | Colour |
| --- | --- | --- |
| PostgreSQL / SQLite data source | database / feather | blue / teal |
| Database, schema | database, tree | secondary |
| Folder | folder | warning (amber) |
| Table, view, materialized view | table, eye, stack | primary |
| Function, sequence | function, numbered list | success |
| Column | columns | weak text |
| Index, constraint | magnifying list, key | warning |
| Trigger | lightning | error |

Loaded folders show how many children they have as a small, weak badge after the label
(`tables 4`), counting system objects only while they are shown.

A data source's status dot sits after its name: amber while connecting, green when connected,
red when the last attempt failed, and no dot when disconnected, so the eye goes to the sources
that are live or in trouble.

## Data source colours

A data source can be given one of eight colours in its dialog (red, orange, yellow, green,
teal, blue, violet, pink), stored as `color = "red"` in `data_sources.toml`. The colour tints
the data source's explorer row, its console tabs (fill and outline) and a thin band along the
top of each of its consoles, so a console on production is hard to mistake for one on a local
copy.

It is a fixed set of names (`dbdesk_core::SourceColor`) rather than free RGB: each name has a
light-theme and a dark-theme shade (`Palette::source`), so a colour picked in one theme still
works in the other. The field is optional and serde-defaulted, so files written before it
existed still load, and it is only written when set.

## Welcome tab

The Welcome tab shows the logo over two soft radial glows (secondary on the left, primary on
the right, drawn as triangle fans because egui has no gradient fills), a title, three square
cards — **New data source**, **New query console** and **Open the manual** — and a short card
with a solid primary button to the manual. The card a new user most likely wants next has an
accent border: New data source while there are none, New query console after that. New query
console is disabled until a data source exists.

## Results grid and syntax colours

The grid's header row is tinted with the primary, rows are striped, and cells use JetBrains
Mono, with numbers right-aligned and `NULL` in weak italics. Syntax colours come from the
palette: keywords secondary, strings success, numbers warning, quoted identifiers primary,
comments weak and italic.

## Screenshots

Pictures of the UI, including every image in the user manual (`book/`), are rendered, not
taken by hand: `screenshots::regenerate_screenshots` (ignored by default) drives the app
headlessly with egui_kittest's wgpu renderer against a SQLite fixture, and a PostgreSQL data
source when `DBDESK_TEST_POSTGRES_URL` is set. See
[testing.md](../testing.md#screenshots).
