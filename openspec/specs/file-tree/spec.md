# file-tree Specification

## Purpose

The right dock's data view: browse the active workspace's file and directory
tree, filter file rows with gitignore-style rules stored in workspace config,
and expose the deferred default-open hook plus file operations through menus
described by the data source.

## Requirements


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
### Requirement: Left-click open hook

Left-clicking a file row SHALL select it and trigger the default open action
through the unified default-open backend. When a registered handler claims
the file (v1: files with the `rs` extension), the backend SHALL open the file
in a center tab or, if already open, select its existing tab; when no handler
claims the file, the open action SHALL have no visible effect beyond the
selection highlight. Left-clicking a directory row SHALL toggle its expansion.
Row clicks SHALL NOT capture keyboard focus: keystrokes continue to be
delivered to the panel of the active center tab.

#### Scenario: File click selects only

- **WHEN** the user left-clicks a file row no handler claims (e.g.
  `README.md`)
- **THEN** the row becomes highlighted as selected and no other visible
  change occurs

#### Scenario: Rs file click opens viewer tab

- **WHEN** the user left-clicks a file row for `src/main.rs`
- **THEN** the row becomes selected and a viewer tab for `main.rs` opens and
  becomes the active center tab

#### Scenario: Directory click toggles expansion

- **WHEN** the user left-clicks a directory row
- **THEN** the directory expands if it was collapsed and collapses if it was
  expanded

#### Scenario: Keyboard stays with active session

- **WHEN** the user clicks any tree row and then types
- **THEN** the keystrokes are delivered to the active center tab's panel
  (the active session when the console tab is active), not to the tree panel
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
### Requirement: Frame-driven notifications do not rebuild the tree

Consuming a terminal frame (or any other data-change notification) SHALL NOT
rebuild or deep-compare the file-tree view when its inputs have not changed.
The view SHALL be cached and invalidated only by its inputs: the active
workspace root, a newly landed scan, a change to the filter rules, or an
explicit refresh.

#### Scenario: Keystrokes do not pay tree cost

- **WHEN** the user types in a workspace containing several hundred thousand
  files and each keystroke consumes a terminal frame
- **THEN** no tree clone or whole-tree comparison runs for those frames, and
  keystroke-to-paint latency does not grow with the workspace file count

#### Scenario: Unchanged view serves from cache

- **WHEN** frames arrive while the root, scan result, and filter rules are all
  unchanged
- **THEN** the cached view is reused and the tree panel still renders the
  same content

#### Scenario: Inputs changing invalidates the cache

- **WHEN** a background scan lands or the filter rules change
- **THEN** the view is recomputed once and subsequent frames use the new
  cached view
### Requirement: Data-change notification is O(1) when nothing changed

A data-change notification whose inputs are unchanged SHALL perform work
constant in the number of files in the workspace — no per-node traversal of
the tree, no deep equality comparison, and no disk reads.

#### Scenario: Large workspace notification is cheap

- **WHEN** a data-change notification fires for a workspace with 500,000
  files and none of the view's inputs have changed
- **THEN** the notification completes without traversing the scanned tree
### Requirement: In-flight scan is not restarted by notifications

While a background scan for the current root is still running, subsequent
data-change notifications SHALL NOT cancel and restart it. A rescan SHALL be
triggered only by: first load for a root, an actual root change, or an
explicit refresh action.

#### Scenario: Typing during initial scan lets it finish

- **WHEN** the user types repeatedly while the initial scan of a large
  workspace has not yet landed
- **THEN** the original scan continues to completion and lands, rather than
  being restarted by each consumed frame

#### Scenario: Explicit refresh still restarts

- **WHEN** the user clicks the refresh control while a scan is in flight
- **THEN** a new scan is started per the refresh requirement
### Requirement: Tree panel skips work while not rendered

When the right-dock tree panel is not rendered, data-change notifications
SHALL NOT compute or sync the tree view. Rendering the panel again SHALL
show the current cached view without requiring a rescan.

#### Scenario: Hidden panel costs nothing per frame

- **WHEN** frames are consumed while the right dock is closed
- **THEN** no tree view computation or tree-state sync runs

#### Scenario: Reopening the dock shows the current tree

- **WHEN** the user reopens the right dock after frames were consumed while
  it was hidden
- **THEN** the panel displays the tree as of the last invalidation, with no
  rescan triggered solely by having been hidden
