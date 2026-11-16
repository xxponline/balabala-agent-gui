# Tasks

## 1. Font resolution and startup verification

- [x] 1.1 Replace the hard-coded `GRID_FONT_FAMILY` with an ordered fallback
  list (`Menlo`, `Cascadia Mono`, `Consolas`, `Ubuntu Sans Mono`,
  `DejaVu Sans Mono`) resolved once, and verify with a unit test that the
  resolved family is used by both `grid_metrics` and the grid row rendering
  (test: assert a single resolution function is the only font source for the
  pane)
- [x] 1.2 Add the startup monospace check (advances of `M`, `a`, space, `—`
  within 0.5px tolerance at 13px) with an `eprintln!` diagnostic naming the
  resolved font and measured advances, and verify with a test that a
  non-monospace advance spread triggers the diagnostic output
- [x] 1.3 Confirm on the Windows host that `cargo run` logs no
  non-monospace diagnostic (fallback list resolved to a mono font) and that
  the log's `cell_w` is consistent with a 0.6em-class mono advance

## 2. Drift-hardened cell rendering

- [x] 2.1 Render each cell inside a merged segment as a fixed-width inline
  chunk (width = cell_w, overflow hidden) so glyph positions derive from cell
  index, and verify with a painted-width test that `position(N) == N ×
  cell_w` (±0.5px) for a row mixing ASCII, spaces, and box-drawing chars
- [x] 2.2 Keep/simplify the existing wide-char segment workaround under the
  new chunk layout, and verify the existing CJK alignment and wide-
  continuation scenarios still pass (`cargo test pane`)
- [x] 2.3 Verify cursor alignment by rendering a row with typed text and a
  cursor cell: the inverted block starts exactly one cell after the last
  character (extends the existing cursor scenario)

## 3. Real-font regression coverage

- [x] 3.1 Add a test that builds a window backed by the host's real platform
  text system (not `NoopTextSystem`) and asserts equal advances for `M`, `a`,
  space, and `—`, failing with the resolved font name when spread exceeds
  tolerance; verify it passes on the Windows host
- [x] 3.2 Re-scope the existing mock-based cell-width tests with comments
  marking them mock-environment tests (no behavior change), and verify
  `cargo test` still passes for the full suite

## 4. File-tree view epoch cache

- [x] 4.1 Add `view_generation` + cached `SourceView` to the file-tree source,
  bumped on scan-land, root change, rules change, and explicit refresh; verify
  with a test that two `view()` calls with an unchanged epoch return the
  cached view without re-filtering (counter/spy on the filter path)
- [x] 4.2 Remove the deep `last_synced != view` comparison from
  `on_data_changed` in favor of epoch comparison, and verify with a test that
  a data-change notification on an unchanged epoch performs no tree
  traversal (spy assertion), covering the large-workspace keystroke scenario
- [x] 4.3 Verify end-to-end that a filter-rule edit followed by refresh still
  re-applies rules (existing refresh scenarios keep passing in `cargo test`)

## 5. In-flight scan guard

- [x] 5.1 Track the root being scanned separately from the landed view root
  and make `refresh()` idempotent for an unchanged root with a scan in
  flight; verify with a test that repeated `on_data_changed` calls during a
  slow scan start exactly one scan
- [x] 5.2 Verify the scan-completes-while-typing scenario: simulate frames
  arriving during an in-flight scan and assert the scan lands (single
  completion) rather than restarting

## 6. Visibility gate

- [x] 6.1 Make `on_data_changed` return early when the right-dock tree panel
  is not rendered, and verify with a test that notifications while hidden do
  no source work
- [x] 6.2 Add the lazy epoch check on the panel's render path so reopening
  the dock shows content current as of the last invalidation, and verify the
  reopen-after-hidden scenario (no rescan, fresh content if epoch moved)

## 7. Integration verification

- [x] 7.1 Run the full suite (`cargo test`) and confirm all existing
  scenarios plus the new tests pass
- [x] 7.2 Manual check on Windows: open `D:\UnrealEngine` workspace, type in
  the console — keystrokes echo without perceptible lag, cursor block sits
  flush against typed text, input-box borders form a rectangle, and the
  initial file-tree scan completes while typing

## Workflow follow-up

- Archive the change after the project's review requirements are satisfied.
- Verify the archived result.
