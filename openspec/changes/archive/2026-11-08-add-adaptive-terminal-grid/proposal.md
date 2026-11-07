# Proposal

## Why

The main pane's terminal grid is a hardcoded 100×30 (`COLS`/`ROWS` constants
in `src/agent_entities/pi_agent_shell.rs`), so resizing the window or pane
leaves the grid at a fixed resolution: large panes waste space, small panes
clip content. Meanwhile pi-shell-rs already supports adaptive rendering in its
standalone console (SIGWINCH → `UiEvent::Resize` → ratatui autoresize), and
exposes the same mechanism to hosts via `EmbeddedApp::resize()` — the GUI
simply never calls it. The pane should behave like a real terminal: column and
row counts follow the available pixel area.

## What Changes

- Measure the terminal grid area's pixel bounds each paint (reusing the
  bounds already obtained by the pane's IME canvas) and convert to grid
  dimensions using the monospace cell metrics: `cols = floor(width /
  cell_width)`, `rows = floor(height / line_height)`, clamped to a safe
  minimum.
- On a dimension change, call `EmbeddedApp::resize(cols, rows)` (embed TUI
  re-lays out and redraws) and `vt100::Parser::set_size(rows, cols)` (screen
  reflows) for every running session; `Exited` sessions are never resized so
  their preserved final screen stays intact (per existing spec).
- Deduplicate: only act when computed (cols, rows) differs from the current
  size; optional throttle aligned with the existing 50 ms poll loop.
- Remove the fixed `COLS`/`ROWS` constants as the source of truth (initial
  size comes from the first measured bounds, with a sane fallback before the
  first layout).

## Capabilities

### New Capabilities

_(none)_

### Modified Capabilities

- `agent-shell`: Two new requirements alongside `Terminal output rendering`
  (as amended by `add-cell-width-terminal-rendering`): adaptive grid sizing
  (grid dimensions derive from the pane's measured pixel area and monospace
  metrics, clamped to a safe minimum), and grid resize propagation
  (dimension changes propagate to both the embedded TUI and the vt100
  screen in one step, unchanged dimensions propagate nothing, and `Exited`
  sessions are exempt to preserve their last rendered screen).

## Impact

- **Code**: `src/app.rs` (bounds measurement, dedupe, dispatch),
  `src/agent_entities/pi_agent_shell.rs` (expose/forward resize, drop fixed
  constants), possibly `src/ui/pane.rs` (metrics helper shared with rendering).
- **Behavior**: grid resolution tracks pane size; TUI layout (pi's own
  screens, overlays) reflows like a real terminal resize.
- **Dependencies**: none added; reuses `EmbeddedApp::resize()` and
  `vt100::Parser::set_size()` which already exist.
- **Interaction**: builds on `add-cell-width-terminal-rendering` (shared
  `cell_width` metric; fixed-width rows make the cols formula exact), but
  has no hard dependency — it can land first, with CJK drift as a known
  limitation until the other change lands.
