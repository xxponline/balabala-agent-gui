# Spec Delta

## Purpose

Provides a desktop GUI shell around `pi` agent sessions: a sidebar manages
multiple sessions (create, select, close, observe lifecycle), and a main pane
renders the active session's terminal output and forwards user input.

## ADDED Requirements

### Requirement: Sidebar session creation

The system SHALL provide a sidebar with a `+` button. Clicking `+` SHALL
create a new agent session, assign it an auto-incrementing name of the form
`pi-N` (where `N` starts at 1 and never reuses a previously issued number),
append it to the sidebar, and make it the active session in the main pane.

#### Scenario: First session

- **WHEN** the user clicks `+` on an empty sidebar
- **THEN** a new item named `pi-1` appears in the sidebar and the main pane
  shows that session's output area

#### Scenario: Multiple sessions

- **WHEN** the user clicks `+` twice in a row
- **THEN** the sidebar contains `pi-1` and `pi-2`, and `pi-2` is the active
  session shown in the main pane

### Requirement: Sidebar session item display

Each sidebar item SHALL display the session name, a status badge, and a
close button that is visible on hover. The status badge SHALL reflect one of
three states: `starting` (subprocess spawned but no output yet), `running`
(subprocess has produced output and not exited), or `exited` (subprocess has
closed its stdout).

#### Scenario: Newly created session shows starting

- **WHEN** a session is created and its subprocess has not yet produced any
  bytes
- **THEN** the sidebar item shows the name with a `starting` badge

#### Scenario: Running session shows running

- **WHEN** a session's subprocess has emitted output and is still alive
- **THEN** the sidebar item shows the name with a `running` badge

#### Scenario: Exited session shows exited

- **WHEN** a session's subprocess closes its stdout (EOF)
- **THEN** the sidebar item switches its badge to `exited` and remains in the
  sidebar until the user closes it

#### Scenario: Close button on hover

- **WHEN** the user hovers the pointer over a sidebar item
- **THEN** a close button (`x`) becomes visible on that item

### Requirement: Session selection focus model

The main pane SHALL always render the currently selected session. Clicking a
sidebar item SHALL switch the main pane to that session AND make it the
recipient of keyboard input. The macOS-level application losing focus SHALL
be the only condition under which keyboard input is not forwarded to the
active session.

#### Scenario: Click sidebar item switches active session

- **WHEN** the user clicks sidebar item `pi-2` while `pi-1` is active
- **THEN** the main pane now renders `pi-2`, and subsequent keystrokes are
  sent to `pi-2`

#### Scenario: Active session visual emphasis

- **WHEN** a session is active
- **THEN** its sidebar item is visually highlighted, and the main pane shows
  a highlighted border or status bar indicating which session is displayed

### Requirement: Active session startup placeholder

While a newly created session is in the `starting` state, the main pane SHALL
display a placeholder (e.g. "waiting for pi to start…") in addition to the
terminal grid. Once the first byte of output arrives, the placeholder SHALL
be removed.

#### Scenario: Placeholder until first output

- **WHEN** the active session is in `starting` state
- **THEN** the main pane shows the placeholder text above or inside the
  terminal area

#### Scenario: Placeholder disappears on first output

- **WHEN** the active session transitions from `starting` to `running`
- **THEN** the placeholder is no longer rendered

### Requirement: Terminal output rendering

The main pane SHALL render the active session's terminal screen cell-by-cell
using monospace text. It SHALL preserve foreground and background colors
emitted by the subprocess, and SHALL draw the terminal cursor as a static
block (foreground/background inverted at the cursor position).

#### Scenario: ANSI colors preserved

- **WHEN** the subprocess emits text with an ANSI color escape sequence
- **THEN** the main pane renders that text in the corresponding color

#### Scenario: Cursor visible

- **WHEN** the active session's vt100 parser reports a cursor position
- **THEN** the cell at that position is rendered with inverted colors

### Requirement: Keyboard input forwarding

While the application window is focused, the system SHALL forward to the
active session: printable characters (including IME-committed strings),
Enter (`\r`), Backspace (`\x7f`), Tab (`\t`), and the four arrow keys
(`\x1b[A` through `\x1b[D`). Ctrl-combination keys SHALL NOT be forwarded
in this change.

#### Scenario: Printable characters forwarded

- **WHEN** the user types "hello" while a session is active and the window
  is focused
- **THEN** the bytes `h e l l o` are sent to that session's subprocess stdin

#### Scenario: Enter forwarded as CR

- **WHEN** the user presses Enter
- **THEN** the byte `\r` is sent to the active session

#### Scenario: Window unfocused does not forward

- **WHEN** the macOS application window is not the focused window
- **THEN** keystrokes are not sent to any session

### Requirement: Manual session close

The user SHALL be able to close a session by clicking its sidebar close
button. Closing SHALL immediately terminate the subprocess (hard kill),
remove the item from the sidebar, and — if the closed session was active —
switch the main pane to the next remaining session or display an empty
placeholder if none remain.

#### Scenario: Closing a non-active session

- **WHEN** the user clicks the close button on an inactive sidebar item
- **THEN** that item disappears from the sidebar and the active session is
  unchanged

#### Scenario: Closing the active session

- **WHEN** the user closes the currently active session
- **THEN** the next sidebar item becomes active and the main pane renders
  it; if no items remain, the main pane shows an empty placeholder

### Requirement: Observed subprocess exit

When a session's subprocess closes its stdout on its own (e.g. the user
runs `/exit` inside `pi`), the system SHALL mark the session as `exited`,
preserve the last rendered screen in the main pane, and keep the item in the
sidebar until the user explicitly closes it.

#### Scenario: Subprocess exits on its own

- **WHEN** the active session's subprocess emits EOF
- **THEN** the badge becomes `exited`, the last screen remains visible, and
  the sidebar item is still present

#### Scenario: User closes an exited session

- **WHEN** the user clicks the close button on an `exited` item
- **THEN** the item is removed from the sidebar (no new subprocess to kill)
