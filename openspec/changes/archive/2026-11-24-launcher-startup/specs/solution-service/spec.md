# Spec Delta

## MODIFIED Requirements

### Requirement: Solution load failures surface as structured errors

When loading fails — the solutions file cannot be parsed, an item declares an
unknown `SolutionType`, or a `simple` item's path does not exist on disk —
the service SHALL reply with the failure reason and SHALL NOT return partial
project state. Every load boundary (workspace open chosen at startup, explicit
reload) SHALL present the error dialog and SHALL NOT open the workspace.

#### Scenario: Corrupt file at startup

- **WHEN** the user chooses a workspace at startup and its
  `balabala.solutions.json` contains invalid JSON
- **THEN** the load reply carries the parse failure, an error dialog
  describes it, and the workspace is not opened

#### Scenario: Failed load yields no partial tree

- **WHEN** one item's path is missing on disk and others are valid
- **THEN** the reply is the failure only — no partial project tree is
  returned or cached
