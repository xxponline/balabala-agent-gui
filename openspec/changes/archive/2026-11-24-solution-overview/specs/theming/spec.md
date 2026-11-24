# Spec Delta

## MODIFIED Requirements

### Requirement: Chrome surfaces are theme-consistent

Every application chrome surface (solution-overview rows, file tree,
terminal pane frame, status bar, tab area, buttons, and read-only viewer
panels) SHALL use colors derived from the active theme, including
background, border, hover, selection, muted text, and status accent colors.
No chrome surface SHALL keep a color that is legible in only one theme.

#### Scenario: Sidebar hover and active rows in both themes

- **WHEN** the theme is dark and the user hovers or selects a project,
  agent, or file-tree row
- **THEN** the hover background, active-row highlight, and text colors are
  legible against the dark background (and symmetrically in the light theme)

#### Scenario: Status badges and destructive affordances

- **WHEN** a session badge shows starting/running/exited and hover reveals a
  close control
- **THEN** badge and control colors remain distinguishable from the row
  background in the active theme
