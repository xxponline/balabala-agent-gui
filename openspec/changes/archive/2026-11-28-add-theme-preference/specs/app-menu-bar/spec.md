# Spec Delta

## ADDED Requirements

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
