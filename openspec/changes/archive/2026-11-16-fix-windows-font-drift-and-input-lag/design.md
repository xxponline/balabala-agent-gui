# Design

## Context

See proposal.md for motivation. The facts that shape this design:

- `src/pane/mod.rs` hard-codes `GRID_FONT_FAMILY = "Menlo"` and derives
  `cell_w` from `layout_line("MMMMMMMMMM").width / 10`. Rendering then places
  glyphs at natural advance inside fixed-width segment divs — the two must
  agree or rows drift.
- gpui's `test-support` platform uses `NoopTextSystem` (advance =
  `600/glyph_id` units of a 1000-unit em → every BMP char is exactly 0.6em),
  so all existing cell-width tests measure a mock, not real fonts.
- `TreePanel::on_data_changed` (called unconditionally from `notify_panels`
  on every consumed frame) runs `source.view()` → `filter_tree` deep clone and
  a derived-`PartialEq` deep compare against `last_synced`. For the 510k-file
  workspace this is O(tree) on the UI thread per keystroke.
- While the initial scan is in flight, `view()` returns `Empty`, so
  `expected != current` holds on every notification → `refresh()` cancels and
  restarts the recursive scan each frame.

## Goals / Non-Goals

**Goals:**

- Cursor block and box borders land on cell boundaries on every supported
  platform, with a test that fails when the resolved font is non-monospace.
- Per-frame data-change notifications cost O(1) in workspace size; the
  initial scan completes while the user keeps typing.
- Keep the existing fixed-width segment rendering contract (row width =
  columns × cell_w) — the spec's alignment scenarios depend on it.

**Non-Goals:**

- Filter-rule semantics, scan throughput, or UnrealEngine-specific excludes.
- Changing the input/echo pipeline (8ms burst + 50ms main loop) — it was not
  implicated in the lag.
- Replacing gpui's text system or vendoring a shaping engine.

## Decisions

### D1: Ordered font fallback list, no bundled font (initially)

Try `Menlo` (macOS), then `Cascadia Mono`, `Consolas`, `Ubuntu Sans Mono`,
`DejaVu Sans Mono` — all present on their target platforms. Rationale:
zero asset weight, system fonts render identically to the rest of the OS.
Alternative considered: bundling an open mono font via
`text_system.add_fonts` — fully deterministic cross-platform, but adds an
asset and a second rendering path; deferred unless the fallback list proves
insufficient on some host (the startup check in D2 makes that failure loud).

### D2: Startup monospace verification with loud diagnostic

On first `grid_metrics` call, measure advance for `M`, `a`, space, and `—`
through the real text system. If spread exceeds a small tolerance (e.g. 0.5px
at 13px), `eprintln!` a diagnostic naming the resolved font and the measured
advances. Rationale: makes the failure visible immediately instead of as a
mysteriously misaligned cursor; costs one cached layout per glyph class.
Alternative considered: hard-fail startup — rejected, a slightly imperfect
font should not take the app down.

### D3: Per-cell glyph positioning inside merged segments (drift hardening)

Instead of letting a merged same-color segment lay out its text at natural
advance, render each cell as a fixed-width inline chunk (width = cell_w,
`overflow_hidden`), so a glyph can never accumulate position error from its
left neighbors. This keeps the spec's "position = N × cell_w" guarantee even
if D1/D2 ever pass a marginally non-monospace font. Alternative considered:
per-cell divs (one div per cell) — simpler but multiplies element count by
cell count; inline chunks inside the existing segment div keep the element
tree shape. Note: this subsumes the existing F2 "wide char is its own
segment" workaround, which can then be simplified.

Implementation note (discovered during task 2.1): gpui quantizes laid-out
box widths and origins to device pixels (`Window::pixel_snap`). A raw
fractional cell width (e.g. 7.62px at 125% scaling) would then be rounded
independently per box, and a 2-cell wide-char segment would round
differently from two 1-cell segments — reintroducing a boundary shift.
`grid_metrics` therefore returns `window.pixel_snap(cell_w)`, so
`cols × cell_w` is always an integer multiple of the device pixel and
painted positions land exactly on `N × cell_w`. Snapping at the source
keeps adaptive sizing, selection overlay, and rendering on one grid.

### D4: Real-metrics test via an injected real platform text system

The mock (`NoopTextSystem`) is why CI is blind. Add a test that builds a
`TestAppContext`/window with the host's real platform text system (the
`build_with_text_system` seam in gpui-pre, fed with
`gpui_platform::current_platform(false).text_system()`) and asserts equal
advances for the sampled glyph classes, plus a painted-width probe asserting
`position(N) == N × cell_w`. Runs are host-dependent: on a host where the
fallback list yields a mono font it must pass; the assertion names the
resolved font on failure so a red CI run is self-diagnosing. Alternative
considered: golden-image rendering tests — heavier and flaky across GPUs;
rejected.

### D5: Tree view behind a generation epoch

Add a `view_generation: u64` (bumped by: scan landed, root changed, rules
changed, explicit refresh) plus a cached `SourceView`. `on_data_changed`
becomes: compare the current epoch (a cheap field compare — root, rules
hash, scan-landed flag) against the cache's epoch; only on mismatch recompute
`view()` and re-sync `TreeState`. The deep `last_synced != view` compare is
removed — the epoch is the invalidation signal. Alternative considered:
`Arc`-sharing the canonical tree so `view()` is a pointer copy — better
constant factor but still O(rules) filter per frame; epoch caching is simpler
and bounds work at zero when idle. Rules changes are detected by re-reading
the rules file on refresh only (already the trigger), plus the existing root
comparison.

### D6: In-flight scan guard

`refresh()` becomes idempotent per (root, epoch): if a scan for the current
root is already running and no invalidating event occurred, return without
cancelling. The `expected != current` check stops using `last_synced.view()`
(empty during scan) as the "current root" signal; instead track the root the
source is *scanning for* separately from the view it has *landed*.

### D7: Visibility gate

`notify_panels` keeps its unconditional notify (cheap), but `on_data_changed`
returns early — before any epoch check that touches the source — when the
right-dock tree panel is not rendered. Reopening the dock reads the cache;
if the epoch moved while hidden, the next `on_data_changed` (or the dock's
own render path) syncs. Must ensure reopening still shows fresh content: the
panel's render path checks the epoch lazily, not only via notifications.

## Risks / Trade-offs

- [D1 fallback list misses a host's fonts] → D2 diagnostic names the resolved
  font and measured advances; fix is one line adding a family, or escalate to
  bundling (D1 alternative).
- [D3 inline-chunk rendering regresses CJK/emoji width behavior] → existing
  wide-char and emoji scenarios stay as tests; F2 probe retained but
  re-pointed at real metrics.
- [D4 real-font test is host-dependent] → keep the assertion tolerant
  (advance spread, not exact px) so any genuine mono font passes on any host;
  failure output names the font.
- [D5 epoch misses an invalidation source] → epoch bump sites are colocated
  with the four existing triggers (scan land, root change, rules refresh,
  manual refresh); a missed bump shows as stale content, and manual refresh
  always bumps, giving a user-visible escape hatch.
- [D7 hidden-panel staleness] → lazy epoch check on render path covers frames
  consumed while hidden; covered by the "reopening the dock" scenario.

## Migration Plan

Single-release fix, no data or API migration. Rollback = revert the commit;
behavioral surface is confined to `src/pane`, `src/tree`,
`src/workspace_property_source/filetree`, and `src/app`'s notify wiring.
Feature-level fallback (keep old font constant until verification passes) is
not needed because D2 only warns.

## Open Questions

- Whether to bundle a font in this change or wait for D2 diagnostics from
  real hosts — deferred deliberately; does not affect specs or task order
  (D2's check and D1's list ship either way, bundling is an additive task).
