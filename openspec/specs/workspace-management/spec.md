# workspace-management Specification

## Purpose

Owns the lifecycle of the single workspace the application has open: the
Launcher chooses which workspace to open at startup, workspace-scoped
configuration lives inside that workspace's directory, and the MRU history
persisted across restarts feeds the Launcher's list of previous choices.

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

On startup the system SHALL open the Launcher window rather than opening any
workspace directly, and SHALL NOT open a workspace until the user chooses one
through it. The most recently opened workspace SHALL NOT be restored
automatically. A chosen directory whose marker file exists but cannot be
parsed SHALL be reported inside the Launcher, which stays open with no
workspace opened.

#### Scenario: Fresh start shows picker empty state

- **WHEN** the application starts with no stored workspace history
- **THEN** the Launcher window opens with an empty history list, and no
  workspace is loaded

#### Scenario: History restores last workspace

- **WHEN** the application starts and the most recent history entry names a
  directory whose marker file is valid
- **THEN** the Launcher window opens listing that workspace, and the workspace
  is not opened until the user chooses it

#### Scenario: Deleted history entry falls back to empty state

- **WHEN** the most recent history entry names a directory that no longer
  exists
- **THEN** the Launcher window opens and no error dialog appears, because no
  workspace was opened
### Requirement: Workspace opening via directory picker

Choosing a directory from the Launcher SHALL validate its marker file
`.balabala.workspace.json`: if the file is missing, the system SHALL ask for
confirmation before creating a new workspace; if it exists but cannot be
parsed, the failure SHALL be reported inside the Launcher with no workspace
opened and the application still running; if it is valid, the workspace
SHALL open.

#### Scenario: Valid workspace opens

- **WHEN** the user picks a directory containing a valid
  `.balabala.workspace.json`
- **THEN** that workspace opens and its solution configuration loads

#### Scenario: Non-workspace directory prompts for creation

- **WHEN** the user picks a directory without `.balabala.workspace.json`
- **THEN** a confirmation prompt offers to create a new workspace there, and
  declining returns to the Launcher with no workspace open

#### Scenario: Corrupt marker blocks startup

- **WHEN** the user picks a directory whose `.balabala.workspace.json`
  contains invalid JSON
- **THEN** the failure is described inside the Launcher, the Launcher stays
  open with no workspace open, and the application keeps running
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
lacks the section. The application SHALL NOT rewrite the legacy file. Seeding
SHALL complete before the Launcher presents history.

#### Scenario: Legacy file seeds history once

- **WHEN** the application starts and an app-root `.workspaces.json` lists
  previously used workspaces
- **THEN** those paths seed the workspace history presented by the Launcher,
  and the legacy file is not rewritten

#### Scenario: Legacy filter rules copied into workspace config

- **WHEN** a workspace whose `.balabala.workspace.json` lacks a `file_tree`
  section is opened while the legacy file holds rules for that workspace's
  path
- **THEN** the rules are written into the workspace's configuration and the
  legacy file is unchanged
