# Spec Delta

## Purpose

The right dock's data view: browse the active workspace's file and directory
tree, filter file rows with gitignore-style rules stored in workspace config,
and expose the deferred default-open hook plus file operations through menus
described by the data source.

## ADDED Requirements

### Requirement: Tree scope follows active workspace

The tree SHALL be rooted at the absolute path of the workspace containing the
currently active agent. When no agent is active the panel SHALL show an empty
state. Switching the active agent inside the same workspace SHALL NOT rescan;
switching to a different workspace SHALL rescan and replace the tree.

#### Scenario: Tree shows active workspace

- **WHEN** an agent belonging to workspace `/a/proj` is active
- **THEN** the right dock tree shows `/a/proj`'s file and directory structure as its root nodes

#### Scenario: Agent switch inside same workspace keeps tree

- **WHEN** the user switches between two agents of the same workspace while a new file has just appeared on disk
- **THEN** the tree is unchanged and does not include the new file until a refresh

#### Scenario: No active agent shows empty state

- **WHEN** no agent session is active
- **THEN** the right dock body shows an empty-state hint instead of a tree

### Requirement: Default scan exclusions

The scan SHALL exclude `.git`, `node_modules`, `target`, `build`, `dist`, and
`.DS_Store` at any depth, SHALL include dotfiles such as `.gitignore`, and
SHALL render directory rows distinguishably from file rows.

#### Scenario: Dependency directories absent

- **WHEN** the workspace contains `node_modules/` and `.git/` directories
- **THEN** neither appears anywhere in the tree

#### Scenario: Dotfiles visible

- **WHEN** the workspace root contains a `.gitignore` file
- **THEN** `.gitignore` appears as a file row in the tree

### Requirement: File-row filtering with gitignore-style rules

Filter rules SHALL come from the `file_tree` section of `.workspaces.json`,
keyed by the workspace's absolute path, as an array of pattern strings. With
no entry every file row shows. Patterns support wildcards (`*`, `?`, `**`)
and path components relative to the workspace root; the last matching rule
wins and a `!` prefix re-includes (gitignore semantics). Rules SHALL apply
to file rows only: directory rows always remain visible.

#### Scenario: Empty rules show everything

- **WHEN** the workspace has no `file_tree` entry or an empty array
- **THEN** all non-excluded file rows are visible

#### Scenario: Hide matching files

- **WHEN** the rules are `["*.log"]` and `docs/` contains only `.log` files
- **THEN** all `.log` file rows are hidden while other files remain, and the `docs/` directory row is still visible

#### Scenario: Deny-all with re-include

- **WHEN** the rules are `["*", "!*.rs"]`
- **THEN** only `.rs` file rows are shown and every directory row stays visible

#### Scenario: Last match wins

- **WHEN** the rules are `["*", "!*.rs", "*_test.rs"]`
- **THEN** `foo.rs` is shown while `foo_test.rs` is hidden

### Requirement: Left-click open hook

Left-clicking a file row SHALL select it and trigger the default open action;
the v1 open action SHALL have no visible effect beyond the selection
highlight, deferring to the future default-open system. Left-clicking a
directory row SHALL toggle its expansion. Row clicks SHALL NOT capture
keyboard focus: keystrokes continue to be delivered to the active session.

#### Scenario: File click selects only

- **WHEN** the user left-clicks a file row
- **THEN** the row becomes highlighted as selected and no other visible change occurs

#### Scenario: Directory click toggles expansion

- **WHEN** the user left-clicks a directory row
- **THEN** the directory expands if it was collapsed and collapses if it was expanded

#### Scenario: Keyboard stays with active session

- **WHEN** the user clicks any tree row and then types
- **THEN** the keystrokes are delivered to the active session, not to the tree panel

### Requirement: Right-click context menu

Right-clicking any file or directory row SHALL open a context menu whose
items are described by the data source; the file tree source SHALL provide
copy path (absolute path) and reveal in Finder. Activating an item SHALL
dispatch back to the data source for execution.

#### Scenario: Menu opens on right-click

- **WHEN** the user right-clicks a file row
- **THEN** a context menu appears offering copy path and reveal in Finder

#### Scenario: Copy path writes absolute path to clipboard

- **WHEN** the user activates copy path on `/a/proj/src/app.rs`
- **THEN** the system clipboard contains the string `/a/proj/src/app.rs`

#### Scenario: Reveal opens Finder with item selected

- **WHEN** the user activates reveal in Finder on a file row
- **THEN** Finder opens with that file selected

### Requirement: Refresh triggers and behavior

The tree SHALL rescan on first load, when the active workspace root path
changes, and when the user clicks the refresh control. A refresh SHALL run
the scan in the background without blocking interaction, SHALL re-read the
workspace's filter rules, and SHALL preserve the expansion state of
previously expanded directories.

#### Scenario: Refresh picks up new files

- **WHEN** a file is created on disk and the user clicks the refresh control
- **THEN** the new file appears in the tree

#### Scenario: Refresh re-reads filter rules

- **WHEN** the user edits the workspace's `file_tree` rules in `.workspaces.json` and clicks refresh
- **THEN** the tree applies the new rules

#### Scenario: Refresh preserves expansion

- **WHEN** `src/` was expanded before the user clicks refresh
- **THEN** `src/` is still expanded after the refresh completes

### Requirement: Empty and degraded states

When no agent is active the panel SHALL show an empty-state hint. When the
workspace root is missing or unreadable at scan time the panel SHALL show a
degraded-state hint and SHALL NOT crash or present stale content as current.

#### Scenario: Deleted workspace root degrades gracefully

- **WHEN** the workspace directory no longer exists and the user clicks refresh
- **THEN** the panel shows a degraded-state hint and the application keeps running
