# workspace-management Specification

## Purpose

Owns the list of on-disk workspaces shown in the sidebar: each workspace wraps
an absolute path that `pi` agents inside it inherit as their working directory,
and the list is persisted to a JSON file across app restarts.

## Requirements


### Requirement: Persistence write failure surfacing

If writing a workspace-scoped configuration file fails (e.g. disk full,
permission denied), the system SHALL keep the in-memory state intact and
SHALL surface the failure to the user as an on-screen error indicator (red
status bar or tooltip). It SHALL NOT panic and SHALL NOT roll back the
in-memory change.

#### Scenario: Save failure shown in UI

- **WHEN** the user adds a solution and the subsequent write to the
  workspace's solutions file fails
- **THEN** the in-memory state keeps the change, and a red error indicator
  is visible explaining the save failure
### Requirement: Startup workspace restore

On startup the system SHALL open the most recently opened workspace directly
into the main window when a stored history entry exists and its marker file
is valid. With no history, or when the most recent entry's directory no longer
exists or lacks a marker file, the system SHALL render the main window with an
in-window empty state offering a directory picker. If the most recent entry's
marker file exists but cannot be parsed, the system SHALL show an error dialog
and exit.

#### Scenario: Fresh start shows picker empty state

- **WHEN** the application starts with no stored workspace history
- **THEN** the main window opens showing an empty state with a directory
  picker, and no workspace is loaded

#### Scenario: History restores last workspace

- **WHEN** the application starts and the most recent history entry names a
  directory whose marker file is valid
- **THEN** the main window opens directly with that workspace loaded and no
  picker shown

#### Scenario: Deleted history entry falls back to empty state

- **WHEN** the most recent history entry names a directory that no longer
  exists
- **THEN** the main window opens showing the empty-state picker and no error
  dialog appears
### Requirement: Workspace opening via directory picker

Choosing a directory (from the startup empty state or the switch entry) SHALL
validate its marker file `.balabala.workspace.json`: if the file is missing,
the system SHALL ask for confirmation before creating a new workspace; if it
exists but cannot be parsed, the system SHALL show an error dialog and exit;
if it is valid, the workspace SHALL open.

#### Scenario: Valid workspace opens

- **WHEN** the user picks a directory containing a valid
  `.balabala.workspace.json`
- **THEN** that workspace opens and its solution configuration loads

#### Scenario: Non-workspace directory prompts for creation

- **WHEN** the user picks a directory without `.balabala.workspace.json`
- **THEN** a confirmation prompt offers to create a new workspace there, and
  declining leaves no workspace open

#### Scenario: Corrupt marker blocks startup

- **WHEN** the user picks a directory whose `.balabala.workspace.json`
  contains invalid JSON
- **THEN** an error dialog describes the failure and the application exits
### Requirement: Workspace creation writes identity and solutions files

Confirming creation SHALL write `.balabala.workspace.json` (with a format
version) and an empty `balabala.solutions.json` into the chosen directory,
then open it as the current workspace. Creation SHALL be atomic from the
user's perspective: if either write fails, no workspace is opened and the
failure is surfaced per the write-failure contract.

#### Scenario: Confirmed creation opens the workspace

- **WHEN** the user confirms creation for `/a/new`
- **THEN** `/a/new/.balabala.workspace.json` and
  `/a/new/balabala.solutions.json` exist and the workspace opens empty

#### Scenario: Write failure surfaces without opening

- **WHEN** creation is confirmed but writing the solutions file fails
- **THEN** the failure is surfaced as an on-screen error and no workspace is
  opened
### Requirement: Workspace switch entry

The left slot SHALL provide a workspace switch control at its top (the entry
point that replaces the former per-workspace sidebar rows). Activating it
SHALL follow the same directory-picker and validation path as startup
selection; switching SHALL first satisfy the gating rules for the currently
open workspace's sessions.

#### Scenario: Switch entry opens the picker

- **WHEN** the user activates the switch control while a workspace is open
- **THEN** a directory picker opens, and cancelling it keeps the current
  workspace unchanged
### Requirement: Gated workspace switching

Switching away SHALL be blocked while any session of the current workspace is
in the `starting` or `running` state, with an on-screen notice naming the
count of active sessions and leaving the workspace untouched. When only
`exited` sessions remain, switching SHALL require a confirmation that states
their preserved terminal screens will be discarded; confirming discards them
and proceeds. With no sessions, switching SHALL proceed directly.

#### Scenario: Active sessions block the switch

- **WHEN** the user activates workspace switching while two sessions are
  `running`
- **THEN** a notice reports 2 active sessions, no picker opens, and the
  current workspace is unchanged

#### Scenario: Exited sessions require confirmation

- **WHEN** the user activates switching and the workspace has only `exited`
  sessions
- **THEN** a confirmation warns that those sessions' screens will be
  discarded; confirming proceeds to the picker and cancelling stays

#### Scenario: No sessions switch directly

- **WHEN** the user activates switching with zero sessions open
- **THEN** the directory picker opens immediately with no confirmation
### Requirement: Workspace data lives in the workspace directory

All workspace-scoped state SHALL be stored inside the workspace directory
(marker and configuration files now; cache and worktree data in later
changes). The application root SHALL NOT store workspace configuration.

#### Scenario: Config written inside workspace

- **WHEN** workspace configuration changes after the workspace is open
- **THEN** the write lands in files under the workspace directory, and no
  workspace file in the application root is modified
### Requirement: Legacy .workspaces.json migration

The legacy `.workspaces.json` SHALL be used as a one-time migration source:
its list seeds workspace history (completion recorded in the history store),
and its `file_tree` rules for a workspace SHALL be copied into that
workspace's configuration when the workspace is opened and its configuration
lacks the section. The application SHALL NOT rewrite the legacy file.

#### Scenario: Legacy file seeds history once

- **WHEN** the application starts and an app-root `.workspaces.json` lists
  previously used workspaces
- **THEN** those paths seed the workspace history used by startup restore,
  and the legacy file is not rewritten

#### Scenario: Legacy filter rules copied into workspace config

- **WHEN** a workspace whose `.balabala.workspace.json` lacks a `file_tree`
  section is opened while the legacy file holds rules for that workspace's
  path
- **THEN** the rules are written into the workspace's configuration and the
  legacy file is unchanged
