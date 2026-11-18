# asset-manager Specification

## Purpose

Owns the right dock's data as a service-layer asset manager: it receives
project registration from solutions, drives asynchronous asset enumeration
(projects to modules to files), applies project-root-relative filtering,
and produces the view, refresh, and empty/degraded states the panel renders.

## Requirements

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
