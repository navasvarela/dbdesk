#!/usr/bin/env bash
# Regenerates the PNG and .icns exports in assets/brand from the SVG sources.
#
# Needs resvg (`cargo install resvg --version 0.48.1 --locked`) and, for the .icns, macOS's
# iconutil. The version is pinned because a different rasteriser version can move edge pixels,
# which would churn every PNG in a diff with no visible change.
set -euo pipefail

RESVG_VERSION="0.48.1"
cd "$(dirname "$0")"

if ! command -v resvg >/dev/null; then
    echo "resvg not found: cargo install resvg --version ${RESVG_VERSION} --locked" >&2
    exit 1
fi
if [[ "$(resvg --version)" != "${RESVG_VERSION}" ]]; then
    echo "expected resvg ${RESVG_VERSION}, found $(resvg --version)" >&2
    exit 1
fi

# The app icon (the mark on a tile), for the window icon and the macOS bundle.
for size in 16 32 64 128 256 512 1024; do
    resvg --width "${size}" --height "${size}" icon.svg "icon-${size}.png"
done

# The bare mark, for documentation and anywhere the tile would look heavy.
for size in 16 32 64 256 512; do
    resvg --width "${size}" --height "${size}" logo.svg "logo-${size}.png"
done

# The repository's social preview (GitHub shows it on links to the repository). Its text is
# set in the bundled Inter, so it renders the same on every machine.
resvg --width 1280 --height 640 --skip-system-fonts \
    --use-font-file ../fonts/inter/Inter-Regular.ttf \
    --use-font-file ../fonts/inter/Inter-SemiBold.ttf \
    social-preview.svg social-preview.png

if ! command -v iconutil >/dev/null; then
    echo "iconutil not found (it ships with macOS); skipping dbrover.icns" >&2
    exit 0
fi

iconset="$(mktemp -d)/dbrover.iconset"
mkdir "${iconset}"
trap 'rm -rf "$(dirname "${iconset}")"' EXIT
# iconutil expects each point size at 1x and 2x under these exact names.
for points in 16 32 128 256 512; do
    cp "icon-${points}.png" "${iconset}/icon_${points}x${points}.png"
    cp "icon-$((points * 2)).png" "${iconset}/icon_${points}x${points}@2x.png"
done
iconutil --convert icns --output dbrover.icns "${iconset}"
