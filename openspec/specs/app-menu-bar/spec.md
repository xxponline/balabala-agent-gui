# app-menu-bar Specification

## Purpose

Owns the application menu bar: its platform presentation (native OS menu bar
where available, in-window fallback otherwise), its standard menu contents,
menu-item activation through the app's action dispatch, and rebuilding the
menu when its content changes.

## Requirements

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

#### Scenario: Windowed menu bar is visible above the dock on Windows

- **WHEN** the user runs the application on Windows and looks at the main window
- **THEN** the application menu bar is rendered inside the window above the
  dock layout, with at least the Edit menu present, and it is visibly
  distinct from the dock panels rather than absent

#### Scenario: Windowed menu bar leaves the dock its full remaining space

- **WHEN** the windowed menu bar is present and the user resizes the window
- **THEN** the dock layout occupies the space below the menu bar and the
  terminal grid inside it tracks that reduced height, so no panel is pushed
  out of the window

### Requirement: Standard menus carry existing actions

The menu bar SHALL include an Edit menu whose Copy item dispatches the
application's existing copy-selection action. A menu item SHALL be enabled
only when its action is available in the current context (for example Copy
is disabled while no text is selected), and SHALL display the action's
keyboard shortcut when one is bound. Menu items SHALL activate through the
same action dispatch path as their keyboard shortcuts.

Where the menu bar is rendered inside the application window, the Copy
item's enabled state SHALL be derived from the presence of a text selection
alone: the focused element SHALL NOT be part of that determination.

#### Scenario: Copy enabled with an active selection

- **WHEN** the user has an active text selection in the console tab and opens the Edit menu
- **THEN** the Copy item is enabled and shows its keyboard shortcut

#### Scenario: Copy disabled without a selection

- **WHEN** the user opens the Edit menu while no text is selected
- **THEN** the Copy item is disabled

#### Scenario: Menu item dispatches the same action as the shortcut

- **WHEN** the user clicks the Copy menu item with an active selection
- **THEN** the selection is copied to the clipboard, identically to pressing the keyboard shortcut

#### Scenario: Focus outside the console does not disable Copy

- **WHEN** the user has an active text selection in the console tab, moves
  focus to another panel, and opens the Edit menu
- **THEN** the Copy item is enabled, because availability is derived from the
  selection alone

#### Scenario: Shortcut shown matches the platform binding

- **WHEN** the user opens the Edit menu on a platform and reads the shortcut
  shown beside Copy
- **THEN** the shown shortcut is the one actually bound to the copy action on
  that platform, and pressing it performs the same copy as clicking the item

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

### Requirement: Copy item enabled state follows selection changes

On platforms where the menu bar is rendered inside the application window,
the system SHALL recompute the Copy item's enabled state when the text
selection is created, changed, or cleared, and the recomputed state SHALL be
in effect the next time the menu is opened — without restarting the
application and without requiring any other menu content to change.

#### Scenario: Selection created after startup enables Copy

- **WHEN** the user starts the application with no selection, selects text in
  the console, and then opens the Edit menu
- **THEN** the Copy item is enabled

#### Scenario: Cleared selection disables Copy

- **WHEN** the user clears an existing selection and then opens the Edit menu
- **THEN** the Copy item is disabled

#### Scenario: Selection change does not disturb menu contents

- **WHEN** the selection changes while other menu items are present
- **THEN** the other menus and items remain present in their original order
  and the menu bar does not close or reopen on its own

### Requirement: View menu carries theme selection

The application menu bar SHALL include a View menu containing a Theme selection
of exactly three items — follow system, light, and dark — with exactly one
item check-marked to reflect the current theme preference. The View menu SHALL
be added once the main window exists; while only the Launcher is open the menu
bar SHALL remain valid without it. The check-mark state SHALL be preserved
when the menu bar rebuilds for unrelated content changes.

#### Scenario: Check mark reflects the active preference

- **WHEN** the user opens View > Theme while the theme preference is follow
  system
- **THEN** exactly the follow system item is check-marked and the light and
  dark items are not

#### Scenario: View menu appears with the main window

- **WHEN** the application is sitting at the Launcher with no workspace chosen
- **THEN** the menu bar contains no View menu, and after the main window opens
  the View menu with its three Theme items is present

#### Scenario: Unrelated menu rebuild preserves the check mark

- **WHEN** the menu bar rebuilds because unrelated menu content changed (for
  example a debug section was registered) while the theme preference is light
- **THEN** the View menu still shows exactly the light item check-marked, and
  all menus and items remain in their original order

### Requirement: Theme selection applies the choice

Choosing a theme item in the View menu SHALL update the persisted preference,
apply the theme to the whole application immediately, and leave the newly
chosen item check-marked the next time the menu is opened, without restarting
the application.

#### Scenario: Selecting a pinned theme applies it and moves the check mark

- **WHEN** the user chooses dark from View > Theme while the preference was
  follow system
- **THEN** the application renders in the dark theme immediately, and the next
  opening of View > Theme shows dark check-marked and the other two items
  unchecked

#### Scenario: Selection survives an application restart

- **WHEN** the user chooses light from View > Theme and later restarts the
  application
- **THEN** the application starts with the light preference in effect and
  View > Theme shows light check-marked
