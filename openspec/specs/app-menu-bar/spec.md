# app-menu-bar Specification

## Purpose

Owns the application menu bar: its platform presentation (native OS menu bar
where available, in-window fallback otherwise), its standard menu contents,
menu-item activation through the app's action dispatch, and rebuilding the
menu when its content changes.

## Requirements

### Requirement: Application menu bar is always present

The application SHALL install an application menu bar at startup in all build
profiles. On platforms with a native application menu bar the menu SHALL be
rendered by the operating system; on other platforms the menu bar SHALL be
rendered inside the application window above the dock layout. The menu bar
SHALL provide at least one menu at all times.

#### Scenario: Menu bar exists in release builds

- **WHEN** the user runs a release build of the application
- **THEN** the application menu bar is present and opens its menus

#### Scenario: Menu bar exists in debug builds

- **WHEN** the user runs a debug build of the application
- **THEN** the application menu bar is present, including the Debug menu

### Requirement: Standard menus carry existing actions

The menu bar SHALL include an Edit menu whose Copy item dispatches the
application's existing copy-selection action. A menu item SHALL be enabled
only when its action is available in the current context (for example Copy
is disabled while no text is selected), and SHALL display the action's
keyboard shortcut when one is bound. Menu items SHALL activate through the
same action dispatch path as their keyboard shortcuts.

#### Scenario: Copy enabled with an active selection

- **WHEN** the user has an active text selection in the console tab and opens the Edit menu
- **THEN** the Copy item is enabled and shows its keyboard shortcut

#### Scenario: Copy disabled without a selection

- **WHEN** the user opens the Edit menu while no text is selected
- **THEN** the Copy item is disabled

#### Scenario: Menu item dispatches the same action as the shortcut

- **WHEN** the user clicks the Copy menu item with an active selection
- **THEN** the selection is copied to the clipboard, identically to pressing the keyboard shortcut

### Requirement: Menu content rebuild on change

Whenever the set of menu items the application exposes changes (for example a
debug section being registered or a dynamic item's label changing), the system
SHALL rebuild the platform menu so the displayed contents match the current
set. Rebuilding SHALL NOT drop items that are still present nor change their
order.

#### Scenario: Registered section appears in the menu

- **WHEN** a debug section is registered after startup
- **THEN** the rebuilt menu bar lists that section's menu item

#### Scenario: Rebuild preserves unchanged items

- **WHEN** the menu is rebuilt after one item's label changes
- **THEN** all other items remain present in their original order
