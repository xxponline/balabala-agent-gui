# Spec Delta

## MODIFIED Requirements

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

## REMOVED Requirements

### Requirement: Workspace switch entry

**Reason**: The left slot no longer provides a workspace switch control. A
workspace is chosen once, at startup, through the Launcher; opening a
different workspace now requires restarting the application. The control, its
`OpenWorkspace`/`SwitchWorkspace` events, and the picker flow it owned have
moved into the Launcher.

**Migration**: Workspace selection and creation are performed in the Launcher
window (see the `launcher` capability). There is no in-session path to switch
workspaces.

### Requirement: Gated workspace switching

**Reason**: The three-way gate (active sessions block / exited-only confirms /
no sessions passes) existed only to guard a switch that no longer exists. With
switching removed, `switch_gate`, its decision type, and the session counting
it fed are unreachable.

**Migration**: None. Sessions are terminated by closing them individually or
by quitting the application; there is no switch to gate.
