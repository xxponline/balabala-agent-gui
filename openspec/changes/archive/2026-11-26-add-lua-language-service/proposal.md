# Proposal

## Why

The workbench has no Lua support at all: opened `.lua` files render as
plain text, and neither the user nor the agent can look up symbols in the
Lua sources that solutions/projects will carry. We need a single
authoritative Lua indexing component that (a) powers a future
Rider-style symbol search box, (b) exposes symbol search and diagnostics
to the agent over MCP, and (c) is reachable today for validation — before
those consumers exist, a debug panel view is the only way to exercise the
whole chain end to end.

## What Changes

- New standalone crate `crates/lua-language-service`: a thin shell over
  `emmylua_code_analysis` (pinned as the current backend, designed to be
  replaceable) that owns the process-level Lua workspace state:
  - `set_roots(Vec<PathBuf>)` — full-replace root injection (DebugView
    today, solution-service later)
  - `search_symbols(query)` — fuzzy/prefix symbol lookup (name →
    file/line/col/kind), assembled by the shell from the backend's
    declaration index; no backend type leaks through the API
  - `diagnostics_all()` — workspace-wide lint diagnostics (the backend's
    diagnostic rule engine), backend-neutral types only
  - Indexing and diagnostics run on a background thread; queries take a
    read lock — the UI thread never blocks on analysis
- New debug section `lua-index` in the Debug panel (debug builds only):
  root path injection control, index status (file count / elapsed /
  state), a mini search box showing live `SymbolHit` results, and a
  diagnostics table. Registered through the existing debug section
  entry point via a small adapter in the main crate; the view entity
  itself lives in the new crate.
- New read-only `.lua` viewer handler in the center tab system with Lua
  syntax highlighting (syntect), alongside the existing Rust and
  markdown handlers.

Out of scope: code completion (explicitly deferred), hover tips, the
workbench symbol search box, the MCP server itself, solution-service
root injection, and a `file_list` query (interface slot only).

## Capabilities

### New Capabilities

- `lua-language-service`: The Lua indexing shell — root injection with
  full-replace semantics, symbol search, workspace diagnostics,
  background indexing off the UI thread, and backend neutrality
  (emmylua types never surface in the public API).

### Modified Capabilities

- `debug-console`: New requirement for the `lua-index` debug section —
  its controls (root injection, search test box, diagnostics table),
  data contract, and visibility behavior.
- `center-tabs`: New requirement for a read-only Lua viewer handler —
  `.lua` files claim into a read-only editor tab with Lua syntax
  highlighting, mirroring the Rust handler contract.

## Impact

- New crate `crates/lua-language-service` (workspace member via
  `crates/*` glob) + root `Cargo.toml` path dependency; new third-party
  dependency `emmylua_code_analysis` (and its tree)
- Main crate: `src/debug/mod.rs` `register_builtin()` gains the section
  adapter; `src/property_detail_manager/` gains a `.lua` handler and a
  `language_for` mapping line
- The `crates/lua-language-service` DebugView must register only in
  debug builds, consistent with `debug-console`'s debug-only registry
  requirement; the service library itself compiles in both profiles
  (release consumers: future MCP, search box)
- No changes to service-routing/service-lifecycle: v1 does not go on
  the bus
