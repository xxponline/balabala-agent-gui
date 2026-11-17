# Spec Delta

## Purpose

Owns the right dock's data as a service-layer asset manager: it receives
project registration from solutions, drives asynchronous asset enumeration
(projects to modules to files), applies project-root-relative filtering,
and produces the view, refresh, and empty/degraded states the panel renders.

## ADDED Requirements

### Requirement: Project registration lifecycle

Projects SHALL be registered into the asset manager when a workspace opens
(all of that workspace's projects, replacing any previous registration),
incrementally when a solution is added, and cleared when the workspace
switches. Registration changes SHALL be reflected in the manager's view
without restarting the application.

#### Scenario: Workspace open registers its projects

- **WHEN** a workspace whose solutions produce two projects opens
- **THEN** both projects are registered and their assets become the
  manager's scope

#### Scenario: Add solution registers incrementally

- **WHEN** the user adds a solution picking `/a/foo` in an open workspace
- **THEN** project `foo` is registered without a restart and its enumeration
  begins

#### Scenario: Workspace switch replaces the registration

- **WHEN** the user switches to a workspace whose solutions produce one
  project
- **THEN** previously registered projects are dropped and no stale rows
  remain in the view

### Requirement: Asynchronous asset enumeration

Registering a project SHALL enqueue an asynchronous enumeration: the
project's assets enumerate to its modules, and each module enumerates its
files. Enumeration SHALL run in the background without blocking interaction,
and its result SHALL land as one complete view.

#### Scenario: Project enumerates its modules

- **WHEN** a project is registered and its enumeration completes
- **THEN** the project's modules are present in the manager's view

#### Scenario: Enumeration does not block interaction

- **WHEN** enumeration of a project with several hundred thousand files is
  in flight
- **THEN** clicking and typing stay responsive and the panel shows its
  previous content (or empty state) until the enumeration lands

### Requirement: Module file enumeration with default exclusions

Each module SHALL enumerate files recursively under its own root. The
enumeration SHALL exclude `.git`, `node_modules`, `target`, `build`, `dist`,
and `.DS_Store` at any depth, SHALL include dotfiles such as `.gitignore`,
SHALL mark directory entries distinctly from file entries in the produced
view, and SHALL never enumerate files that lie outside every registered
module root.

#### Scenario: Dependency directories absent

- **WHEN** a module root contains `node_modules/` and `.git/` directories
- **THEN** neither appears anywhere under its project's rows

#### Scenario: Dotfiles visible

- **WHEN** a module root contains a `.gitignore` file
- **THEN** `.gitignore` appears as a file row in the view

#### Scenario: Workspace-owned files are not managed

- **WHEN** the workspace directory contains `.balabala.workspace.json` and
  no project root covers the workspace directory
- **THEN** that file never appears in the view

### Requirement: File-row filtering relative to project roots

Filter rules SHALL come from the `file_tree` section of the workspace's
`.balabala.workspace.json` and SHALL be applied by the asset manager.
Patterns support wildcards (`*`, `?`, `**`) and path components relative to
each project's root; the last matching rule wins and a `!` prefix
re-includes (gitignore semantics). Rules SHALL apply to file rows only:
directory rows always remain visible. With no section or an empty array
every file row shows.

#### Scenario: Empty rules show everything

- **WHEN** the workspace's `.balabala.workspace.json` has no `file_tree`
  section or an empty array
- **THEN** all non-excluded file rows are visible

#### Scenario: Hide matching files in every project

- **WHEN** the rules are `["*.log"]` and two registered projects each
  contain `.log` files
- **THEN** `.log` rows are hidden under both projects while other file rows
  show, and directory rows remain visible

#### Scenario: Deny-all with re-include

- **WHEN** the rules are `["*", "!*.rs"]`
- **THEN** only `.rs` file rows are shown and every directory row stays visible

#### Scenario: Last match wins

- **WHEN** the rules are `["*", "!*.rs", "*_test.rs"]`
- **THEN** `foo.rs` is shown while `foo_test.rs` is hidden

#### Scenario: Patterns are evaluated per project root

- **WHEN** the rules are `["src/**"]` and two projects both contain
  `src/` directories
- **THEN** both projects' `src` subtrees are matched by the same rule,
  each relative to its own project root

### Requirement: Refresh re-enumerates in the background

The asset manager SHALL re-enumerate on first registration, when the
registered project set changes, and when the panel requests a refresh. A
refresh SHALL run in the background without blocking interaction and SHALL
re-read the workspace's filter rules.

#### Scenario: Refresh picks up new files

- **WHEN** a file is created under a module root and the user clicks the
  refresh control
- **THEN** the new file appears in the view

#### Scenario: Refresh re-reads filter rules

- **WHEN** the user edits the `file_tree` rules in the workspace's
  `.balabala.workspace.json` and clicks refresh
- **THEN** the view applies the newly read rules

### Requirement: Empty and degraded states

When no workspace is open or no projects are registered the manager SHALL
report an empty view. When a module root is missing or unreadable at
enumeration time the manager SHALL report a degraded state naming the
failure, SHALL NOT crash, and SHALL NOT present stale content as current.

#### Scenario: Zero-project workspace is an empty view

- **WHEN** a workspace with no solutions is open
- **THEN** the manager reports an empty view and no enumeration runs

#### Scenario: Deleted module root degrades gracefully

- **WHEN** a project's root directory no longer exists and the user clicks
  refresh
- **THEN** the manager reports a degraded state and the application keeps
  running

### Requirement: Data-change notification is O(1) when nothing changed

A data-change notification whose inputs are unchanged SHALL perform work
constant in the number of enumerated assets — no per-node traversal of the
view, no deep equality comparison, and no disk reads.

#### Scenario: Large asset set notification is cheap

- **WHEN** a data-change notification fires while a project with 500,000
  files is loaded and none of the view's inputs have changed
- **THEN** the notification completes without traversing the enumerated tree

### Requirement: In-flight enumeration is not restarted by notifications

While a background enumeration is still running, subsequent data-change
notifications SHALL NOT cancel and restart it. Re-enumeration SHALL be
triggered only by first registration, an actual registration change, or an
explicit refresh action.

#### Scenario: Typing during initial enumeration lets it finish

- **WHEN** the user types repeatedly while the initial enumeration of a
  large project has not yet landed
- **THEN** the original enumeration continues to completion and lands,
  rather than being restarted by each consumed frame

#### Scenario: Explicit refresh still restarts

- **WHEN** the user clicks the refresh control while an enumeration is in
  flight
- **THEN** a new enumeration is started per the refresh requirement
