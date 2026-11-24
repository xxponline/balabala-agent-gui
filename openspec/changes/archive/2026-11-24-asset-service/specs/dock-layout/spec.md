# Spec Delta

## MODIFIED Requirements

### Requirement: Right dock renders by default

On startup the right dock SHALL be visible and render the asset-overview
panel: a header carrying the panel title, a refresh control, and a close
button, plus a body area that shows the asset tree of registered projects or
its empty state. The panel SHALL register through the dock's standard panel
path so resize and close mechanics apply to it unchanged.

#### Scenario: Right dock visible on startup

- **WHEN** the application window opens for the first time in a session
- **THEN** the right dock occupies its slot at the window's right edge showing the asset-overview panel

#### Scenario: Header chrome present

- **WHEN** the user inspects the right dock header
- **THEN** it shows the panel title, a refresh control, and a close button

### Requirement: Right dock close

A close control on the right dock SHALL close the dock: it renders at zero
width (no panel content visible), the center pane expands into the freed space,
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
- **THEN** the right dock is visible again at its default width showing the asset-overview panel
