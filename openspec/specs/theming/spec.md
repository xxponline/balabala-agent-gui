# theming Specification

## Purpose

Defines the application's light/dark theme behavior: following the operating
system appearance, keeping every chrome surface consistent with the active
theme, and mapping terminal colors (defaults and ANSI palette) per theme while
preserving session-emitted colors.

## Requirements

### Requirement: Theme follows operating-system appearance

The application SHALL render in the light or dark theme matching the operating
system's appearance at startup, and SHALL switch the whole application live
when the system appearance changes. There SHALL be no in-app theme override
and no persisted theme preference.

#### Scenario: Dark system appearance at launch

- **WHEN** the application starts while the operating system is in dark
  appearance
- **THEN** the window chrome, panels, and terminal area render in the dark
  theme without any user action

#### Scenario: Live switch while running

- **WHEN** the operating system appearance changes from light to dark (or vice
  versa) while the application is open
- **THEN** all open windows re-render in the new theme before the next user
  interaction, with no restart required
### Requirement: Chrome surfaces are theme-consistent

Every application chrome surface (solution-overview rows, asset-overview
rows, terminal pane frame, status bar, tab area, buttons, and read-only
viewer panels) SHALL use colors derived from the active theme, including
background, border, hover, selection, muted text, and status accent colors.
No chrome surface SHALL keep a color that is legible in only one theme.

#### Scenario: Sidebar hover and active rows in both themes

- **WHEN** the theme is dark and the user hovers or selects a project,
  agent, or asset-overview row
- **THEN** the hover background, active-row highlight, and text colors are
  legible against the dark background (and symmetrically in the light theme)

#### Scenario: Status badges and destructive affordances

- **WHEN** a session badge shows starting/running/exited and hover reveals a
  close control
- **THEN** badge and control colors remain distinguishable from the row
  background in the active theme
### Requirement: Terminal default colors follow the theme

The terminal grid's default foreground and default background SHALL follow the
active theme so empty cells and unstyled text match the pane surface. Colors
explicitly emitted by the session SHALL be passed through unchanged in both
themes.

#### Scenario: Empty terminal area matches the pane

- **WHEN** a session screen contains cells with no explicit colors
- **THEN** those cells render in the theme's default background and foreground,
  with no visible seam against the pane frame

#### Scenario: Explicit session colors pass through

- **WHEN** a session emits text with an explicit RGB color
- **THEN** the cell renders in exactly that RGB value in both themes
### Requirement: ANSI palette is theme-appropriate

ANSI color indices 0-15 SHALL map to a palette tuned for the active theme, so
index-based colors (status text, labels, highlights) stay legible in both
themes. Higher xterm indices (256-color cube and grayscale) SHALL keep the
standard conversion in both themes. When the theme switches, already-rendered
session screens SHALL re-render under the new palette without restarting the
session.

#### Scenario: Index-colored text readable in dark theme

- **WHEN** the theme is dark and a session emits index-colored text (e.g. green
  success or yellow warning)
- **THEN** the text is legible on the dark default background

#### Scenario: Theme switch re-maps existing screen

- **WHEN** the system appearance changes while a session screen already shows
  index-colored content
- **THEN** the same screen is displayed with the new theme's palette after the
  switch, session still running
### Requirement: Session content colors follow the injected theme

Session-rendered content whose colors are designed by the embedded interface
(tool-card panel backgrounds, scrollbar track/thumb, code-block syntax
highlighting) SHALL be provided a theme-matching color set by the host when
spawning the session and SHALL be re-provided when the theme changes, so this
content is legible in the active theme. Without injection, the embedded
interface's default (dark-designed) colors SHALL remain unchanged.

#### Scenario: Light theme receives light color set

- **WHEN** a session is spawned while the light theme is active
- **THEN** its tool-card panels, scrollbar, and syntax highlighting render in
  the light color set, legible on the light default background

#### Scenario: Theme change updates live sessions

- **WHEN** the system appearance changes while sessions are running
- **THEN** running sessions re-render their content in the new color set
  without restarting or losing scrollback
