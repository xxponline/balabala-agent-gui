# Spec Delta

## MODIFIED Requirements

### Requirement: Solution configuration file

A workspace SHALL carry its solution configuration in `balabala.solutions.json`
in the workspace directory: a JSON array of solution items, each declaring a
`SolutionType`. Each item SHALL carry two configuration flags — whether its
projects register into the asset service, and whether git management is
enabled for it — and a missing flag SHALL default to enabled, so files
written before the flags existed load with unchanged behavior. The file SHALL
be created (empty array) together with the workspace marker; a workspace
without the file SHALL load as a valid workspace with zero projects.

#### Scenario: Empty solutions file yields zero projects

- **WHEN** a workspace loads and its `balabala.solutions.json` contains `[]`
- **THEN** the workspace opens with no project rows and no error

#### Scenario: Missing solutions file is a valid empty workspace

- **WHEN** a workspace loads and its `balabala.solutions.json` does not exist
- **THEN** the workspace opens with no project rows and no error

#### Scenario: Legacy item without flags loads with unchanged behavior

- **WHEN** a solutions file predates the flags and its items declare
  neither of them
- **THEN** both flags behave as enabled: the item's projects register into
  the asset service and git management applies as before
