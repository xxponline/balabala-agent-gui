# Proposal

## Why

`terminal_grid()` renders every vt100 cell as naturally-flowing monospace text,
so a wide CJK character (visual width 2 cells) plus its continuation cell
(empty → rendered as one nbsp) contributes 2+1 = 3 columns of visual width for
2 cells of logical width. Every wide character therefore pushes the rest of its
row ~1 cell to the right: rows containing CJK drift out of alignment and their
row ends are clipped. This is already wrong at the fixed 100×30 grid, and it
would get visibly worse once the grid size adapts to the pane (planned
separately in `add-adaptive-terminal-grid`), because column counts are then
derived from pixel width and must match what the renderer actually paints.

## What Changes

- Rework `terminal_grid()` in `src/ui/pane.rs` from "naturally flowing text
  segments" to fixed-width cell groups:
  - skip `is_wide_continuation()` cells (they contribute no width),
  - give `is_wide()` cells 2 columns of width, ordinary/empty cells 1 column,
  - size each rendered segment box to `columns × cell_width` in pixels.
- Introduce an explicit cell-width metric (monospace advance width at the
  pane's font size) as the single source of truth for segment widths.
- Color-run merging is preserved; segments additionally carry a column count.
- No changes to `vt100`, `pi-shell-rs`, session lifecycle, resize wiring, or
  input forwarding.

## Capabilities

### New Capabilities

_(none)_

### Modified Capabilities

- `agent-shell`: The `Terminal output rendering` requirement gains an explicit
  cell-width alignment guarantee — every row's rendered pixel width is
  exactly `columns × cell_width`, wide (CJK/emoji) characters occupy two
  cells' worth of width, and continuation cells are not painted — with
  scenarios covering mixed ASCII/CJK rows.

## Impact

- **Code**: `src/ui/pane.rs` only (`terminal_grid()` and its segment/row
  construction). `src/app.rs` untouched.
- **Behavior**: rows containing CJK/emoji stop drifting; row ends line up
  exactly with the grid's logical column count.
- **Dependencies**: none added (metrics come from the existing GPUI text
  system).
- **Risk**: per-row element count rises (segment boxes instead of free-flowing
  text); emoji/combining-character edge cases need manual verification
  (recorded as an open question in design.md).
