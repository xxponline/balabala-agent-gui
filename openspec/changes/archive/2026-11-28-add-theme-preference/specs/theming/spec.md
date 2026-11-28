# Spec Delta

## MODIFIED Requirements

### Requirement: Theme follows operating-system appearance

The application SHALL honor a persisted three-state theme preference — follow
system, light, or dark — defaulting to follow system, applied at startup
before the Launcher appears; a missing or unreadable preference file
SHALL behave as the default. In follow system mode the application SHALL
render matching the operating system's appearance and switch live when it
changes. A pinned light or dark preference SHALL hold against system
appearance changes until it returns to follow system.

#### Scenario: Dark system appearance at launch

- **WHEN** the application starts while the operating system is in dark
  appearance and the theme preference is follow system (the default)
- **THEN** the window chrome, panels, and terminal area render in the dark
  theme without any user action

#### Scenario: Live switch while running

- **WHEN** the operating system appearance changes from light to dark (or vice
  versa) while the application is open and the theme preference is follow
  system
- **THEN** all open windows re-render in the new theme before the next user
  interaction, with no restart required

#### Scenario: Pinned preference holds against system changes

- **WHEN** the theme preference is pinned to light while the operating system
  is in dark appearance, and the operating system later switches to light
- **THEN** the application keeps rendering in the pinned light theme, and the
  system appearance change does not alter it

#### Scenario: Preference applied at startup before the Launcher

- **WHEN** the application starts with a stored preference of dark while the
  operating system is in light appearance
- **THEN** the Launcher and, after a workspace is chosen, the main window
  render in the dark theme without any user action

#### Scenario: Preference persists across restarts

- **WHEN** the user changes the theme preference to dark, quits, and launches
  the application again
- **THEN** the application starts in the dark theme without re-selecting it

#### Scenario: Missing or unreadable preference falls back to the default

- **WHEN** the application starts and the stored preference file is missing,
  unreadable, or contains an unrecognized value
- **THEN** the preference behaves as follow system and the theme matches the
  operating system appearance

#### Scenario: Returning to follow system snaps to the current appearance

- **WHEN** the preference is pinned to dark, the operating system is in light
  appearance, and the user sets the preference back to follow system
- **THEN** the application switches to the light theme immediately, without
  waiting for another system appearance change
