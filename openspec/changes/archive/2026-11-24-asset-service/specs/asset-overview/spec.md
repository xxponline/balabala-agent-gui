# Spec Delta

## Purpose

The right dock's data view: browse the active workspace's file and directory
tree as published by the asset service, filter file and directory rows with
gitignore-style rules each project provides from its root `.gitignore`,
build each row's menu from the service's declarations, and carry out the
actions that belong to the view layer itself.

## ADDED Requirements

### Requirement: Left-click open hook

Left-clicking a file row SHALL select it and trigger the default open
action. When a registered handler claims the file (v1: files with the `rs`
extension), it SHALL open the file in a center tab or, if already open,
select its existing tab; with no handler it SHALL have no visible effect
beyond the selection highlight. Left-clicking a directory row SHALL toggle
its expansion. Row clicks SHALL NOT capture keyboard focus: keystrokes
continue to the active center tab's panel.

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

Right-clicking a file or directory row SHALL open a context menu whose
items come from the action descriptors the asset service attached to that
node in the published view — copy path (absolute path) and reveal in
Finder — with no message sent to the service to build the menu. Because
both are marked as executed by the view, activating one SHALL carry out in
the view and SHALL send the service no invoke message. Right-clicking a row
the service declared no actions for SHALL open no menu.

#### Scenario: Menu opens on right-click

- **WHEN** the user right-clicks a file row
- **THEN** a context menu appears offering copy path and reveal in Finder

#### Scenario: Menu is built without a round trip

- **WHEN** the user right-clicks a file row
- **THEN** the menu appears without any message being sent to the service
  first

#### Scenario: Copy path writes absolute path to clipboard

- **WHEN** the user activates copy path on `/a/proj/src/app.rs`
- **THEN** the system clipboard contains the string `/a/proj/src/app.rs`,
  and no invoke message was sent to the asset service

#### Scenario: Reveal opens Finder with item selected

- **WHEN** the user activates reveal in Finder on a file row
- **THEN** Finder opens with that file selected, and no invoke message was
  sent to the asset service

#### Scenario: Project row offers no menu

- **WHEN** the user right-clicks a project row
- **THEN** no context menu opens, because the service declared no actions
  on that node

### Requirement: Refresh triggers and behavior

The panel's refresh control SHALL request asset re-enumeration from the
asset service on click; first registration and a changed registered project
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

### Requirement: Frame-driven notifications do not rebuild the tree

Consuming a terminal frame (or any other data-change notification) SHALL NOT
rebuild or deep-compare the asset view when its inputs have not changed.
The view SHALL be cached and invalidated only by its inputs: the published
view arriving from the service, a change to the filter rules, or an explicit
refresh.

#### Scenario: Keystrokes do not pay tree cost

- **WHEN** the user types in a workspace containing several hundred thousand
  files and each keystroke consumes a terminal frame
- **THEN** no tree clone or whole-tree comparison runs for those frames, and
  keystroke-to-paint latency does not grow with the asset count

#### Scenario: Unchanged view serves from cache

- **WHEN** frames arrive while the published view and the filter rules are
  unchanged
- **THEN** the cached view is reused and the tree panel still renders the
  same content

#### Scenario: Inputs changing invalidates the cache

- **WHEN** a published view arrives from the service or the filter rules
  change
- **THEN** the view is recomputed once and subsequent frames use the new
  cached view

### Requirement: Tree panel skips work while not rendered

When the right-dock tree panel is not rendered, data-change notifications
SHALL NOT compute or sync the tree view from the published view. Rendering
the panel again SHALL show the current cached view without requiring a
re-enumeration.

#### Scenario: Hidden panel costs nothing per frame

- **WHEN** frames are consumed while the right dock is closed
- **THEN** no tree view computation or tree-state sync runs

#### Scenario: Reopening the dock shows the current tree

- **WHEN** the user reopens the right dock after frames were consumed while
  it was hidden
- **THEN** the panel displays the tree as of the last invalidation, with no
  re-enumeration triggered solely by having been hidden

### Requirement: Tree scope follows registered projects

The tree SHALL render the asset service's registered project set as its root
nodes and SHALL NOT depend on an active agent session: the view SHALL appear
whenever the service reports registered projects. A workspace switch SHALL
replace the roots through re-registration, and switching the active session
within the same project SHALL NOT rescan or replace the tree.

#### Scenario: Tree shows registered projects

- **WHEN** a workspace with two registered projects is open while any session
  (or none) is active
- **THEN** the right dock tree shows both projects' rows as root nodes

#### Scenario: No registered projects shows empty state

- **WHEN** a workspace with zero projects is open
- **THEN** the right dock body shows an empty-state hint instead of a tree

#### Scenario: Session switching leaves the tree alone

- **WHEN** the user switches the active session inside one project while a
  new file has just appeared on disk
- **THEN** the tree is unchanged and does not include the new file until a
  refresh

### Requirement: Empty and degraded display

The panel SHALL show an empty-state hint when the asset service reports an
empty view, and a degraded-state hint when the service reports degraded.
The panel SHALL NOT crash and SHALL NOT present stale content as current.

#### Scenario: Empty view shows hint

- **WHEN** the service reports an empty view
- **THEN** the panel body shows the empty-state hint instead of a tree

#### Scenario: Degraded view shows hint

- **WHEN** the service reports a degraded state after a module root became
  unreadable
- **THEN** the panel shows a degraded-state hint and the application keeps
  running
