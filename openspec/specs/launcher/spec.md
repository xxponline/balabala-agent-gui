# launcher Specification

## Purpose

Acts as the application's startup gate: it is the only window present before a
workspace is opened, presents the workspace history and the open/create
entries, and refuses to be skipped — closing it interrupts startup outright.

## Requirements

### Requirement: Startup opens the Launcher before any workspace

On startup the application SHALL open the Launcher window as its only window
and SHALL NOT open a workspace or construct the main window until the user
chooses one. Startup SHALL NOT automatically open the most recently used
workspace, whatever the history contains.

#### Scenario: Startup with existing history

- **WHEN** the application starts and workspace history names a valid
  workspace
- **THEN** the Launcher opens with that history available, no workspace is
  loaded, and the main window is not yet constructed

#### Scenario: Startup with no history

- **WHEN** the application starts with no stored workspace history
- **THEN** the Launcher opens with an empty history list and no workspace is
  loaded

### Requirement: Launcher presents workspace history

The Launcher SHALL list previously opened workspaces from the workspace
history, most recently opened first, and SHALL let the user choose one of
them to open.

#### Scenario: History listed most recent first

- **WHEN** the workspace history holds `/a`, then `/b`, then `/a` again
- **THEN** the Launcher lists `/a` ahead of `/b`, with no duplicate entry for
  `/a`

#### Scenario: Choosing a history entry opens that workspace

- **WHEN** the user chooses a history entry whose directory holds a valid
  workspace marker
- **THEN** that workspace opens in the main window

### Requirement: The Launcher validates a choice before handing it off

Before opening the main window for a chosen directory, the Launcher SHALL
check that directory's marker file `.balabala.workspace.json`. A missing
directory, a missing marker, or a marker that cannot be parsed SHALL be
reported inside the Launcher as an error, SHALL leave the Launcher open with
no workspace opened, and SHALL NOT terminate the application. A directory
without a marker SHALL be offered the same creation confirmation it would
receive from the picker.

#### Scenario: Dead history entry stays in the Launcher

- **WHEN** the user chooses a history entry whose directory no longer exists
- **THEN** the error is shown inside the Launcher, the Launcher stays open,
  and the application keeps running

#### Scenario: Unparseable marker stays in the Launcher

- **WHEN** the user chooses a directory whose `.balabala.workspace.json`
  cannot be parsed
- **THEN** the error is shown inside the Launcher, the Launcher stays open,
  and the application keeps running

#### Scenario: Marker-less directory offers creation

- **WHEN** the user chooses an existing directory that has no workspace
  marker
- **THEN** the Launcher offers to create a workspace there, and declining
  leaves the Launcher open with no workspace opened

### Requirement: A workspace lost after validation aborts startup

If the directory the Launcher has already validated becomes unusable before
the main window finishes opening it — it is deleted, or its marker file is
removed or corrupted in that interval — the system SHALL show an error
dialog and exit. This is the only startup path that terminates the
application for an unusable workspace; failures the Launcher can still see
for itself are handled without leaving it.

#### Scenario: Workspace deleted during handoff

- **WHEN** the Launcher validates a directory, opens the main window, and
  that directory is deleted before the workspace load completes
- **THEN** an error dialog describes the failure and the application exits

### Requirement: Launcher offers open and create entries

The Launcher SHALL offer a means to open an existing workspace directory and
a means to create a new workspace, so that a first-time user reaches a usable
workspace without any prior history.

#### Scenario: First-run user creates a workspace

- **WHEN** a user with no history creates a workspace through the Launcher
- **THEN** the workspace is created and opened, and subsequent starts list it
  in the Launcher history

### Requirement: Closing the Launcher interrupts startup

Closing the Launcher before a workspace is chosen SHALL interrupt startup:
no workspace is opened and the application exits. The Launcher SHALL NOT be
skippable — there is no path that reaches the main window without choosing a
workspace through it.

#### Scenario: Closing the Launcher exits the application

- **WHEN** the user closes the Launcher window without choosing a workspace
- **THEN** no workspace is opened and the application process exits

#### Scenario: No path bypasses the Launcher

- **WHEN** the application is started and inspected before any choice is made
- **THEN** no workspace is open and the main window does not exist

### Requirement: No other subsystem starts during the Launcher

Until a workspace is chosen, the application SHALL NOT load services, SHALL
NOT assemble the main window layout, and SHALL NOT open a workspace. Startup
work other than presenting the Launcher is deferred to the moment a workspace
is chosen.

#### Scenario: Services are not loaded while the Launcher is open

- **WHEN** the application is sitting at the Launcher with no choice made
- **THEN** no service instance has been loaded and no workspace is open

#### Scenario: Choosing a workspace starts the rest of startup

- **WHEN** the user chooses a workspace in the Launcher
- **THEN** the main window is constructed and its startup work runs, opening
  that workspace
