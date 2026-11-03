# Spec Delta

## MODIFIED Requirements

### Requirement: Sidebar session creation

The system SHALL provide a `[+ agent]` button at the bottom of each expanded
workspace's agent list. Clicking it SHALL create a new agent session inside
that workspace, assign it a per-workspace auto-incrementing name of the form
`pi-N` (where `N` starts at 1 within that workspace and never reuses a number
already issued inside that workspace), display it in the sidebar as
`<dirname>/pi-N` (e.g. `foo/pi-1`), spawn the `pi` subprocess with a working
directory equal to the workspace's absolute path, and make it the active
session in the main pane. If no workspace exists yet, no `[+ agent]` button
SHALL be rendered.

#### Scenario: First session

- **WHEN** the user expands a previously empty workspace for `/Users/me/code/foo`
  and clicks `[+ agent]`
- **THEN** a new item labelled `foo/pi-1` appears indented under the workspace,
  the main pane shows that session's output area, and the `pi` subprocess runs
  with cwd `/Users/me/code/foo`

#### Scenario: Multiple sessions

- **WHEN** the user clicks `[+ agent]` twice in a row inside `foo`
- **THEN** the sidebar contains `foo/pi-1` and `foo/pi-2`, and `foo/pi-2` is
  the active session shown in the main pane

#### Scenario: Independent counters per workspace

- **WHEN** the user creates one agent in workspace `foo` and one agent in
  workspace `bar`
- **THEN** the sidebar shows both `foo/pi-1` and `bar/pi-1` as separate items

### Requirement: Sidebar session item display

Each agent item in the sidebar SHALL be indented under its parent workspace
and SHALL display the session's prefixed name (e.g. `foo/pi-1`), a status
badge, and a close button that is visible on hover. The status badge SHALL
reflect one of three states: `starting` (subprocess spawned but no output
yet), `running` (subprocess has produced output and not exited), or `exited`
(subprocess has closed its stdout).

#### Scenario: Newly created session shows starting

- **WHEN** a session is created and its subprocess has not yet produced any
  bytes
- **THEN** the sidebar item shows the prefixed name with a `starting` badge

#### Scenario: Running session shows running

- **WHEN** a session's subprocess has emitted output and is still alive
- **THEN** the sidebar item shows the prefixed name with a `running` badge

#### Scenario: Exited session shows exited

- **WHEN** a session's subprocess closes its stdout (EOF)
- **THEN** the sidebar item switches its badge to `exited` and remains in the
  sidebar until the user closes it

#### Scenario: Close button on hover

- **WHEN** the user hovers the pointer over a sidebar agent item
- **THEN** a close button (`x`) becomes visible on that item

### Requirement: Session selection focus model

The main pane SHALL always render the currently selected agent session.
Clicking an agent item in the sidebar SHALL switch the main pane to that
session AND make it the recipient of keyboard input. Clicking a workspace row
SHALL NOT switch the main pane. The macOS-level application losing focus SHALL
be the only condition under which keyboard input is not forwarded to the
active session. The main pane's status bar SHALL display the active session's
full workspace absolute path, its prefixed agent name, and its status — for
example `/Users/me/code/foo — pi-1 [Running]`.

#### Scenario: Click sidebar item switches active session

- **WHEN** the user clicks sidebar item `bar/pi-1` while `foo/pi-1` is active
- **THEN** the main pane now renders `bar/pi-1`, subsequent keystrokes are
  sent to `bar/pi-1`, and the status bar shows `…/bar — pi-1 [Running]`

#### Scenario: Click workspace row does not switch main pane

- **WHEN** the user clicks a workspace row while an agent in a different
  workspace is active
- **THEN** the main pane continues to render the previously active agent

#### Scenario: Active session visual emphasis

- **WHEN** a session is active
- **THEN** its sidebar item is visually highlighted, and the main pane shows
  a highlighted border or status bar indicating which session is displayed
