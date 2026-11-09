# Spec Delta

## Purpose

Owns the window's left and right dock slots: the left dock carries the sidebar
and can be resized by dragging, while the right dock is a reservable panel slot
that renders by default, can be resized, and can be closed — restored only by
restarting the application — providing the layout contract the next feature
module will fill.

## ADDED Requirements

### Requirement: Left dock draggable resize

The left dock SHALL be resizable by dragging its right edge, clamped to a safe
minimum width and a maximum that keeps the center pane usable. The sidebar
content SHALL reflow into the new width, and the terminal grid SHALL recompute
its columns and rows from the changed available area in the same step.

#### Scenario: Drag widens the sidebar

- **WHEN** the user drags the left dock's right edge outward while a session is shown
- **THEN** the sidebar becomes wider and the terminal grid recomputes fewer columns from the reduced center area

#### Scenario: Resize clamped at minimum

- **WHEN** the user drags the left dock's right edge inward past the minimum width
- **THEN** the dock stops at the minimum width and the center pane remains visible

### Requirement: Left dock is not closable

The system SHALL NOT offer any control that closes or hides the left dock, so
the sidebar's controls (`+ new workspace`, `[+ agent]`, close buttons) remain
reachable at all times.

#### Scenario: No close affordance on left dock

- **WHEN** the user inspects the left dock's edges and header
- **THEN** no close or hide control is present for the left dock

### Requirement: Right dock renders by default

On startup the right dock SHALL be visible and render its placeholder panel
(a header with a title and a close button, plus a body area). The placeholder
is the integration sample for the next feature module.

#### Scenario: Right dock visible on startup

- **WHEN** the application window opens for the first time in a session
- **THEN** the right dock occupies its slot at the window's right edge showing the placeholder panel

### Requirement: Right dock draggable resize

The right dock SHALL be resizable by dragging its left edge, clamped to the
same safe bounds as the left dock, and the terminal grid SHALL recompute from
the changed center area in the same step.

#### Scenario: Drag changes right dock width

- **WHEN** the user drags the right dock's left edge and releases it
- **THEN** the right dock keeps the new width and the terminal grid's column count reflects the new center width

### Requirement: Right dock close

A close control on the right dock SHALL close the dock: it renders at zero
width (no placeholder visible), the center pane expands into the freed space,
and the terminal grid recomputes. The system SHALL NOT provide any control to
reopen the right dock within the same session; a closed right dock is restored
only by restarting the application.

#### Scenario: Close expands the center pane

- **WHEN** the user clicks the right dock's close button
- **THEN** the right dock disappears, the terminal area widens, and the grid recomputes its columns

#### Scenario: No reopen control while closed

- **WHEN** the right dock is closed and the user inspects the window
- **THEN** no control is present that reopens the right dock within the current session

#### Scenario: Restart restores the right dock

- **WHEN** the user closes the right dock and then restarts the application
- **THEN** the right dock is visible again at its default width showing the placeholder panel
