## Install

| Platform | Download |
| --- | --- |
| macOS 11 or later, Apple silicon or Intel | `dbdesk-<version>-macos-universal.dmg` |
| Linux x86_64, most distributions | `dbdesk-<version>-x86_64.AppImage` |
| Debian, Ubuntu and derivatives | `dbdesk_<version>_amd64.deb` |

**macOS:** open the disk image and drag dbdesk to Applications. These builds aren't signed by
Apple yet, so the first launch is blocked with "Apple could not verify dbdesk". Open **System
Settings › Privacy & Security**, scroll down, and click **Open Anyway**. After each update,
macOS also asks once more for access to every saved password; choose **Always Allow**. Signed
builds, planned for a later release, will remove both.

**Linux:** make the AppImage executable (`chmod +x dbdesk-*.AppImage`) and run it, or install the
`.deb` with `sudo apt install ./dbdesk_*_amd64.deb`. Saved passwords need a Secret Service such
as GNOME Keyring or KWallet, which desktop environments run already.

**Checking a download:** compare it with `SHA256SUMS`, for example
`shasum -a 256 -c SHA256SUMS --ignore-missing`.

The full guide is in the [user manual](https://github.com/navasvarela/dbdesk/blob/main/book/src/install.md).
