# asset-service Specification

## Purpose

Owns the right dock's asset data as a bus service: it receives project
registration over the layer, drives asynchronous enumeration and
project-root-relative filtering off the handler, and publishes the resulting
view to subscribers — along with the actions each published node offers.

## Requirements

### Requirement: Project registration lifecycle

Projects SHALL be registered into the asset service when a workspace opens
(replacing any previous registration), incrementally when a solution is
added, and cleared when the workspace switches. Registration SHALL come
from the solution service's already-enumerated project tree as project
descriptors; the service SHALL NOT resolve solutions itself. Only projects
whose solution has asset registration enabled SHALL appear. Changes SHALL
reach the published view without restarting the application.

#### Scenario: Workspace open registers its projects

- **WHEN** a workspace whose solutions produce two projects opens
- **THEN** both projects are registered and their assets become the
  service's scope

#### Scenario: Add solution registers incrementally

- **WHEN** the user adds a solution picking `/a/foo` in an open workspace
- **THEN** project `foo` is registered without a restart and its enumeration
  begins

#### Scenario: Workspace switch replaces the registration

- **WHEN** the user switches to a workspace whose solutions produce one
  project
- **THEN** previously registered projects are dropped and no stale rows
  remain in the view

#### Scenario: Registration-disabled solution stays out of the tree

- **WHEN** a solution with asset registration disabled produces one project
  and the workspace opens
- **THEN** that project is never registered and its assets never enter the
  view
### Requirement: Asynchronous asset enumeration

Registering a project SHALL enqueue an asynchronous enumeration: the
project's assets enumerate to its modules, and each module enumerates its
files. Enumeration SHALL run without blocking interaction, and its result
SHALL land as one complete view.

#### Scenario: Project enumerates its modules

- **WHEN** a project is registered and its enumeration completes
- **THEN** the project's modules are present in the service's view

#### Scenario: Enumeration does not block interaction

- **WHEN** enumeration of a project with several hundred thousand files is
  in flight
- **THEN** clicking and typing stay responsive and the panel shows its
  previous content (or empty state) until the enumeration lands
### Requirement: Enumeration does not delay other services

Enumeration SHALL NOT run on the layer's handler while it enumerates: the
work SHALL be handed off so that messages for other services continue to be
processed during a long enumeration. An enumeration of a project with
several hundred thousand files in flight SHALL NOT delay a message sent to
another service by more than normal handler latency.

#### Scenario: Another service stays responsive during enumeration

- **WHEN** a project with several hundred thousand files is enumerating and
  a message is sent to the solution service
- **THEN** the solution service handles it without waiting for the
  enumeration to finish

#### Scenario: Enumeration result still lands as one view

- **WHEN** the handed-off enumeration completes
- **THEN** its result is applied as a single complete view, not streamed
  row by row
### Requirement: Published view reaches subscribers

When the view lands — first registration, a registration change, an
enumeration completing, or an explicit refresh — the service SHALL publish
it to its subscribers as a single complete message. A subscriber SHALL NOT
poll the service to learn the current view, and an enumeration whose inputs
are unchanged SHALL publish nothing.

#### Scenario: Landing the view notifies subscribers once

- **WHEN** an enumeration completes for a registered project
- **THEN** each subscriber receives exactly one message carrying the
  complete view

#### Scenario: Unchanged inputs publish nothing

- **WHEN** a data-change notification fires and none of the view's inputs
  have changed
- **THEN** no view message is published
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
enumeration. With no `.gitignore` every non-dot row shows.

#### Scenario: Empty rules show everything

- **WHEN** the project root is not git-managed or has no `.gitignore`
- **THEN** every row that is not dot-prefixed is visible

#### Scenario: Hide matching files in every project

- **WHEN** each of two git-managed projects' `.gitignore` files hide
  `*.log` and both projects contain `.log` files
- **THEN** `.log` rows are hidden under both projects while other file rows
  show, and directory rows remain visible
### Requirement: Pattern matching follows gitignore semantics

Patterns use wildcards (`*`, `?`, `**`) and path components relative to
each project's root; the last matching rule wins and a `!` prefix
re-includes, a trailing `/` matches directories only, and a bare pattern
matches any entry name at any depth. The default dot ignore SHALL NOT be
re-included by any rule.

#### Scenario: Last match wins

- **WHEN** a project's `.gitignore` lists `*`, then `!*.rs`, then
  `*_test.rs`
- **THEN** `foo.rs` is shown while `foo_test.rs` is hidden

#### Scenario: Patterns are evaluated per project root

- **WHEN** each of two git-managed projects' `.gitignore` files contain
  `src/**` and both projects contain `src/` directories
- **THEN** both projects' `src` subtrees are matched by the same rule,
  each relative to its own project root

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
### Requirement: Excluded directories remove their whole subtree

Rules apply to file and directory rows alike: an excluded directory SHALL
be absent together with its entire subtree, so `!` rules beneath an
excluded directory never take effect while a `!` rule matching the
directory itself re-includes its subtree.

#### Scenario: Deny-all with re-include

- **WHEN** a project's `.gitignore` lists `*` followed by `!*.rs`
- **THEN** only `.rs` file rows at each project's root are shown, nested
  `.rs` rows are absent because every directory is excluded with its
  subtree, matching gitignore's rule that a re-include cannot reach inside
  an excluded directory

#### Scenario: Re-including a directory re-includes its subtree

- **WHEN** a project's `.gitignore` lists `*`, then `!src/`, then `!src/**`
- **THEN** the `src` directory and all of its descendants are shown while
  every other entry is absent

#### Scenario: Re-include beneath an excluded directory has no effect

- **WHEN** a project's `.gitignore` lists `build/` followed by
  `!build/keep.txt` and the project contains `build/keep.txt`
- **THEN** neither `build` nor `build/keep.txt` appears in the view
### Requirement: Refresh re-enumerates in the background

The asset service SHALL re-enumerate on first registration, when the
registered project set changes, and when the panel requests a refresh. A
refresh SHALL run without blocking interaction and SHALL re-read each
project's filter sources (git state and the project root's `.gitignore`).

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
creation the project SHALL use its resolved git-managed flag — enabled
only when its root is a git directory and its solution enables git
management — instead of probing the filesystem. Modules below the project
root SHALL receive the project's filter set.

#### Scenario: Git-managed project applies its root gitignore

- **WHEN** a project root contains `.git/` and a `.gitignore` that hides
  `*.tmp`, and the project contains `cache.tmp`
- **THEN** `cache.tmp` does not appear in the view
### Requirement: Git-managed project provides its root gitignore

A project whose git-managed flag is set SHALL read its root `.gitignore`
and provide those rules, with the default dot ignore, to every module it
creates.

#### Scenario: Rules reach every module

- **WHEN** a git-managed project root's `.gitignore` hides `*.tmp` and the
  project has a nested module
- **THEN** `*.tmp` rows are hidden under both the root and the nested
  module
### Requirement: Unmanaged project is not probed

A project whose git-managed flag is clear SHALL not be probed: a
`.gitignore` beneath it — and a `.git` directory beneath it when the
solution disables git management — SHALL have no effect.

#### Scenario: Non-git project does not read its gitignore

- **WHEN** a project root lacks `.git` but contains a `.gitignore` hiding
  `*.tmp`, and the project contains `cache.tmp`
- **THEN** `cache.tmp` still appears in the view

#### Scenario: Git directory with git management disabled does not read gitignore

- **WHEN** a project root contains `.git/` and a `.gitignore` hiding
  `*.tmp`, its solution disables git management, and the project contains
  `cache.tmp`
- **THEN** `cache.tmp` appears in the view
### Requirement: Empty and degraded states

When no workspace is open or no projects are registered the service SHALL
report an empty view. When a module root is missing or unreadable at
enumeration time the service SHALL report a degraded state naming the
failure, SHALL NOT crash, and SHALL NOT present stale content as current.

#### Scenario: Zero-project workspace is an empty view

- **WHEN** a workspace with no solutions is open
- **THEN** the service reports an empty view and no enumeration runs

#### Scenario: Deleted module root degrades gracefully

- **WHEN** a project's root directory no longer exists and the user clicks
  refresh
- **THEN** the service reports a degraded state and the application keeps
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
### Requirement: File and directory rows declare view-executed actions

The service SHALL declare on each module, directory, and file row the
actions copy path (absolute path) and reveal in Finder, each marked as
executed by the view, and SHALL declare no actions on project rows. The service SHALL
own the display labels; because these actions are view-executed, activating
one SHALL NOT send the service an invoke message.

#### Scenario: File row offers both actions

- **WHEN** the service publishes a file row and the user right-clicks it
- **THEN** the menu offers copy path and reveal in Finder, both marked as
  view-executed, with no invoke message sent to the service

#### Scenario: Project row offers no actions

- **WHEN** the user right-clicks a project row
- **THEN** no context menu opens, because the service declares no actions
  on it
