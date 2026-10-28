## Install

| Platform | Download |
| --- | --- |
| macOS 11 or later, Apple silicon or Intel | `dbrover-<version>-macos-universal.dmg` |
| Linux x86_64, most distributions | `dbrover-<version>-x86_64.AppImage` |
| Debian, Ubuntu and derivatives | `dbrover_<version>_amd64.deb` |

**macOS:** open the disk image and drag DbRover to Applications. These builds aren't signed by
Apple yet, so the first launch is blocked with "Apple could not verify DbRover". Open **System
Settings › Privacy & Security**, scroll down, and click **Open Anyway**. After each update,
macOS also asks once more for access to every saved password; choose **Always Allow**. Signed
builds, planned for a later release, will remove both.

**Linux:** make the AppImage executable (`chmod +x dbrover-*.AppImage`) and run it, or install the
`.deb` with `sudo apt install ./dbrover_*_amd64.deb`. Saved passwords need a Secret Service such
as GNOME Keyring or KWallet, which desktop environments run already.

**Checking a download:** compare it with `SHA256SUMS`, for example
`shasum -a 256 -c SHA256SUMS --ignore-missing`.

The full guide is in the [user manual](https://dbdesk.org/manual/install.html).
