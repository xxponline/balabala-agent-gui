# Design

## Context

`terminal_grid()` in `src/ui/pane.rs` walks `vt100::Screen::cell(row, col)` and
merges adjacent same-color cells into text segments rendered as free-flowing
monospace text. Width is whatever the font produces. vt100 stores a wide
character as a leading cell (`is_wide()`, `contents()` = the char) plus a
continuation cell (`is_wide_continuation()`, empty `contents()`), and the
renderer currently paints the continuation as U+00A0 — so a 2-cell CJK char
renders as 2-cell glyph + 1-cell nbsp = 3 cells of visual width. Rows drift
right by one cell per wide character; row ends are clipped at the pane edge.
See proposal.md for why this must be fixed before the pane becomes adaptive.

Relevant facts:

- The renderer is the only place pixels are produced; `vt100` and
  `pi-shell-rs` store/draw nothing on screen. No third-party code changes.
- vt100 0.18 `Cell` already exposes `is_wide()` / `is_wide_continuation()`.
- The pane font is Menlo at 13px (hardcoded in `terminal_grid`); GPUI's text
  system can report the advance width for that font/size
  (`typographic_bounds(font_id, 13px, 'M')` or `layout_line`).

## Goals / Non-Goals

**Goals:**

- Row pixel width is deterministic: `columns × cell_width` exactly.
- One shared cell-width metric for rendering (the adaptive change
  `add-adaptive-terminal-grid` will reuse the same metric for its
  cols/rows conversion).
- Preserve existing behavior otherwise: color-run merging, cursor inversion,
  nbsp handling for empty cells, `starting` placeholder, focus/IME wiring.

**Non-Goals:**

- No resize/adaptive wiring (separate change).
- No changes to `vt100`, `pi-shell-rs`, input forwarding, session lifecycle.
- No font-size scaling or zoom.

## Decisions

**D1: Render segments as fixed-width boxes, not free-flowing text.**
Each color-run segment carries a column count; the segment div is sized
`w(px(columns × cell_width))` with the text inside. Alternative considered:
keep text flow and only skip continuation cells (cheaper) — rejected because
it leaves width at the font's discretion and the adaptive change's
`columns = floor(width / cell_width)` formula would no longer match painted
width. Text overflow inside a fixed box (kerning/ligature surprises) is
absorved by clipping, which is exactly what a terminal does.

**D2: Cell-width metric comes from the GPUI text system, measured once.**
Advance width of `M` (or a run of `M`s averaged) in Menlo 13px, obtained via
the window's text system during paint and cached on `TerminalApp`/module
state for the frame. Alternative: hardcode a constant (Menlo 13px advance is
stable on macOS) — acceptable fallback, but measuring keeps font substitution
(e.g. Menlo missing) honest. Either way the value is a single `f32` shared by
rendering and (later) adaptive conversion.

**D3: Wide-char loop rule.**
For each row, iterate columns; on `is_wide_continuation()` skip (contribute
nothing); on `is_wide()` consume the cell at width 2 (the continuation cell
is naturally skipped by the loop when it reaches it — no index arithmetic
needed since continuation renders nothing); otherwise width 1. Empty cells
keep the U+00A0 nbsp treatment inside a width-1 box so row divs never
collapse to zero height.

**D4: Cursor inversion is width-aware.**
When the cursor sits on a wide character's leading cell, the inverted block
spans 2 cells; on the continuation cell it is not drawn separately (the
leading cell's block already covers it).

## Risks / Trade-offs

- [Per-row element count rises: segments become boxes; worst case (full-color
  per cell) one box per cell] → Same order of magnitude as today's segment
  count for typical TUI output; benchmark with a 100×30 full-diff frame if
  dragging feels sluggish. Mitigation: merge adjacent same-width boxes is
  already the color-run logic; nothing new to do unless measured.
- [Emoji / combining marks may not behave as `is_wide()` suggests (ZWJ
  sequences, skin-tone modifiers)] → Known limitation; vt100 itself decides
  cell occupancy, we only honor it. Verify a few emoji manually; if a glyph
  overflows its box it clips (terminal-like), never shifts the row.
- [Fixed boxes could clip glyphs whose ink is wider than the advance width
  (italic f, some fallback fonts)] → `overflow: hidden` on segments keeps
  row geometry exact; visually identical to real terminals.

## Migration Plan

Single-commit rendering swap behind no flag; behavior change is immediately
visible (CJK rows line up). Rollback = revert the commit.

## Open Questions

- Does GPUI's `layout_line`/`typographic_bounds` need an explicit font load
  step for "Menlo" before metrics are available? (spike in task 1; fallback:
  measure from a laid-out test string during first paint.)
- Emoji rendering verification (covered in tasks, not spec-blocking).
