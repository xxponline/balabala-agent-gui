# Spec Delta

## MODIFIED Requirements

### Requirement: File-row filtering with gitignore-style rules

Filter rules SHALL come from the `file_tree` section of the active workspace's
`.balabala.workspace.json`, as an array of pattern strings. With no section or
an empty array every file row shows. Patterns support wildcards (`*`, `?`,
`**`) and path components relative to the workspace root; the last matching
rule wins and a `!` prefix re-includes (gitignore semantics). Rules SHALL
apply to file rows only: directory rows always remain visible.

#### Scenario: Empty rules show everything

- **WHEN** the workspace's `.balabala.workspace.json` has no `file_tree`
  section or an empty array
- **THEN** all non-excluded file rows are visible

#### Scenario: Hide matching files

- **WHEN** the rules are `["*.log"]` and `docs/` contains only `.log` files
- **THEN** all `.log` file rows are hidden while other file rows are visible,
  and the `docs/` directory row is still visible

#### Scenario: Deny-all with re-include

- **WHEN** the rules are `["*", "!*.rs"]`
- **THEN** only `.rs` file rows are shown and every directory row stays visible

#### Scenario: Last match wins

- **WHEN** the rules are `["*", "!*.rs", "*_test.rs"]`
- **THEN** `foo.rs` is shown while `foo_test.rs` is hidden

#### Scenario: Rules re-read from workspace config on refresh

- **WHEN** the user edits the `file_tree` section in the workspace's
  `.balabala.workspace.json` and clicks the refresh control
- **THEN** the tree applies the newly read rules
