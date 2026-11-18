# Spec Delta

## MODIFIED Requirements

### Requirement: File-row filtering relative to project roots

Filter rules SHALL come from the `file_tree` section of the workspace's
`.balabala.workspace.json` and SHALL be applied by the asset manager.
Patterns support wildcards (`*`, `?`, `**`) and path components relative to
each project's root; the last matching rule wins and a `!` prefix
re-includes (gitignore semantics), a trailing `/` matches directories only,
and a bare pattern matches any entry name at any depth. Rules SHALL apply
to file and directory rows alike: an excluded directory SHALL be absent
from the view together with its entire subtree, so `!` rules beneath an
excluded directory never take effect while a `!` rule matching the
directory itself re-includes its subtree. With no section or an empty
array every row shows.

#### Scenario: Empty rules show everything

- **WHEN** the workspace's `.balabala.workspace.json` has no `file_tree`
  section or an empty array
- **THEN** all non-excluded rows are visible

#### Scenario: Hide matching files in every project

- **WHEN** the rules are `["*.log"]` and two registered projects each
  contain `.log` files
- **THEN** `.log` rows are hidden under both projects while other file rows
  show, and directory rows remain visible

#### Scenario: Deny-all with re-include

- **WHEN** the rules are `["*", "!*.rs"]`
- **THEN** only `.rs` file rows at each project's root are shown, nested
  `.rs` rows are absent because every directory is excluded with its
  subtree, matching gitignore's rule that a re-include cannot reach inside
  an excluded directory

#### Scenario: Last match wins

- **WHEN** the rules are `["*", "!*.rs", "*_test.rs"]`
- **THEN** `foo.rs` is shown while `foo_test.rs` is hidden

#### Scenario: Patterns are evaluated per project root

- **WHEN** the rules are `["src/**"]` and two projects both contain
  `src/` directories
- **THEN** both projects' `src` subtrees are matched by the same rule,
  each relative to its own project root

#### Scenario: Re-including a directory re-includes its subtree

- **WHEN** the rules are `["*", "!src/", "!src/**"]`
- **THEN** the `src` directory and all of its descendants are shown while
  every other entry is absent

#### Scenario: Re-include beneath an excluded directory has no effect

- **WHEN** the rules are `["build/", "!build/keep.txt"]` and the project
  contains `build/keep.txt`
- **THEN** neither `build` nor `build/keep.txt` appears in the view

#### Scenario: Trailing slash matches directories only

- **WHEN** the rules are `["build/"]` and the project contains both a
  directory named `build` and a file named `build`
- **THEN** the directory `build` is absent from the view while the file
  `build` is shown

## ADDED Requirements

### Requirement: Enumeration prunes excluded directories

Enumeration SHALL NOT descend into an excluded directory: once an entry
evaluates as excluded, the enumeration SHALL skip that branch entirely,
neither reading its contents nor producing any descendant row. A directory
that is not itself excluded SHALL be enumerated and appear in the view
even when all of its children are filtered out.

#### Scenario: Excluded subtree is never enumerated

- **WHEN** the rules exclude `build/` and the module root contains
  `build/` with nested files at several depths
- **THEN** no row under `build` appears anywhere in the view

#### Scenario: Unexcluded directory with all children filtered stays visible

- **WHEN** the rules are `["*.log"]` and the directory `logs/` contains
  only `.log` files
- **THEN** the `logs` directory row is present with no children
