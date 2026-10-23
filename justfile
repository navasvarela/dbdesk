# Everyday commands. Run `just` to list them.

# List the recipes.
default:
    @just --list

# Check formatting and run clippy, as CI does.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format every crate.
format:
    cargo fmt --all

# Run the app; arguments go to dbdesk, e.g. `just run --version`.
run *args:
    cargo run -p dbdesk-app -- {{ args }}

# Tests that need a database server are skipped unless its DBDESK_TEST_*_URL is set; after
# `docker compose up -d --wait`, `set -a; . ./test-servers.env; set +a` sets them all (see the
# README). Not loaded here, so `just test` still works with no server running.
# Arguments replace `--workspace` rather than add to it, because cargo ignores `-p` when
# `--workspace` is also given.

# Run every test, or pass cargo test arguments, e.g. `just test -p dbdesk-sql`.
test *args:
    cargo test {{ if args == "" { "--workspace" } else { args } }}
