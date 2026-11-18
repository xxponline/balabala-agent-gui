# Spec Delta

## MODIFIED Requirements

### Requirement: File-row filtering relative to project roots

Filter rules SHALL come from the filter set each project provides at
module creation — the default dot ignore plus, when the project root is
git-managed, that root's `.gitignore` — and SHALL be evaluated by the
enumeration. Patterns support wildcards (`*`, `?`, `**`) and path
components relative to each project's root; the last matching rule wins
and a `!` prefix re-includes (gitignore semantics), a trailing `/` matches
directories only, and a bare pattern matches any entry name at any depth.
Rules SHALL apply to file and directory rows alike: an excluded directory
SHALL be absent from the view together with its entire subtree, so `!`
rules beneath an excluded directory never take effect while a `!` rule
matching the directory itself re-includes its subtree. The default dot
ignore SHALL NOT be re-included by any rule. With no `.gitignore` every
non-dot row shows.

#### Scenario: Empty rules show everything

- **WHEN** the project root is not git-managed or has no `.gitignore`
- **THEN** every row that is not dot-prefixed is visible

#### Scenario: Hide matching files in every project

- **WHEN** each of two git-managed projects' `.gitignore` files hide
  `*.log` and both projects contain `.log` files
- **THEN** `.log` rows are hidden under both projects while other file rows
  show, and directory rows remain visible

#### Scenario: Deny-all with re-include

- **WHEN** a project's `.gitignore` lists `*` followed by `!*.rs`
- **THEN** only `.rs` file rows at each project's root are shown, nested
  `.rs` rows are absent because every directory is excluded with its
  subtree, matching gitignore's rule that a re-include cannot reach inside
  an excluded directory

#### Scenario: Last match wins

- **WHEN** a project's `.gitignore` lists `*`, then `!*.rs`, then
  `*_test.rs`
- **THEN** `foo.rs` is shown while `foo_test.rs` is hidden

#### Scenario: Patterns are evaluated per project root

- **WHEN** each of two git-managed projects' `.gitignore` files contain
  `src/**` and both projects contain `src/` directories
- **THEN** both projects' `src` subtrees are matched by the same rule,
  each relative to its own project root

#### Scenario: Re-including a directory re-includes its subtree

- **WHEN** a project's `.gitignore` lists `*`, then `!src/`, then `!src/**`
- **THEN** the `src` directory and all of its descendants are shown while
  every other entry is absent

#### Scenario: Re-include beneath an excluded directory has no effect

- **WHEN** a project's `.gitignore` lists `build/` followed by
  `!build/keep.txt` and the project contains `build/keep.txt`
- **THEN** neither `build` nor `build/keep.txt` appears in the view

#### Scenario: Trailing slash matches directories only

- **WHEN** a project's `.gitignore` contains `build/` and the project
  contains both a directory named `build` and a file named `build`
- **THEN** the directory `build` is absent from the view while the file
  `build` is shown

#### Scenario: Dot ignore cannot be re-included

- **WHEN** a git-managed project's `.gitignore` contains `!.env` and the
  project contains `.env`
- **THEN** `.env` does not appear in the view — the default dot ignore is
  a separate filter that no rule re-includes

### Requirement: Refresh re-enumerates in the background

The asset manager SHALL re-enumerate on first registration, when the
registered project set changes, and when the panel requests a refresh. A
refresh SHALL run in the background without blocking interaction and SHALL
re-read each project's filter sources (git state and the project root's
`.gitignore`).

#### Scenario: Refresh picks up new files

- **WHEN** a file is created under a module root and the user clicks the
  refresh control
- **THEN** the new file appears in the view

#### Scenario: Refresh re-reads filter rules

- **WHEN** the user edits the project root's `.gitignore` and clicks
  refresh
- **THEN** the view applies the newly read rules

## REMOVED Requirements

### Requirement: Module file enumeration with default exclusions

Each module SHALL enumerate files recursively under its own root. The
enumeration SHALL exclude `.git`, `node_modules`, `target`, `build`, `dist`,
and `.DS_Store` at any depth, SHALL include dotfiles such as `.gitignore`,
SHALL mark directory entries distinctly from file entries in the produced
view, and SHALL never enumerate files that lie outside every registered
module root.

**Reason**: The six-name default exclusion list and the "dotfiles visible"
contract are both inverted by this change — default exclusion now means
"hide dot-prefixed entries only", which cannot be expressed as an edit of
this requirement without contradicting its own scenarios.

**Migration**: Superseded by the ADDED requirement "Module file
enumeration with default dot exclusions" in this delta, which carries the
enumeration-scope clauses forward and replaces the exclusion semantics.

## ADDED Requirements

### Requirement: Module file enumeration with default dot exclusions

Each module SHALL enumerate files recursively under its own root. The
enumeration SHALL exclude every entry whose name starts with `.` — files
and directories, at any depth — and SHALL NOT exclude other names by
default: `node_modules`, `target`, `build`, and `dist` appear unless the
project's rules hide them. Directory entries SHALL be marked distinctly
from file entries, and files outside every registered module root SHALL
never be enumerated.

#### Scenario: Dot entries absent at any depth

- **WHEN** a module root contains `.git/`, `.gitignore`, and a nested
  `src/.DS_Store`
- **THEN** none of them appears anywhere under its project's rows

#### Scenario: Dependency directories present by default

- **WHEN** a non-git-managed project root contains `node_modules/` and no
  rules hide it
- **THEN** `node_modules` appears in the view as an ordinary directory

#### Scenario: Workspace-owned files are not managed

- **WHEN** the workspace directory contains `.balabala.workspace.json` and
  no project root covers the workspace directory
- **THEN** that file never appears in the view

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

- **WHEN** the rules hide `*.log` and the directory `logs/` contains only
  `.log` files
- **THEN** the `logs` directory row is present with no children

### Requirement: Filter provisioning from the project root

Each module's filter set SHALL be derived from its project. At module
creation the project SHALL detect whether its root is git-managed; if so
it SHALL read the project root's `.gitignore` and provide those rules,
with the default dot ignore, to every module it creates. A non-git root
SHALL not be probed: a `.gitignore` beneath it SHALL have no effect.
Modules below the project root SHALL receive the project's filter set.

#### Scenario: Git-managed project applies its root gitignore

- **WHEN** a project root contains `.git/` and a `.gitignore` that hides
  `*.tmp`, and the project contains `cache.tmp`
- **THEN** `cache.tmp` does not appear in the view

#### Scenario: Non-git project does not read its gitignore

- **WHEN** a project root lacks `.git` but contains a `.gitignore` hiding
  `*.tmp`, and the project contains `cache.tmp`
- **THEN** `cache.tmp` still appears in the view
