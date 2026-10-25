# Contributing

## Commit messages

Commits follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):

```text
<type>(<scope>): <summary>

<body>

<footer>
```

- **type**, one of:

  | Type | For |
  | --- | --- |
  | `feat` | Something a user can now do or see |
  | `fix` | A bug fix |
  | `docs` | Documentation only: `docs/`, the manual in `book/`, READMEs, comments |
  | `refactor` | A code change that neither adds a feature nor fixes a bug |
  | `perf` | A change that makes something faster or lighter |
  | `test` | Adding or correcting tests, with no change to the code under test |
  | `build` | Cargo manifests, dependencies, `assets/` export scripts |
  | `ci` | `.github/workflows` |
  | `style` | Formatting only (`cargo fmt`), no change in behaviour |
  | `chore` | Anything else that doesn't touch the shipped app, such as `.gitignore` |
  | `revert` | Reverting an earlier commit |

- **scope** (optional) names the part of the codebase, so `git log` can be skimmed by area.
  Use the crate without its `dbdesk-` prefix (`core`, `sql`, `engine`, `store`, `app`,
  `driver-postgres`, `driver-sqlite`), or a narrower area of the app (`explorer`, `console`,
  `grid`, `theme`, `keymap`), or `book`, `brand`, `roadmap`. Leave it out when a change spans
  several areas.
- **summary**: imperative mood ("add", not "added" or "adds"), lower case, no full stop, and
  the whole first line within 72 characters. It says what changes for someone reading the
  log, not which files were edited.
- **body** (optional, after a blank line): why the change was made, and anything a reviewer
  would otherwise have to dig for. Wrapped at 72 characters.
- **Breaking changes** — anything that makes an existing `data_sources.toml`, `keymap.toml` or
  other saved file stop loading, or changes a documented default — get a `!` after the type or
  scope, and a `BREAKING CHANGE:` footer saying what to do about it.

Examples:

```text
feat(explorer): show a count badge on loaded folders
fix(driver-postgres): report the error position for multi-byte SQL
docs(book): describe saved consoles and history
build: bump egui and egui_dock together to 0.37
refactor(console)!: store consoles under their data source's id

BREAKING CHANGE: consoles saved before this change are not found any more;
move them from consoles/<name>.sql to consoles/<data-source-id>/<name>.sql.
```

## Branches

Branch names use the same types, then a short description in kebab case:

```text
<type>/<description>
```

- Lower case, words separated by hyphens, no more than five or six words.
- Add the milestone or track item when there is one, so related branches sort together:
  `feat/m5-ssh-tunnels`, `docs/d4-website`.
- Examples: `feat/query-history`, `fix/explorer-filter-system-objects`,
  `docs/roadmap-and-conventions`, `ci/book-link-check`.

A branch that mixes several types takes the type of its main change: a feature with its tests
and docs is `feat/…`.

## Pull requests

- The title follows the commit format, because it becomes the merge commit's summary:
  `feat(console): save consoles between runs`.
- Work that splits into layers that build on each other (a dialect, then a driver, then the
  UI) is a stack: see [Stacked pull requests](#stacked-pull-requests). Independent changes are
  separate pull requests against `main`.
- `just lint` and `just test` pass before review. They run `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` and
  `cargo test --workspace`. See [testing](docs/testing.md) for the PostgreSQL tests.

Commits and branches from before this convention (`M4: …`, `m4-daily-driver`) keep their
names; history isn't rewritten.

## Stacked pull requests

A change that is easier to review in layers lands as a GitHub stack, with the
[`gh stack`](https://gh.io/stacks) extension (`gh extension install github/gh-stack`). Each
layer is a small pull request that GitHub retargets as the ones below it merge, with `main`'s
required checks on every layer.

```bash
gh stack init feat/db3-duckdb-dialect
```

Commit the first layer, then start the next one on top of it:

```bash
gh stack add feat/db3-duckdb-driver
```

When every layer has commits, push them all and open one pull request per layer:

```bash
gh stack submit --auto --open
```

`--auto` titles each pull request after its branch, so give each one a Conventional Commits
title and a description with `gh pr edit`.

- **Keeping up with `main`:** `gh stack rebase`, then `gh stack push`. This rewrites the stack's
  own branches (`--force-with-lease`). Never rebase or force-push `main`, nor a branch outside a
  stack. Where force-pushing isn't possible, merging `origin/main` into the branch works too;
  the squash merge flattens it.
- **A fix in a lower layer:** commit it there, then `gh stack rebase --no-trunk` and
  `gh stack push`.
- **Merging:** GitHub doesn't queue auto-merge for stacked pull requests, and a plain
  `gh pr merge` is refused. Once a layer's checks are green, merge it, and everything below it,
  with:

  ```bash
  gh stack merge <pr-number> --squash --yes
  ```

  Each layer becomes its own squash commit on `main`, titled with its pull request's title. The
  layers above it are retargeted to `main` automatically. The merge is all or nothing: if one
  layer conflicts with `main`, nothing merges until it is brought up to date.

## Adding a database

Support for a new database engine follows the steps in
[Adding a database](docs/adding-a-database.md): the dialect, the driver crate, the wiring, the
app, the test server and the documentation, one pull request each.
