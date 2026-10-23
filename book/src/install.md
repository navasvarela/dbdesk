# Install and first launch

dbdesk has no installer or prebuilt download yet. You build it from source, which takes a few
minutes the first time.

## What you need

- **Rust 1.95 or newer**, installed with [rustup](https://rustup.rs). Check with
  `rustc --version`.
- **Git**, to fetch the source.
- On **Linux**, the development headers for the windowing system. On Debian or Ubuntu:

  ```bash
  sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
  ```

dbdesk is developed on macOS, and its tests also run on Linux. Windows isn't tested.

## Build and run

```bash
git clone https://github.com/navasvarela/dbdesk.git
cd dbdesk
cargo run --release -p dbdesk-app
```

To install the `dbdesk` command so you can start it from anywhere:

```bash
cargo install --path crates/dbdesk-app
```

## First launch

![dbdesk on first launch](images/first-launch.png)

On first launch the explorer is empty ("No data sources yet") and the Welcome tab offers three
things to do:

- **New data source** — add a connection to a database. See [Data sources](data-sources.md).
- **New query console** — open a console on a data source. It is greyed out until you have
  one.
- **Open the manual** — opens this manual in your browser.

The log console at the bottom shows `engine started`, and the status bar reads
`Engine running · 0 connected`. If it says the engine is stopped, see
[Troubleshooting](troubleshooting.md).

## Light and dark

dbdesk follows your system's light or dark setting. To choose one yourself, open
**View → Theme** and pick **Match system**, **Light** or **Dark**. The choice is remembered.

## The layout

Every panel is a tab you can drag: onto another panel's tab bar to join it, or onto the edge of
a panel to split it. Drag the gaps between panels to resize them. dbdesk remembers the layout
and the window size when you quit. **View → Reset layout** puts everything back where it
started.
