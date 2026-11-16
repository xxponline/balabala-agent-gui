# Spec Delta

## ADDED Requirements

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
