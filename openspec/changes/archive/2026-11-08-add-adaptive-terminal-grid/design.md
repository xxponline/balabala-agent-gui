# Design

## Context

- `PiAgentShell::new` hardcodes `COLS = 100`, `ROWS = 30` for both
  `EmbedConfig` and `vt100::Parser::new`. Nothing ever changes them.
- pi-shell-rs already implements the full adaptive path:
  `EmbeddedApp::resize(cols, rows)` writes the shared size slot and injects
  `UiEvent::Resize`; the embed event loop redraws, ratatui `autoresize`
  (reading `VirtualBackend::size()`) re-lays the TUI out, and a full-size
  ANSI frame comes out for the host's vt100 parser. This is exactly the path
  the standalone console uses on SIGWINCH.
- `vt100::Parser::set_size(rows, cols)` reflows the parsed screen.
- The pane's IME canvas in `src/app.rs` already receives the grid area's
  `pane_bounds` during paint — no new measurement plumbing is needed.
- The 50 ms poll loop (`start_update_loop`) drains ANSI frames and notifies
  on consumption; resize-driven frames ride this loop naturally.
- `add-cell-width-terminal-rendering` establishes a shared `cell_width`
  metric and exact `columns × cell_width` row widths (see proposal — no hard
  dependency, but the cols formula becomes exact once it lands).

See proposal.md for motivation; the sizing requirement lives in the spec
delta.

## Goals / Non-Goals

**Goals:**

- Grid resolution tracks the pane's pixel area like a real terminal.
- Resize propagates to both halves of the stack (embed TUI + vt100 screen)
  atomically, deduplicated.
- `Exited` sessions keep their preserved final screen untouched.
- No resize storms during live window dragging.

**Non-Goals:**

- Font-size scaling / zoom (grid dimensions adapt, not the font).
- Resizing sidebar, status bar, or non-terminal chrome.
- Persisting grid dimensions across restarts.
- Mouse reporting / terminal resize SIGWINCH semantics beyond what
  `EmbeddedApp::resize` already provides.

## Decisions

**D1: Read grid bounds from the IME canvas paint closure, not window
bounds.** The canvas sits inside the `flex_1` grid div, so its bounds are
exactly the area available to cells (already excludes sidebar, status bar,
separator, pane padding, `starting` placeholder). Alternatives: compute
`window.bounds() - fixed chrome` (fragile duplication of layout constants) or
a dedicated bounds-tracking element (new API surface for no gain). The canvas
already paints every frame for IME registration.

**D2: Convert pixels → cells with text-system metrics.**
`cols = floor(grid_w / cell_w)`, `rows = floor(grid_h /line_h)`, where
`cell_w` is the monospace advance and `line_h` the Menlo 13px line height,
both from the GPUI text system (same source as the cell-width renderer
change, so rendering and conversion share one truth). Clamp to a minimum
(e.g. 20×5; NonZeroU16-safe) — see spec scenario. Rounding remainder leaves
a sub-cell gap at the right/bottom edge; that is normal terminal behavior.

**D3: Propagate only on change, with a last-writer dedupe.**
Keep the session's current (cols, rows); on each paint-derived measurement,
skip if equal. During a drag each distinct size fires once per session — no
timer needed initially. If live-drag cost proves high (N sessions × full
TUI redraw per frame), throttle commits to the 50 ms poll cadence as a
follow-up; do not pre-build it.

**D4: Resize all `Running` (and `Starting`) sessions, skip `Exited`.**
All sessions render in the same pane, so background sessions must reflow
too or switching would show a stale-size frame. `Exited` sessions: embed
thread is dead (resize would be a no-op send) and `set_size` would reflow
the preserved screen — explicitly forbidden by spec. Note the ordering rule:
the resize must reach `embed.resize()` and `parser.set_size()` together so
the next ANSI frame and the reflowed screen agree on geometry.

**D5: Frame-ordering is tolerated, not synchronized.**
ANSI frames queued before a resize were rendered at the old size; applying
them to a reflowed screen can transiently garble one frame until the embed's
post-resize full redraw arrives (≤ 50 ms later). Real terminals have the
same asynchrony. Avoiding it would require draining/discarding frames on
resize — rejected as complexity for a sub-frame artifact.

**D6: Initial size = first measured bounds; constants become fallback.**
Before the first layout there is no bounds; `EmbedConfig`/`Parser::new` keep
a fallback (keep 100×30 or drop to 80×24 — prefer keeping current constants
as fallback to avoid changing startup appearance) until the first paint
measures and resizes.

**D7: Break the content→bounds feedback loop with clipping + zeroed auto
minimum sizes.** Measuring bounds from the canvas is only honest if the
container size is content-independent: flex auto minimum size let the grid's
min-content (fallback 100×30 = 780×630px) stretch the `flex_1` grid div and
then the whole `terminal-pane` (the root is a row, so the pane's automatic
minimum size binds on the horizontal axis). The first paint therefore
measured 780×630 — and `floor(780/7.8)=100`, `floor(630/21)=30` landed
exactly on the fallback, letting D3's dedupe lock the overflow in place
forever (observed: zero resize propagations with the pane overflowing the
window). Fix: `overflow_hidden()` + explicit `min_w/min_h(0)` on the grid
container, `overflow_hidden()` on `terminal-pane` — the spec requires the
"pixel area *available* to the grid", which content-inflated bounds are
not. Overflowing rows clip like a real terminal; verified headlessly
(741×505 for a 1000×600 window →95×24 cells) and live (`[grid]` log:
startup 741×506 → 94×24, drag tracks every distinct size, clamps at 20×5).

## Risks / Trade-offs

- [Live drag resizes every session each frame → CPU churn] → D3 dedupe
  bounds it to distinct sizes; measure with 3+ sessions before adding
  throttling.
- [Font metrics unavailable before first text layout] → fallback path in D6;
  spike task confirms measurement timing.
- [Reflow on resize changes vt100 scrollback framing] → same as any real
  terminal; `set_size`'s reflow semantics are upstream-tested.
- [Interaction with the cell-width change if it lands later] → cols formula
  still holds with natural text flow for ASCII; CJK drift is the known
  limitation owned by the other change.

## Open Questions

- Exact minimum clamp values (20×5 proposed) — cosmetic, adjustable in
  implementation without spec changes.
- Whether `Starting` sessions (no frame yet) should be resized eagerly or on
  first frame — trivial either way; default: treat like `Running`.
