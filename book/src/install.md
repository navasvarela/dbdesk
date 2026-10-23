# Install and first launch

Download dbdesk from the
[releases page](https://github.com/navasvarela/dbdesk/releases): pick the file for your system
from the latest release's **Assets**. Until the first release is published, the page is empty;
build from source instead (see the end of this page).

| System | File |
| --- | --- |
| macOS 11 or later, Apple silicon or Intel | `dbdesk-<version>-macos-universal.dmg` |
| Linux x86_64, most distributions | `dbdesk-<version>-x86_64.AppImage` |
| Debian, Ubuntu and derivatives | `dbdesk_<version>_amd64.deb` |

Windows builds are on the way: they're built for every release but only offered once they've
been tried on a real Windows machine.

## macOS

1. Open the `.dmg` and drag **dbdesk** onto **Applications**.
2. Open dbdesk from Applications. These builds aren't signed by Apple yet, so the first launch
   is blocked with "Apple could not verify dbdesk".
3. Open **System Settings › Privacy & Security**, scroll down to the message about dbdesk, and
   click **Open Anyway**. macOS asks once more; confirm, and it opens normally from then on.

After each update, macOS asks again for access to each password dbdesk saved in the Keychain.
Choose **Always Allow**. This happens because the app's signature changes with every unsigned
build; signed builds, planned for a later release, will remove both steps.

## Linux

**AppImage**, on any distribution:

```bash
chmod +x dbdesk-*-x86_64.AppImage
./dbdesk-*-x86_64.AppImage
```

**.deb**, on Debian, Ubuntu and derivatives. This also adds dbdesk to the application menu:

```bash
sudo apt install ./dbdesk_*_amd64.deb
```

Saved passwords go to the Secret Service (GNOME Keyring, KWallet), which desktop environments
already run. On a bare window manager without one, see [Troubleshooting](troubleshooting.md).

## Check the download and the version

Each release has a `SHA256SUMS` file. In the folder with your download:

```bash
shasum -a 256 -c SHA256SUMS --ignore-missing
```

**Help › About dbdesk** shows the version and the commit it was built from, and **Copy
version** copies them for a bug report. From a terminal, `dbdesk --version` prints the same.

## Updates

When dbdesk starts, it asks GitHub whether a newer release is out. If one is, the status bar
says so, for example **dbdesk 0.2.0 is available**; click it, then **Open release page** to
download the new version and install it as above. **Help › Check for updates…** asks at any
time.

dbdesk doesn't update itself yet, and the check sends nothing about you or your data: it reads
the public list of releases. To stop the check at start-up, untick **Check for a new release
when dbdesk starts** in **View → Settings…**.

## Build from source

For contributors, or to run the latest unreleased code. You need:

- **Rust 1.95 or newer**, installed with [rustup](https://rustup.rs). Check with
  `rustc --version`.
- **Git**, to fetch the source.
- On **Linux**, the development headers for the windowing system. On Debian or Ubuntu:

  ```bash
  sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
  ```

Then:

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

The log console under the Welcome tab shows `engine started`, and the status bar reads
`Engine running · 0 connected`. If it says the engine is stopped, see
[Troubleshooting](troubleshooting.md).

## Light and dark

dbdesk follows your system's light or dark setting. To choose one yourself, open
**View → Theme** and pick **Match system**, **Light** or **Dark**. The choice is remembered.

## The layout

dbdesk opens maximized the first time; after that it remembers the window's size and position.

Every panel is a tab you can drag: onto another panel's tab bar to join it, or onto the edge of
a panel to split it. Drag the gaps between panels to resize them. dbdesk remembers the layout
and the window size when you quit. **View → Reset layout** puts everything back where it
started.
