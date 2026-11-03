# Spec Delta

## MODIFIED Requirements

### Requirement: Keyboard input forwarding

While the application window is focused, the system SHALL forward to the
active session: printable characters (including IME-committed strings) via
both `Event::Text` and `Event::Ime(ImeEvent::Commit(s))`, Enter (`\r`),
Backspace (`\x7f`), Tab (`\t`), and the four arrow keys (`\x1b[A` through
`\x1b[D`). The system SHALL forward the committed string as its raw UTF-8
bytes. Other IME events (`Preedit`, `DeleteSurrounding`) SHALL NOT be
forwarded in this change. Ctrl-combination keys SHALL NOT be forwarded in
this change.

IME-committed strings SHALL only be forwarded while the active pane holds
keyboard focus (i.e. the user has clicked inside the pane area after
switching to that agent). While the pane is selected but not focused, the
system SHALL NOT claim IME ownership, and IME input SHALL NOT be routed to
the subprocess.

#### Scenario: Printable characters forwarded

- **WHEN** the user types "hello" while a session is active and the window
  is focused
- **THEN** the bytes `h e l l o` are sent to that session's subprocess stdin

#### Scenario: IME committed text forwarded as UTF-8

- **WHEN** the user composes "你好" with an IME and commits it (e.g. by
  pressing space or enter on the candidate) while the pane has keyboard
  focus
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

## ADDED Requirements

### Requirement: Pane keyboard focus claim

The main pane SHALL own a focusable interactive surface. Clicking anywhere
inside the pane area (including its border and rendered contents) SHALL
claim keyboard focus for that pane. Clicking a sidebar item SHALL switch
the active selection but SHALL NOT claim pane keyboard focus. While no pane
has focus, the system SHALL NOT enable IME input routing to any session.

#### Scenario: Click pane claims focus

- **WHEN** the user clicks anywhere inside the main pane area while an agent
  session is active
- **THEN** that pane owns keyboard focus, and IME-committed text will be
  routed to the active session

#### Scenario: Click sidebar switches selection without focus

- **WHEN** the user clicks a sidebar agent row to switch the active session
- **THEN** the main pane renders that agent, but pane keyboard focus is not
  claimed until the user clicks inside the pane area

#### Scenario: Active vs inactive pane stroke

- **WHEN** an agent session is selected for display in the main pane
- **THEN** the pane border is drawn in the active color (blue); when no
  agent is selected, the pane is not rendered as an interactive surface
