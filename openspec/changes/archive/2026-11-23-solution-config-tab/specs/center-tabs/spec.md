# Spec Delta

## ADDED Requirements

### Requirement: SolutionConfig tab opens through the add-solution entry

The left-slot add-solution entry SHALL open the SolutionConfig form as a
center slot tab through an entry point separate from the file-object
backend. Its dedup identity SHALL be a fixed form id, independent of file
paths and section ids. While a SolutionConfig tab is open, activating the
entry SHALL select it instead of opening a second one; the tab SHALL be
closable and SHALL reopen fresh after closing.

#### Scenario: Entry opens the form as a center tab

- **WHEN** the user activates the add-solution entry while the console is
  the only center tab
- **THEN** a SolutionConfig tab opens after the console and becomes active,
  and no system file picker appears

#### Scenario: Re-entry selects the single tab

- **WHEN** a SolutionConfig tab is open and the user activates the
  add-solution entry again
- **THEN** the existing SolutionConfig tab becomes active and exactly one
  SolutionConfig tab exists

#### Scenario: Closing the form reopens fresh

- **WHEN** the user closes the SolutionConfig tab and later activates the
  add-solution entry again
- **THEN** a fresh SolutionConfig tab opens
