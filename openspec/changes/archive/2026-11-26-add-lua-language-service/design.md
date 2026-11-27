# Design

## Context

The repo has no Lua support today. The debug panel (`src/debug/`) owns
an in-process section registry (`register` / `sections` /
`register_builtin`) gated by `#![cfg(debug_assertions)]`, whose factory
signature is `fn(&'static str, &mut Context<TerminalApp>) -> AnyView` —
defined in the binary crate, so an external crate cannot implement it
directly. The precedent for external views is `AgentTerminal`: the view
entity lives in `crates/agent-service`, the main crate only wires it.
The center-tab system dispatches opened files through a static
`HANDLERS` table (`src/property_detail_manager/mod.rs`) with `rs` and
`md` handlers, and `language_for()` maps extensions to syntect language
names. See proposal.md for motivation; specs/lua-language-service,
specs/debug-console, and specs/center-tabs carry the contracts.

Backend facts (verified against `emmylua_code_analysis` 0.25.1): it
exposes an `EmmyLuaAnalysis` index (workspace roots, per-file VFS,
declaration/module/reference indexes) and per-file diagnostics via
`diagnose_file(file_id, CancellationToken)`; it has **no** workspace
symbol query and **no** semantic-token API; it is `unsafe impl Send +
Sync` upstream. Symbol search must be assembled by the shell from the
declaration index. Syntax highlighting is syntect's job, not the
backend's.

## Goals / Non-Goals

**Goals:**
- One authoritative Lua index in-process, reachable through a
  backend-neutral API (`set_roots`, `search_symbols`,
  `diagnostics_all`)
- End-to-end validation today via a `lua-index` debug section
- UI thread never blocks on analysis
- Backend replaceable without touching callers

**Non-Goals:**
- Code completion and hover (explicitly deferred by the user)
- The workbench symbol search box, MCP server, solution-service
  injection (future consumers of the same API)
- Going on the bus (no kameo actor, no contract in v1)
- Semantic (type-aware) highlighting — syntect lexical highlighting only

## Decisions

**D1 — Crate shape: library + view in one crate, wiring in the binary.**
`crates/lua-language-service` holds the service state, the query
implementation, and the `LuaIndexView` entity. The binary adds a
3-line factory in `register_builtin()` that does
`cx.new(|cx| LuaIndexView::new(cx)).into()` and registers it.
Alternative rejected: putting the view in `src/debug/` would make the
crate GUI-agnostic but split the feature across two places; the
`AgentTerminal` precedent shows the harness accepts crate-owned views.
The view references `TerminalApp` only through `WeakEntity` passed in
at construction, same as `ThreadsPipelineSection`.

**D2 — State ownership: process-level singleton inside the crate.**
`static` holder (`OnceLock<RwLock<ServiceState>>`) owns roots +
backend instance + index status. `set_roots` is full-replace: clear old
roots, inject new, trigger background reindex. This keeps root state
alive when the debug tab closes and lets future solution-service
injection call the same entry point with zero view changes. Alternative
rejected: per-view state (dies with the tab, duplicate indexing);
bus service (right eventual shape if/when a third consumer appears,
premature now — would force a contract design before any second
consumer exists).

**D3 — Backend behind a private trait; own types in public API.**
A crate-private `trait Backend` with the two operations the shell needs
(`search(query) -> Vec<Hit>`, `diagnostics() -> Vec<Diag>`), implemented
by an `EmmyluaBackend`. Public structs (`SymbolHit`, `LuaDiagnostic`,
`IndexStatus`) are defined in the shell. `lsp_types::Diagnostic` never
crosses the boundary — severity is mapped to a shell enum. The trait is
private (not `pub`) in v1: no second implementation exists, and a public
trait is a compatibility promise; making it public later is a minor
change. Alternative considered: pub trait from day one — rejected as
premature API surface.

**D4 — Symbol search: shell-side linear scan, cache deferred.**
`search_symbols` iterates the backend's per-file declaration trees,
collects name/kind/file/line/col, and matches (exact + prefix first,
then substring). O(total decls) per query. This is fine for debug-view
and MCP usage; the future keystroke-driven workbench search box may
need a name→hits cache invalidated on reindex — the API shape
(`search(&str)`) does not change when that lands, so it is deferred.
The backend exposes no fuzzy query, so any cache would be ours anyway.

**D5 — Threading: one background worker, generation counter.**
`set_roots` and diagnostics requests are queued to a single worker
thread (std thread + channel; no tokio requirement in the shell).
The worker owns the `&mut` backend operations; readers take the
`RwLock` read side. A generation counter lets the view distinguish
"results from the latest injection" from stale ones. Diagnostics get a
`CancellationToken`-equivalent (a simple AtomicBool flag per run) so a
new injection aborts an in-flight diagnostics sweep. Alternative
considered: `spawn_blocking` per request — rejected because indexing
must not run concurrently with itself and ordering must be preserved.

**D6 — Debug section wiring follows the existing registry exactly.**
The section id is `lua-index`. The factory adapter lives in
`src/debug/mod.rs::register_builtin()`. The whole registration stays
inside the `#![cfg(debug_assertions)]` module, so release builds lose
the section (and the factory), while the `lua-language-service`
library itself still compiles in release for future consumers. The
view's background work is tied to the entity (Task held in the field →
dropped on close), but the *service* state persists — closing the tab
stops the view, not the index.

**D7 — Lua viewer handler is a one-line mapping plus a handler struct.**
A `LuaViewerHandler` claiming `lua` extension reusing the existing
`EditorPanel` path (read-only `EditorState`), and `language_for()`
gains `"lua" => "lua"` (syntect ships a Lua syntax). No dependency on
`lua-language-service` — coloring is lexical and independent of the
index, so the viewer works even with no workspace injected.

## Risks / Trade-offs

- [Full `init_std_lib` + workspace index can take seconds on large
  workspaces] → runs on the worker thread; the view shows
  in-progress state; queries answer from partial state (spec: never
  block).
- [Linear scan search may be too slow for the future keystroke-driven
  search box] → deferred cache behind unchanged API (D4); acceptable
  now because only debug-view and MCP call it in v1.
- [Upstream `unsafe impl Send + Sync` could hide a soundness issue we
  inherit] → we only move the backend on our worker thread and read it
  under a lock; no cross-thread sharing outside the `RwLock`.
- [Backend swap later may not fit the `Backend` trait exactly (e.g., a
  server-based backend needs async)] → trait is private and small
  (two methods); reshaping it is a crate-internal change.
- [Two Lua features land in different specs (index vs viewer)] →
  intentional: coloring is syntect-only and must not depend on the
  index being present (D7).

## Open Questions

- None blocking. Deferred by agreement: completion/hover, MCP tool
  surface shape, workbench search box, solution-service injection,
  whether the singleton graduates to a bus service when a third
  consumer appears.
