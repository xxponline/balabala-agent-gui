# Spec Delta

## MODIFIED Requirements

### Requirement: Workspace persistence to JSON file

The system SHALL persist the list of workspace paths to `.workspaces.json` in
the project root on every mutation (workspace added or removed). The file
format SHALL be a JSON object whose `workspaces` key holds an array of
absolute path strings; an optional `file_tree` object (keyed by workspace
absolute path, each value an array of pattern strings) MAY coexist. A write
SHALL preserve every section the mutation does not own. Collapse state and
agent sessions SHALL NOT be persisted.

#### Scenario: Add triggers write

- **WHEN** the user adds a new workspace
- **THEN** `.workspaces.json` in the project root is rewritten and contains the new absolute path

#### Scenario: Remove triggers write

- **WHEN** the user deletes an existing empty workspace
- **THEN** `.workspaces.json` in the project root is rewritten and no longer contains the removed path

#### Scenario: Filter section survives workspace mutation

- **WHEN** `.workspaces.json` carries a `file_tree` section and the user adds or removes a workspace
- **THEN** the rewrite keeps the `file_tree` section intact
