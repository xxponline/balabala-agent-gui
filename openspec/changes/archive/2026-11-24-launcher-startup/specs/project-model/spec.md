# Spec Delta

## MODIFIED Requirements

### Requirement: Solution load failure blocks the workspace

If `balabala.solutions.json` cannot be parsed, declares an unknown
`SolutionType`, or a `simple` item's `path` does not exist on disk, the system
SHALL show an error dialog describing the failure and SHALL NOT open the
workspace. The dialog SHALL appear at every load boundary (workspace open
chosen at startup, explicit reload) — the system SHALL NOT fall back to an
empty project list.

#### Scenario: Corrupt solutions file at startup

- **WHEN** the user chooses a workspace at startup whose
  `balabala.solutions.json` contains invalid JSON
- **THEN** an error dialog describes the parse failure and the application
  exits without opening the workspace

#### Scenario: Solution path missing on disk

- **WHEN** a workspace loads and a `simple` item names a directory that does
  not exist
- **THEN** an error dialog describes the missing path and the workspace is
  not opened

#### Scenario: Unknown solution type

- **WHEN** a solutions file contains an item whose `SolutionType` is not in
  the registry
- **THEN** an error dialog describes the unknown type and the workspace is
  not opened
