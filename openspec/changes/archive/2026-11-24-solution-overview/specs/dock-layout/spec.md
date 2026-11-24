# Spec Delta

## MODIFIED Requirements

### Requirement: Left dock draggable resize

The left dock SHALL be resizable by dragging its right edge, clamped to a safe
minimum width and a maximum that keeps the center pane usable. The
solution-overview content SHALL reflow into the new width, and the terminal
grid SHALL recompute its columns and rows from the changed available area in
the same step.

#### Scenario: Drag widens the sidebar

- **WHEN** the user drags the left dock's right edge outward while a session is shown
- **THEN** the sidebar becomes wider and the terminal grid recomputes fewer columns from the reduced center area

#### Scenario: Resize clamped at minimum

- **WHEN** the user drags the left dock's right edge inward past the minimum width
- **THEN** the dock stops at the minimum width and the center pane remains visible
