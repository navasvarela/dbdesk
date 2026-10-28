#!/usr/bin/env bash
# Assembles the website into one folder: the landing page from site/ with its pictures and
# clips (site/media, made by site/clips.sh), the brand assets, fonts and database logos it uses
# (copied, so assets/ stays their only home), and the user manual built by mdBook under
# manual/. The Pages workflow publishes what this produces.
#
#   site/build.sh [OUTPUT_DIR]     # default: target/site
#
# Needs mdbook 0.5.4 and mdbook-linkcheck2 0.13.0 (see docs/testing.md). To preview:
#   site/build.sh && python3 -m http.server -d target/site
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-${root}/target/site}"

rm -rf "${out}"
mkdir -p "${out}/assets/brand" "${out}/assets/fonts" "${out}/assets/databases"

# The page names the version and links to its files, written in from the workspace's version
# rather than asked of GitHub when the page loads.
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "${root}/Cargo.toml" | head -n 1)"
if [ -z "${version}" ]; then
    echo "no workspace version in Cargo.toml" >&2
    exit 1
fi
sed "s/{{version}}/${version}/g" "${root}/site/index.html" > "${out}/index.html"
cp "${root}/site/style.css" "${out}/"
cp -R "${root}/site/media" "${out}/media"
cp "${root}/assets/brand/logo.svg" "${root}/assets/brand/social-preview.png" \
    "${out}/assets/brand/"
cp "${root}/assets/fonts/inter/Inter-Regular.ttf" "${root}/assets/fonts/inter/Inter-SemiBold.ttf" \
    "${root}/assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf" "${out}/assets/fonts/"
cp "${root}/assets/fonts/inter/LICENSE.txt" "${out}/assets/fonts/Inter-LICENSE.txt"
cp "${root}/assets/fonts/jetbrains-mono/OFL.txt" "${out}/assets/fonts/JetBrainsMono-OFL.txt"
cp "${root}/assets/icons/databases/"*.svg "${out}/assets/databases/"

# The linkcheck2 backend fails the build on a broken link inside the manual.
mdbook build "${root}/book"
cp -R "${root}/book/book/html" "${out}/manual"

echo "site written to ${out}"
