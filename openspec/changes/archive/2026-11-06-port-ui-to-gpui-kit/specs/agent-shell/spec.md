# Spec Delta

## MODIFIED Requirements

### Requirement: Keyboard input forwarding

While the application window is focused, the system SHALL forward to the
active session: printable characters (including IME-committed strings),
Enter (`\r`), Backspace (`\x7f`), Tab (`\t`), and the four arrow keys
(`\x1b[A` through `\x1b[D`). The system SHALL forward an IME-committed
string as its raw UTF-8 bytes. Other IME events (preedit composition,
surrounding-text deletion) SHALL NOT be forwarded. Ctrl-combination keys
SHALL NOT be forwarded.

Selecting an agent item in the sidebar SHALL make that session's pane the
recipient of keyboard input AND IME input in the same step: the system
SHALL claim IME ownership for the pane immediately upon selection, with
no additional click required inside the pane area. While the session is
selected, IME-committed strings SHALL be routed to its subprocess. The
macOS-level application losing focus SHALL remain the only condition under
which keystrokes and IME commits are not forwarded.

#### Scenario: Printable characters forwarded

- **WHEN** the user types "hello" while a session is active and the window
  is focused
- **THEN** the bytes `h e l l o` are sent to that session's subprocess stdin

#### Scenario: IME committed text forwarded as UTF-8

- **WHEN** the user composes "你好" with an IME and commits it (e.g. by
  pressing space or enter on the candidate) while the session is selected
- **THEN** the UTF-8 bytes for "你好" are sent to that session's subprocess
  stdin, and the session's terminal shows the committed text

#### Scenario: IME preedit not forwarded

- **WHEN** the user is composing an IME candidate (preedit visible) but has
  not yet committed it
- **THEN** no bytes are sent to the subprocess stdin for the preedit text

#### Scenario: Enter forwarded as CR

- **WHEN** the user presses Enter
- **THEN** the byte `\r` is sent to the active session

#### Scenario: Window unfocused does not forward

- **WHEN** the macOS application window is not the focused window
- **THEN** keystrokes are not sent to any session

#### Scenario: Sidebar selection grants keyboard and IME focus in one step

- **WHEN** the user clicks a different agent item in the sidebar and then
  composes text with an IME without clicking inside the pane area
- **THEN** the committed text is sent to the newly selected session's
  subprocess, and no bytes are sent to the previously selected session

### Requirement: Pane keyboard focus claim

The main pane SHALL own a focusable interactive surface. Clicking anywhere
inside the pane area (including its border and rendered contents) SHALL
claim keyboard focus for that pane. Clicking a sidebar agent item SHALL
switch the active selection AND claim pane keyboard focus for that session
in the same step (one-step focus, consistent with Keyboard input
forwarding). While no pane has focus, the system SHALL NOT enable IME
input routing to any session.

#### Scenario: Click pane claims focus

- **WHEN** the user clicks anywhere inside the main pane area while an agent
  session is active
- **THEN** that pane owns keyboard focus, and IME-committed text will be
  routed to the active session

#### Scenario: Click sidebar switches selection without focus

- **WHEN** the user clicks a sidebar agent row to switch the active session
- **THEN** the main pane renders that agent and its pane claims keyboard
  focus in the same step; no additional click inside the pane area is
  required (the two-step behavior this scenario's name refers to is
  superseded by the one-step model)

#### Scenario: Active vs inactive pane stroke

- **WHEN** an agent session is selected for display in the main pane
- **THEN** the pane border is drawn in the active color (blue); when no
  agent is selected, the pane is not rendered as an interactive surface
