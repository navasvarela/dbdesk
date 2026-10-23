#!/usr/bin/env bash
# Builds the macOS disk image: dbdesk.app next to a shortcut to /Applications, compressed.
#
#   packaging/macos-dmg.sh <path/to/dbdesk.app> <output.dmg>
#
# Why not cargo-packager's own `dmg` format: it runs create-dmg, which attaches a read-write
# image, styles its Finder window, and converts it. The convert step failed with "Resource
# temporarily unavailable" on macOS 26 even outside any sandbox (R1, 2026-10-23), and the same
# sequence is a known source of flaky failures on GitHub's macOS runners. A plain
# `hdiutil create` from a folder has none of those steps; what is lost is only the custom
# window layout, while the drag-to-Applications shortcut stays.
set -euo pipefail

app=${1:?usage: macos-dmg.sh <path/to/dbdesk.app> <output.dmg>}
dmg=${2:?usage: macos-dmg.sh <path/to/dbdesk.app> <output.dmg>}

staging=$(mktemp -d)
trap 'rm -rf "${staging:?}"' EXIT

cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -quiet -volname dbdesk -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$dmg"
echo "$dmg"
