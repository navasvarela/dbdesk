# 0005. Customisation through files and commands now, WebAssembly plugins later

- Status: Accepted
- Date: 2026-10-24

## Context

DbRover exists partly so its user can make it their own (see [Vision](../vision.md)). Until now
the vision listed "a plugin system loaded at runtime" as a non-goal: extension meant adding
crates to the workspace and rebuilding. That keeps the code simple, but it means every
customisation beyond keys and formatting needs a Rust change and a new build.

The question is how far DbRover should open itself up, and through what:
- files only;
- external programs;
- an embedded scripting language;
- native libraries;
- WebAssembly.

Each costs something different to build, to keep stable once others depend on it, and in what a
misbehaving extension can do.

## Decision

Three levels, built in this order:

1. **Declarative files (H1).** `actions.toml` (custom actions running templated SQL),
   `snippets.toml` and `theme.toml`, handled like `keymap.toml`: problems are listed, never
   fatal, and the files can be reloaded. Placeholders are quoted by the dialect, never pasted in
   as raw text.
2. **External commands (H2).** An action may run a program, given its context as arguments,
   environment variables and rows on stdin, with no shell, a timeout and capped output. Secrets
   are never passed. Only the user's own `actions.toml` may define one.
3. **WebAssembly plugins (H3), later.** They are sandboxed, use a versioned WIT interface over the
   same extension points, and get only the capabilities each plugin declares and the user grants.
   Querying a database reuses the MCP plan's permission model. Work starts only once at least
   three real needs can't be met by levels 1 and 2.

Native shared libraries and an embedded scripting language are not used.

The vision's non-goal changes from "a plugin system loaded at runtime" to "a plugin system that
loads native code".

The full plan is in [Hackability](../hackability.md).

## Consequences

- **Most customisation becomes possible without a rebuild,** and stays as plain files the user
  can version and share, which fits the vision's "plain config files".
- **Command actions make DbRover as extensible as the user's shell,** in any language, at the
  cost of a small engine feature, with no plugin interface to keep stable.
- **The extension points are named now** (actions, viewers, exporters, lint rules, data source
  providers). H1 and H2 use the same names as H3 later would, so plugins slot in without
  reshaping the app.
- **Plugins never draw their own UI.** They contribute entries to lists the app draws, which
  limits them, but keeps egui's immediate-mode code inside the app and the interface small.
- **Accepted:** until H3, some extensions (a lint rule, a geometry viewer) still need a change to
  DbRover itself.
- **Accepted:** command actions run with the user's own rights. That is safe because only the
  user's own file can define them, and it is why shared or imported configuration never carries
  actions.

## Alternatives considered

- **Keep the non-goal: extension only by adding crates.** Simplest, but every personal tweak
  needs Rust and a rebuild. That defeats the personalisation goal for anything beyond keys.
- **Native plugins (`dylib`, `abi_stable`).** The fastest at runtime and able to do anything, but
  Rust has no stable ABI between compiler versions, so plugins break with every DbRover update
  unless a C-shaped interface is maintained. Loading them is `unsafe`, which the workspace
  forbids, and a crashing plugin takes the app down.
- **An embedded scripting language (Lua through `mlua`, or Rhai).** Quick to embed and good for
  small scripts. It is a second API to design and keep stable, but in one language only. Lua
  needs its standard library pruned to be sandboxed. Rhai is safe, but few people write it.
  WebAssembly gives the same with a real sandbox and any language.
- **WebAssembly plugins now.** The right end point, but the most work: an interface, capability
  grants, installing, and keeping it all stable. Doing it before files and commands have shown
  which extension points matter risks designing the wrong interface.
