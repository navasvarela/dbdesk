# Database logos

One logo per database dbdesk has a driver for, shown next to data sources in the explorer, in
the New data source menu and in the data source dialog.

| File | Logo | Brand colour |
| --- | --- | --- |
| `postgresql.svg` | PostgreSQL | `#4169E1` |
| `sqlite.svg` | SQLite | `#003B57` |

## Source and licence

The files come from [Simple Icons](https://simpleicons.org) 16.34.0
(`https://cdn.jsdelivr.net/npm/simple-icons@16.34.0/icons/<slug>.svg`), whose icon data is
released under [CC0 1.0](https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md).

The only change is `fill="#FFFFFF"` on the root `<svg>` element. Simple Icons leave the fill
unset, which renders black, and egui tints an image by multiplying its colours, so a black
shape would stay black whatever the tint. White takes the tint exactly, which is how each logo
gets a shade that reads in the light and the dark theme (`Palette::driver` in
`crates/dbdesk-app/src/theme.rs`).

The logos themselves remain trademarks of their owners. dbdesk shows them only to identify the
database a data source connects to. See each project's policy, e.g.
[PostgreSQL's](https://www.postgresql.org/about/policies/trademarks/).

## Adding a database

1. Download the icon from the same Simple Icons release (find its slug on simpleicons.org; some
   brands, such as Oracle, have asked Simple Icons to remove theirs).
2. Add `fill="#FFFFFF"` to its `<svg>` element and save it here as `<slug>.svg`.
3. Add it to `icons::driver_logo`, give it light- and dark-theme shades in `Palette::driver`,
   and add a row to the table above. The contrast test checks the shades.
