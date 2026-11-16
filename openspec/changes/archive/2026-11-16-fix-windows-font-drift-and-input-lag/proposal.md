# Proposal

## Why

Running the app on Windows exposes two production bugs that the current test
suite structurally cannot see:

1. **Terminal cursor/border drift.** The grid's monospace contract (row width =
   columns × cell width) assumes every glyph advances by the measured cell
   width. The font family in use (`Menlo`) does not exist on Windows, so gpui
   falls back per glyph class to non-monospace fonts; the measured cell width
   (taken only from `M`) no longer matches actual advances of spaces,
   lowercase Latin, or box-drawing characters, so text drifts inside its
   fixed-width segments — the cursor block detaches from the typed text and
   box borders stop lining up across rows. gpui's test-support text system is
   a mock with uniform 0.6em advances, so every cell-width test stays green
   while real rendering is broken.

2. **Typing latency on large workspaces.** Every consumed terminal frame calls
   `notify_panels`, which unconditionally rebuilds the file-tree view —
   deep-cloning and deep-comparing the entire workspace tree on the UI thread.
   With a 510k-file workspace (`D:\UnrealEngine`) that is hundreds of
   milliseconds per keystroke (confirmed: a small workspace has no lag).
   Additionally, while the initial scan is still in flight, `view()` reports an
   empty root, so each frame treats the root as "changed" and restarts the
   full recursive scan — the scan can never finish while the user is typing.

Both defects are invisible to the current tests (mock font metrics; no large-
workspace fixture), so the fix must include tests that give these failure
modes visibility.

## What Changes

- **Font strategy for the terminal grid**: resolve a monospace font family
  that exists on the host platform via an explicit ordered fallback list (and
  optionally a bundled font), instead of assuming `Menlo`.
- **Startup metrics verification**: at startup (or first grid measurement),
  verify the resolved font is actually monospace across representative glyph
  classes (`M`, `a`, space, `─`) and surface a clear diagnostic if not, so a
  non-monospace fallback fails loudly rather than drifting silently.
- **Drift-hardened rendering**: keep glyph positioning inside fixed-width
  segments from accumulating drift even if a future font surprise slips
  through, so the cursor block and box borders always land on cell boundaries.
- **Real-metrics regression tests**: add tests that measure advance widths
  through a real text system (not the mock) and fail when the resolved font is
  non-monospace; keep existing mock-based tests but stop treating them as
  evidence of real rendering correctness.
- **File-tree update cost**: cache the tree view behind a generation/epoch
  (scan landing or rules change invalidates it) so a data-change notification
  becomes O(1) instead of a full deep clone + deep compare per frame.
- **Scan restart loop fix**: an in-flight scan for the current root SHALL NOT
  be restarted by subsequent data-change notifications; rescan triggers remain
  first load, root change, and explicit refresh (as already specified).
- **Panel visibility**: skip tree view work when the right-dock tree panel is
  not rendered (work is currently done even with the dock closed).

Out of scope: UnrealEngine-specific scan exclusions (nice-to-have; the
generation cache removes the per-frame cost regardless), scan throughput
optimization itself, and any change to filter-rule semantics.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `agent-shell`: the terminal output rendering requirement gains explicit
  cross-platform monospace resolution + startup metrics verification + a
  no-drift guarantee (cursor and borders land on cell boundaries on every
  supported platform, verified with real font metrics).
- `file-tree`: the refresh/update requirements gain an explicit per-frame cost
  bound (data-change notifications do not rebuild or deep-compare the tree)
  and an explicit no-restart rule for an in-flight scan of the unchanged root.

## Impact

- **Code**: `src/pane/mod.rs` (font family constant, `grid_metrics`,
  `build_grid`/segment layout, startup verification hook, real-metrics tests);
  `src/tree/mod.rs` (`on_data_changed` generation check, visibility check);
  `src/workspace_property_source/filetree.rs` (`refresh` in-flight guard,
  `view()` caching); `src/app.rs` (`notify_panels` wiring unchanged, possible
  visibility plumbing).
- **Dependencies/assets**: possibly a bundled monospace font asset and its
  registration with the text system; no API changes.
- **Tests**: new real-text-system advance tests in `pane`, generation-cache
  and scan-restart tests in `tree`/`filetree`; existing mock-based cell-width
  tests remain but are re-scoped as mock-environment tests.
- **Behavior visible to users**: cursor and box borders align with the cell
  grid on Windows; typing stays responsive in very large workspaces; the
  initial file-tree scan completes while the user types.
