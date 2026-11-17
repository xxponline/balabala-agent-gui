# Spec Delta

## MODIFIED Requirements

### Requirement: Sidebar session item display

Each agent item in the sidebar SHALL be indented under its parent project row
and SHALL display the session's prefixed name (e.g. `foo/pi-1`), a status
badge, and a close button that is visible on hover. The status badge SHALL
reflect one of three states: `starting` (session created but no output yet),
`running` (session has produced output and its agent connection is still
open), or `exited` (the session's `pi` process has ended its agent
connection).

#### Scenario: Newly created session shows starting

- **WHEN** a session is created and has not yet produced any output
- **THEN** the sidebar item shows the prefixed name with a `starting` badge

#### Scenario: Running session shows running

- **WHEN** a session has emitted output and its agent connection is still
  open
- **THEN** the sidebar item shows a `running` badge

#### Scenario: Exited session shows exited

- **WHEN** a session's `pi` process ends its agent connection
- **THEN** the sidebar item switches its badge to `exited` and remains in
  the sidebar until the user closes it

#### Scenario: Close button on hover

- **WHEN** the user hovers the pointer over a sidebar agent item
- **THEN** a close button (`x`) becomes visible on that item

## ADDED Requirements

### Requirement: Project session creation

The system SHALL provide a `[+ agent]` button at the bottom of each project
row's agent list. Clicking it SHALL create a new agent session inside that
project, assign a per-project auto-incrementing name `pi-N` (N starts at 1
per project and is never reused within it), display it as `<project>/pi-N`,
start an embedded session driving the `pi` agent process, and make it the
active session in the main pane. If no project exists, no `[+ agent]` button
SHALL render.

#### Scenario: First session

- **WHEN** the user expands a previously empty project `foo` rooted at
  `/Users/me/code/foo` and clicks `[+ agent]`
- **THEN** a new item labelled `foo/pi-1` appears indented under the project,
  the main pane shows that session's output area, and the session's `pi`
  process runs with cwd `/Users/me/code/foo`

#### Scenario: Multiple sessions

- **WHEN** the user clicks `[+ agent]` twice in a row inside project `foo`
- **THEN** the sidebar contains `foo/pi-1` and `foo/pi-2`, and `foo/pi-2` is
  the active session shown in the main pane

#### Scenario: Independent counters per project

- **WHEN** the user creates one agent in project `foo` and one agent in
  project `bar`
- **THEN** the sidebar shows both `foo/pi-1` and `bar/pi-1` as separate items

#### Scenario: No project renders no button

- **WHEN** the workspace has zero projects
- **THEN** no `[+ agent]` button is rendered anywhere in the sidebar

### Requirement: Session working directory

A newly created session's embedded `pi` process SHALL run with its working
directory equal to the owning project's root path.

#### Scenario: Cwd equals project root

- **WHEN** the user creates a session in project `foo` rooted at
  `/Users/me/code/foo`, a directory outside the workspace
- **THEN** the `pi` process runs with cwd `/Users/me/code/foo`, not the
  workspace directory

### Requirement: Project row and session selection focus model

The center slot SHALL render as a tab group whose content is the active tab;
the console tab SHALL always render the currently selected agent session.
Clicking an agent item in the sidebar SHALL switch to the console tab,
display that session, and make it the recipient of keyboard input. Clicking
a project row SHALL switch only that row's expanded state and SHALL NOT
switch the active tab.

#### Scenario: Click sidebar item switches active session

- **WHEN** the user clicks sidebar item `bar/pi-1` while `foo/pi-1` is active
- **THEN** the console tab becomes the active tab rendering `bar/pi-1`,
  subsequent keystrokes are sent to `bar/pi-1`, and the status bar shows
  bar's project root path followed by `— pi-1 [Running]`

#### Scenario: Click sidebar item returns from a viewer tab

- **WHEN** the user views `main.rs` in a viewer tab and then clicks sidebar
  item `foo/pi-1`
- **THEN** the center slot switches back to the console tab, which renders
  `foo/pi-1`, and keystrokes are delivered to that session

#### Scenario: Click project row does not switch main pane

- **WHEN** the user clicks a project row while an agent of a different
  project is active
- **THEN** the active tab continues to display what it displayed before, and
  only the clicked project row's expansion state changes

#### Scenario: Active session visual emphasis

- **WHEN** a session is active
- **THEN** its sidebar item is visually highlighted, and the console tab
  shows a highlighted border or status bar indicating which session is
  displayed

### Requirement: Console status bar content

The console tab's status bar SHALL display the active session's project root
absolute path, its prefixed agent name, and its status — for example
`/Users/me/code/foo — pi-1 [Running]`.

#### Scenario: Status bar shows project path and status

- **WHEN** session `pi-1` of project `foo` rooted at `/Users/me/code/foo` is
  active and running
- **THEN** the status bar reads `/Users/me/code/foo — pi-1 [Running]`

### Requirement: Project row collapse toggle

Each project row SHALL expose a collapse toggle that hides or reveals its
agent rows. Toggling SHALL NOT change the active session, SHALL NOT switch
the active tab, and SHALL NOT steal keyboard focus from the current pane.

#### Scenario: Collapsed project hides its agents

- **WHEN** the user collapses a project row that lists two agent items
- **THEN** the two agent items are hidden while the project row remains
  visible

#### Scenario: Toggle preserves active session

- **WHEN** the user collapses the project owning the active session
- **THEN** the main pane still renders that session and keystrokes still
  reach it

## REMOVED Requirements

### Requirement: Sidebar session creation
**Reason**: Workspace rows are gone; sessions are now created under project rows with per-project counters and project-root cwd, so the workspace-anchored form of this requirement cannot survive as written (its scenarios name workspace behavior).
**Migration**: See "Project session creation".

### Requirement: Session selection focus model
**Reason**: Its scenarios and status-bar contract are written against workspace rows, which no longer exist; the contract is restated against project rows.
**Migration**: See "Project row and session selection focus model".
