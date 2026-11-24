# Spec Delta

## MODIFIED Requirements

### Requirement: Application menu bar is always present

The application SHALL install an application menu bar at startup in all build
profiles, including while the Launcher window is the only window open: during
that phase at least one menu SHALL be present, and menus whose actions depend
on the main window's application state SHALL be added once that state exists.
On platforms with a native application menu bar the menu SHALL be rendered by
the operating system; on other platforms the menu bar SHALL be rendered
inside the application window above the dock layout. The menu bar SHALL
provide at least one menu at all times.

#### Scenario: Menu bar exists in release builds

- **WHEN** the user runs a release build of the application
- **THEN** the application menu bar is present and opens its menus

#### Scenario: Menu bar exists in debug builds

- **WHEN** the user runs a debug build of the application
- **THEN** the application menu bar is present, including the Debug menu

#### Scenario: Menu bar exists while the Launcher is open

- **WHEN** the application is sitting at the Launcher with no workspace
  chosen
- **THEN** the application menu bar is present with at least one menu, and no
  menu entry depends on main-window state that does not yet exist
