# Tasks

## 1. Crate scaffolding

- [x] 1.1 Create `crates/lua-language-service` with `Cargo.toml`
  (workspace member via `crates/*` glob, `emmylua_code_analysis = "0.25.1"`
  dependency, gpui-kit for the view) and an empty `src/lib.rs`; verify
  `cargo build -p lua-language-service` succeeds and the crate appears
  in `cargo metadata`
- [x] 1.2 Define the shell's public types (`SymbolHit`, `LuaDiagnostic`
  with shell severity enum, `IndexStatus`) in `src/lib.rs` (or a
  `types` module); verify `cargo doc -p lua-language-service --no-deps`
  shows no backend crate types in any public signature (grep the rustdoc
  output for `emmylua`/`lsp_types`)

## 2. Service core (roots, worker, queries)

- [x] 2.1 Implement the process-level state holder (OnceLock + RwLock,
  design D2) and `set_roots(Vec<PathBuf>)` full-replace semantics; unit
  test: inject `/a`, re-inject `/b`, assert query surface no longer
  sees `/a`, and empty injection clears everything (spec: Root
  injection replaces the workspace set)
- [x] 2.2 Implement the background worker (single thread + channel,
  generation counter, abort flag; design D5) that performs indexing on
  `set_roots`; unit test: injecting a fixture workspace updates
  `IndexStatus` (file count, elapsed, Idle/InProgress) without the
  caller performing the index work synchronously
- [x] 2.3 Implement `search_symbols(query)` over the backend
  declaration index (linear scan, exact/prefix/substring ordering;
  design D4); unit test: fixture workspace with `on_ready` returns a hit
  with name/kind/file/line/col for prefix `on_`, and empty workspace
  returns `[]` (spec: Symbol search answers by name)
- [x] 2.4 Implement `diagnostics_all()` with cancellation wired to the
  abort flag; unit test: fixture workspace with a syntax error yields a
  `LuaDiagnostic` with severity+message, empty workspace yields `[]`, and
  a new `set_roots` during a sweep aborts the stale run (spec:
  Workspace diagnostics; Analysis runs off the UI thread)
- [x] 2.5 Add a test asserting query-during-indexing answers from
  partial state rather than blocking (verify: test completes under a
  short timeout while a large fixture index is in progress)

## 3. Debug section (view + wiring)

- [x] 3.1 Implement `LuaIndexView` entity in the crate: root injection
  control, `IndexStatus` line, live search box with hit list,
  diagnostics table (spec: lua-index section requirements); unit test
  the render rows against injected fixtures (project test pattern:
  `ElementSnapshot`/row assertions as in `panel_tests.rs`)
- [x] 3.2 Register the section: factory adapter in
  `src/debug/mod.rs::register_builtin()` (id `lua-index`); verify with
  the existing debug-section test pattern that the section appears in
  `sections()` in debug builds and is absent from the registry in
  `cargo test --release`-gated tests (spec: Section appears only in
  debug builds)
- [x] 3.3 Verify headless open of the section (pattern:
  `src/debug/term_rate_tests.rs`) dedups by id, shows an empty hit list
  before injection, and populates hits/diagnostics after a fixture
  injection

- [x] 3.4 Browse entry point in the root injection control: system
  directory dialog (rfd, solution_config pattern) fills the root input
  and injects on confirm; cancel writes nothing; render assertion for
  the control (spec: Lua index section offers a system folder picker;
  dialog body is build glue, not touched headless)

## 4. Lua viewer handler (coloring)

- [x] 4.1 Add `LuaViewerHandler` claiming the `lua` extension in
  `src/property_detail_manager/` (reuse `EditorPanel` read-only path)
  and register it in `HANDLERS`; add `"lua" => "lua"` to
  `language_for()`; verify with a handler test that a `.lua` object
  opens a read-only tab titled with the file name (spec: Read-only Lua
  viewer handler)
- [x] 4.2 Verify Lua syntax highlighting is applied (syntect Lua
  syntax present) and dedup by path re-selects rather than reopens

## 5. Integration

- [x] 5.1 Full regression: `cargo test` (workspace) passes; confirm
  release build compiles the `lua-language-service` library but drops
  the debug section (spec: no lua-index section in release)
- [x] 5.2 Manual smoke (debug build): open Debug > lua-index, inject a
  real Lua directory, see file count/elapsed, type a query, see hits,
  see diagnostics populate while controls stay responsive
