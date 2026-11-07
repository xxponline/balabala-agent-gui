# Spec Delta

## MODIFIED Requirements

### Requirement: Sidebar session creation

The system SHALL provide a `[+ agent]` button at the bottom of each expanded
workspace's agent list. Clicking it SHALL create a new agent session inside
that workspace, assign it a per-workspace auto-incrementing name of the form
`pi-N` (where `N` starts at 1 within that workspace and never reuses a number
already issued inside that workspace), display it in the sidebar as
`<dirname>/pi-N` (e.g. `foo/pi-1`), start an embedded session for it that
drives the `pi` agent process over its RPC interface with a working directory
equal to the workspace's absolute path, and make it the active session in the
main pane. If no workspace exists yet, no `[+ agent]` button SHALL be rendered.

#### Scenario: First session

- **WHEN** the user expands a previously empty workspace for `/Users/me/code/foo`
  and clicks `[+ agent]`
- **THEN** a new item labelled `foo/pi-1` appears indented under the workspace,
  the main pane shows that session's output area, and the session's `pi`
  process runs with cwd `/Users/me/code/foo`

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
reflect one of three states: `starting` (session created but no output yet),
`running` (session has produced output and its agent connection is still
open), or `exited` (the session's `pi` process has ended its agent
connection).

#### Scenario: Newly created session shows starting

- **WHEN** a session is created and has not yet produced any output
- **THEN** the sidebar item shows the prefixed name with a `starting` badge

#### Scenario: Running session shows running

- **WHEN** a session has emitted output and its agent connection is still open
- **THEN** the sidebar item shows the prefixed name with a `running` badge

#### Scenario: Exited session shows exited

- **WHEN** a session's `pi` process ends its agent connection
- **THEN** the sidebar item switches its badge to `exited` and remains in
  the sidebar until the user closes it

#### Scenario: Close button on hover

- **WHEN** the user hovers the pointer over a sidebar agent item
- **THEN** a close button (`x`) becomes visible on that item

### Requirement: Terminal output rendering

The main pane SHALL render the active session's terminal screen cell-by-cell
using monospace text. It SHALL preserve foreground and background colors
produced by the session's embedded interface, and SHALL draw the terminal
cursor as a static block (foreground/background inverted at the cursor
position).

#### Scenario: ANSI colors preserved

- **WHEN** the session emits text with an ANSI color escape sequence
- **THEN** the main pane renders that text in the corresponding color

#### Scenario: Cursor visible

- **WHEN** the active session's vt100 parser reports a cursor position
- **THEN** the cell at that position is rendered with inverted colors

### Requirement: Keyboard input forwarding

While the application window is focused, the system SHALL deliver to the
active session's input: printable characters (including IME-committed
strings), Enter, Backspace, Tab, and the four arrow keys. The system SHALL
deliver an IME-committed string to the session as committed text (its UTF-8
characters), not as raw terminal bytes. Other IME events (preedit
composition, surrounding-text deletion) SHALL NOT be delivered. Ctrl-combination
keys SHALL NOT be delivered.

Selecting an agent item in the sidebar SHALL make that session's pane the
recipient of keyboard input AND IME input in the same step: the system
SHALL claim IME ownership for the pane immediately upon selection, with
no additional click required inside the pane area. While the session is
selected, IME-committed strings SHALL be routed to that session. The
macOS-level application losing focus SHALL remain the only condition under
which keystrokes and IME commits are not delivered.

#### Scenario: Printable characters forwarded

- **WHEN** the user types "hello" while a session is active and the window
  is focused
- **THEN** key input for `h e l l o` is delivered to that session

#### Scenario: IME committed text forwarded as UTF-8

- **WHEN** the user composes "你好" with an IME and commits it (e.g. by
  pressing space or enter on the candidate) while the session is selected
- **THEN** the UTF-8 characters of "你好" are delivered to that session as
  committed text input, and the session's terminal shows the committed text

#### Scenario: IME preedit not forwarded

- **WHEN** the user is composing an IME candidate (preedit visible) but has
  not yet committed it
- **THEN** no input is delivered to the session for the preedit text

#### Scenario: Enter forwarded as CR

- **WHEN** the user presses Enter
- **THEN** the Enter key input is forwarded to the active session

#### Scenario: Window unfocused does not forward

- **WHEN** the macOS application window is not the focused window
- **THEN** keystrokes are not delivered to any session

#### Scenario: Sidebar selection grants keyboard and IME focus in one step

- **WHEN** the user clicks a different agent item in the sidebar and then
  composes text with an IME without clicking inside the pane area
- **THEN** the committed text is delivered to the newly selected session,
  and no input is delivered to the previously selected session

### Requirement: Manual session close

The user SHALL be able to close a session by clicking its sidebar close
button. Closing SHALL immediately shut down the embedded session
(terminating its `pi` process), remove the item from the sidebar, and — if
the closed session was active — switch the main pane to the next remaining
session or display an empty placeholder if none remain.

#### Scenario: Closing a non-active session

- **WHEN** the user clicks the close button on an inactive sidebar item
- **THEN** that item disappears from the sidebar and the active session is
  unchanged

#### Scenario: Closing the active session

- **WHEN** the user closes the currently active session
- **THEN** the next sidebar item becomes active and the main pane renders
  it; if no items remain, the main pane shows an empty placeholder

### Requirement: Observed subprocess exit

When a session's `pi` process ends its agent connection on its own (e.g. the
user runs `/exit` inside `pi`), the system SHALL mark the session as
`exited`, preserve the last rendered screen in the main pane, and keep the
item in the sidebar until the user explicitly closes it.

#### Scenario: Subprocess exits on its own

- **WHEN** the active session's `pi` process closes its agent connection
- **THEN** the badge becomes `exited`, the last screen remains visible, and
  the sidebar item is still present

#### Scenario: User closes an exited session

- **WHEN** the user clicks the close button on an `exited` item
- **THEN** the item is removed from the sidebar (no `pi` process left to
  terminate)
