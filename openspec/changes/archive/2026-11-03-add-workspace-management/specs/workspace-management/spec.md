# Spec Delta

## Purpose

Owns the list of on-disk workspaces shown in the sidebar: each workspace wraps
an absolute path that `pi` agents inside it inherit as their working directory,
and the list is persisted to a JSON file across app restarts.

## ADDED Requirements

### Requirement: Workspace creation via folder picker

The system SHALL provide a `+ new workspace` button at the top of the sidebar.
Clicking it SHALL open a native folder picker. On a picked folder the system
SHALL append a new workspace to the sidebar, default it to the expanded state,
and immediately persist the updated list. Cancelling the picker SHALL leave
the sidebar unchanged.

#### Scenario: First workspace

- **WHEN** the user clicks `+ new workspace` on an empty sidebar and picks
  `/Users/me/code/foo`
- **THEN** a new workspace row labelled `foo` appears in the sidebar, expanded,
  and the on-disk persistence file contains `/Users/me/code/foo`

#### Scenario: Cancelled picker

- **WHEN** the user clicks `+ new workspace` and then cancels the native picker
- **THEN** the sidebar is unchanged and the persistence file is not rewritten

### Requirement: Workspace path dedup

The system SHALL treat an absolute path as the unique identity of a workspace.
If the user picks a folder whose canonical absolute path is already present,
the system SHALL NOT add a duplicate workspace and SHALL surface the rejection
to the user (log line or on-screen notice).

#### Scenario: Duplicate path rejected

- **WHEN** a workspace for `/Users/me/code/foo` already exists and the user picks
  the same folder through the picker
- **THEN** no new workspace row appears, the sidebar order is unchanged, and
  the persistence file is unchanged

### Requirement: Workspace row display

Each workspace row in the sidebar SHALL display only the last path component
(e.g. `foo` for `/Users/me/code/foo`). Hovering the row SHALL show a tooltip
with the absolute path. The row SHALL also expose a collapse toggle (`▶` /
`▼`) and a close button (`×`) that is visible on hover.

#### Scenario: Row shows short name, tooltip shows full path

- **WHEN** the user hovers a workspace row whose path is `/Users/me/code/foo`
- **THEN** the row itself displays `foo`, and a tooltip reveals the absolute
  path

### Requirement: Workspace collapse toggle

Clicking a workspace row SHALL only toggle its expanded/collapsed state. It
SHALL NOT change the active session in the main pane, and SHALL NOT steal
keyboard focus from the currently active agent.

#### Scenario: Click toggles without changing main pane

- **WHEN** the user clicks a collapsed workspace row while another workspace's
  agent is active in the main pane
- **THEN** the workspace row expands to reveal its agents, and the main pane
  still renders the previously active agent

### Requirement: Workspace deletion when empty

The system SHALL allow the user to delete a workspace via its `×` button only
when the workspace has zero agent sessions. If the workspace still has agents
(active or exited), the `×` button SHALL be disabled (greyed out) and SHALL
NOT respond to clicks. Deleting a workspace SHALL immediately persist the
updated list.

#### Scenario: Deleting an empty workspace

- **WHEN** the user clicks `×` on a workspace that has no agents
- **THEN** the workspace row is removed from the sidebar and the persistence
  file no longer lists its path

#### Scenario: Deletion blocked while agents exist

- **WHEN** the user hovers a workspace row that still has one or more agents
- **THEN** the `×` button is visibly disabled and clicking it does not remove
  the workspace

### Requirement: Workspace persistence to JSON file

The system SHALL persist the list of workspace paths to `.workspaces.json` in
the project root on every mutation (workspace added or removed). The file
format SHALL be a JSON object with a single `workspaces` array of absolute
path strings. Collapse state and agent sessions SHALL NOT be persisted.

#### Scenario: Add triggers write

- **WHEN** the user adds a new workspace
- **THEN** `.workspaces.json` in the project root is rewritten and contains the
  new absolute path

#### Scenario: Remove triggers write

- **WHEN** the user deletes an existing empty workspace
- **THEN** `.workspaces.json` in the project root is rewritten and no longer
  contains the removed path

### Requirement: Workspace persistence load on startup

On startup the system SHALL read `.workspaces.json` from the project root. If
the file does not exist, the system SHALL start with an empty workspace list.
If the file exists but cannot be parsed or read, the system SHALL show an
error dialog and exit; it SHALL NOT silently fall back to an empty list.

#### Scenario: Missing file starts empty

- **WHEN** `.workspaces.json` does not exist on startup
- **THEN** the app starts normally with an empty sidebar

#### Scenario: Corrupt file blocks startup

- **WHEN** `.workspaces.json` exists but contains invalid JSON
- **THEN** the app displays an error dialog describing the failure and exits
  without rendering the main window

### Requirement: Persistence write failure surfacing

If writing `.workspaces.json` fails (e.g. disk full, permission denied), the
system SHALL keep the in-memory workspace list intact and SHALL surface the
failure to the user as an on-screen error indicator (red status bar or
tooltip). It SHALL NOT panic and SHALL NOT roll back the in-memory change.

#### Scenario: Save failure shown in UI

- **WHEN** the user adds a workspace and the subsequent write to
  `.workspaces.json` fails
- **THEN** the new workspace still appears in the sidebar, and a red error
  indicator is visible explaining the save failure
