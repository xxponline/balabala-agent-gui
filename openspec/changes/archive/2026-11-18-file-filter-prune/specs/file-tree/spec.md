# Spec Delta

## MODIFIED Requirements

### Requirement: Refresh triggers and behavior

The panel's refresh control SHALL request asset re-enumeration from the
asset manager on click; first registration and a changed registered project
set likewise trigger re-enumeration. A refresh SHALL run in the background
without blocking interaction, SHALL re-read each project's filter rules
(the project root's `.gitignore` and git state), and SHALL preserve the
expansion state of previously expanded directories.

#### Scenario: Refresh picks up new files

- **WHEN** a file is created on disk and the user clicks the refresh control
- **THEN** the new file appears in the tree

#### Scenario: Refresh re-reads filter rules

- **WHEN** the user edits the project root's `.gitignore` and clicks
  refresh
- **THEN** the tree applies the newly read rules

#### Scenario: Refresh preserves expansion

- **WHEN** `src/` was expanded before the user clicks refresh
- **THEN** `src/` is still expanded after the refresh completes
