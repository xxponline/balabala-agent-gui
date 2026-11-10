# Spec Delta

## MODIFIED Requirements

### Requirement: Right dock renders by default

On startup the right dock SHALL be visible and render the file tree panel: a
header carrying the panel title, a refresh control, a read-only summary of
the active filter rules, and a close button, plus a body area that shows the
workspace file tree or its empty state. The panel SHALL register through the
dock's standard panel path so resize and close mechanics apply to it
unchanged.

#### Scenario: Right dock visible on startup

- **WHEN** the application window opens for the first time in a session
- **THEN** the right dock occupies its slot at the window's right edge showing the file tree panel

#### Scenario: Header chrome present

- **WHEN** the user inspects the right dock header
- **THEN** it shows the panel title, a refresh control, a read-only filter-rule summary, and a close button
