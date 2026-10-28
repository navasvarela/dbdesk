#!/usr/bin/env bash
# Rewrites DuckDB-LICENSE and DuckDB-THIRD-PARTY.txt beside this script from DuckDB's
# repository at a release tag. The packages ship both (crates/dbrover-app/Cargo.toml's
# resources). Run it when upgrading DuckDB (see docs/duckdb-prebuilt.md):
#
#   packaging/duckdb/update-licenses.sh 1.5.6
#
# The notices cover every third-party component in DuckDB's source tree that can end up in its
# library, whether DbRover compiles it or links DuckDB's prebuilt one: everything under
# third_party/, the ICU extension's copy of ICU, and the ALP compression algorithms. Test data,
# the TPC-H generator, the shell's line editor and the Swift bindings never reach DbRover.
set -euo pipefail

[[ $# -eq 1 ]] || { echo "usage: $0 <duckdb-version>" >&2; exit 2; }
version=$1
here=$(cd "$(dirname "$0")" && pwd)
raw="https://raw.githubusercontent.com/duckdb/duckdb/v$version"

paths=$(curl -fsSL "https://api.github.com/repos/duckdb/duckdb/git/trees/v$version?recursive=1" \
    | jq -r '.tree[] | select(.type == "blob") | .path
        | select(test("(^|/)(LICEN[CS]E|NOTICES?)[^/]*$"; "i"))
        | select(test("^(third_party/|extension/icu/third_party/|src/include/duckdb/storage/compression/)"))' \
    | sort)
[[ -n "$paths" ]] || { echo "no licence files found for v$version" >&2; exit 1; }

curl -fsSL -o "$here/DuckDB-LICENSE" "$raw/LICENSE"

{
    echo "Third-party software in DuckDB $version"
    echo
    echo "DuckDB, under the MIT licence in DuckDB-LICENSE beside this file, includes the"
    echo "components below, each under its own licence. Gathered from"
    echo "https://github.com/duckdb/duckdb/tree/v$version by packaging/duckdb/update-licenses.sh."
    for path in $paths; do
        component=${path%/*}
        echo
        echo "================================================================================"
        echo "${component#third_party/} ($path)"
        echo "================================================================================"
        echo
        curl -fsSL "$raw/$path"
    done
} > "$here/DuckDB-THIRD-PARTY.txt"
