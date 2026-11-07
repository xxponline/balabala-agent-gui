# Spec Delta

## ADDED Requirements

### Requirement: Adaptive terminal grid sizing

The terminal grid's column and row counts SHALL be derived from the measured
pixel area available to the grid and the pane's monospace metrics (cell
advance width and line height), using floor division clamped to a safe
minimum (never zero or an invalid size).

#### Scenario: Window resize changes grid resolution

- **WHEN** the user enlarges the application window while a session is shown
  in the main pane
- **THEN** the grid renders more columns and/or rows in the freed space

#### Scenario: Tiny pane clamps to minimum size

- **WHEN** the pane is shrunk so that the pixel area would compute fewer than
  the minimum columns or rows
- **THEN** the grid uses the minimum dimensions instead of zero or an
  invalid size

### Requirement: Grid resize propagation

When the computed grid dimensions differ from the current dimensions, the
system SHALL propagate the new size to both the embedded interface (so its
TUI re-lays out and redraws) and the session's vt100 screen (so the parsed
screen reflows), in the same step. When dimensions are unchanged, no resize
SHALL be propagated. Sessions in the `exited` state SHALL NOT be resized, so
their preserved last rendered screen is not reflowed.

#### Scenario: Resize reaches the embedded TUI

- **WHEN** a session's grid dimensions change while it is running
- **THEN** the embedded TUI redraws its layout for the new size and the
  following parsed screen reflects the new dimensions

#### Scenario: Unchanged layout propagates nothing

- **WHEN** repaints occur while the grid's computed (columns, rows) equal its
  current dimensions
- **THEN** no resize is propagated to the embedded interface or the vt100
  screen

#### Scenario: Exited session screen preserved

- **WHEN** the window is resized while the active session (or a background
  session) is in the `exited` state
- **THEN** that session's vt100 screen and last rendered frame are unchanged
