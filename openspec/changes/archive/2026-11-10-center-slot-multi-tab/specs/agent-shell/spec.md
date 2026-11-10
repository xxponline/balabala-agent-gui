# Spec Delta

## MODIFIED Requirements

### Requirement: Session selection focus model

The center slot SHALL render as a tab group whose displayed content is the
active tab. The console tab SHALL always render the currently selected agent
session. Clicking an agent item in the sidebar SHALL switch the center slot
back to the console tab, display that session, AND make it the recipient of
keyboard input. Clicking a workspace row SHALL NOT switch the active tab. The
macOS-level application losing focus SHALL be the only condition under which
keyboard input is not forwarded to the active tab's panel. The console tab's
status bar SHALL display the active session's full workspace absolute path,
its prefixed agent name, and its status — for example
`/Users/me/code/foo — pi-1 [Running]`.

#### Scenario: Click sidebar item switches active session

- **WHEN** the user clicks sidebar item `bar/pi-1` while `foo/pi-1` is active
- **THEN** the console tab becomes the active tab rendering `bar/pi-1`,
  subsequent keystrokes are sent to `bar/pi-1`, and the status bar shows
  `…/bar — pi-1 [Running]`

#### Scenario: Click sidebar item returns from a viewer tab

- **WHEN** the user views `main.rs` in a viewer tab and then clicks sidebar
  item `foo/pi-1`
- **THEN** the center slot switches back to the console tab, which renders
  `foo/pi-1`, and keystrokes are delivered to that session

#### Scenario: Click workspace row does not switch main pane

- **WHEN** the user clicks a workspace row while an agent in a different
  workspace is active
- **THEN** the active tab continues to display what it displayed before

#### Scenario: Active session visual emphasis

- **WHEN** a session is active
- **THEN** its sidebar item is visually highlighted, and the console tab
  shows a highlighted border or status bar indicating which session is
  displayed

### Requirement: Keyboard input forwarding

While the application window is focused, the system SHALL deliver input to
the panel of the active center tab. When the console tab is active, that
panel is the active session's input, and the system SHALL deliver: printable
characters (including IME-committed strings), Enter, Backspace, Tab, and the
four arrow keys. The system SHALL deliver an IME-committed string as
committed text (its UTF-8 characters), not as raw terminal bytes. Other IME
events (preedit composition, surrounding-text deletion) SHALL NOT be
delivered. Ctrl-combination keys SHALL NOT be delivered. When another tab is
active, its panel receives the keystrokes and IME input instead of the
session.

Selecting an agent item in the sidebar SHALL switch to the console tab and
make that session's pane the recipient of keyboard input AND IME input in the
same step: the system SHALL claim IME ownership for the pane immediately upon
selection, with no additional click required inside the pane area. While the
session is selected, IME-committed strings SHALL be routed to that session.
The macOS-level application losing focus SHALL remain the only condition
under which keystrokes and IME commits are not delivered to the active tab's
panel.

#### Scenario: Printable characters forwarded

- **WHEN** the user types "hello" while the console tab is active and the
  window is focused
- **THEN** key input for `h e l l o` is delivered to that session

#### Scenario: Keystrokes follow the active tab

- **WHEN** the user types while a viewer tab is active and the window is
  focused
- **THEN** the keystrokes are delivered to the viewer tab's panel and not to
  any session

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

- **WHEN** the user presses Enter while the console tab is active
- **THEN** the Enter key input is forwarded to the active session

#### Scenario: Window unfocused does not forward

- **WHEN** the macOS application window is not the focused window
- **THEN** keystrokes are not delivered to any panel

#### Scenario: Sidebar selection grants keyboard and IME focus in one step

- **WHEN** the user clicks a different agent item in the sidebar and then
  composes text with an IME without clicking inside the pane area
- **THEN** the committed text is delivered to the newly selected session,
  and no input is delivered to the previously selected session

### Requirement: Terminal output rendering

The console tab SHALL render the active session's terminal screen
cell-by-cell using monospace text. It SHALL preserve foreground and background
colors produced by the session's embedded interface, and SHALL draw the
terminal cursor as a static block (foreground/background inverted at the
cursor position). Every rendered row SHALL have a pixel width exactly equal
to the grid's column count times the monospace cell width: a wide character
(e.g. CJK) SHALL contribute two cells of width, its continuation cell SHALL
NOT be painted as an extra column, and an empty cell SHALL contribute one
cell of width.

#### Scenario: ANSI colors preserved

- **WHEN** the session emits text with an ANSI color escape sequence
- **THEN** the console tab renders that text in the corresponding color

#### Scenario: Cursor visible

- **WHEN** the active session's vt100 parser reports a cursor position
- **THEN** the cell at that position is rendered with inverted colors

#### Scenario: Mixed ASCII/CJK row stays aligned

- **WHEN** a row contains both ASCII characters and CJK characters (e.g.
  `> 帮我解释这段代码`)
- **THEN** the row's rendered width equals its logical column count times the
  cell width, and the row's right edge lines up with rows containing only
  ASCII characters of the same column count

#### Scenario: Wide character continuation not painted

- **WHEN** the vt100 screen reports a cell as a wide-character continuation
- **THEN** that cell contributes no additional rendered width beyond the two
  cells already consumed by its leading wide character

### Requirement: Pane keyboard focus claim

Each center tab's panel SHALL own a focusable interactive surface. Clicking
anywhere inside the active tab's panel area (including its border and
rendered contents) SHALL claim keyboard focus for that panel. Clicking a
sidebar agent item SHALL switch to the console tab, set the active selection,
AND claim pane keyboard focus for that session in the same step (one-step
focus, consistent with Keyboard input forwarding). While no pane has focus,
the system SHALL NOT enable IME input routing to any session.

#### Scenario: Click pane claims focus

- **WHEN** the user clicks anywhere inside the console tab's panel area
  while an agent session is active
- **THEN** that pane owns keyboard focus, and IME-committed text will be
  routed to the active session

#### Scenario: Click viewer tab claims focus for the viewer

- **WHEN** the user clicks inside the viewer tab's panel area
- **THEN** that viewer panel owns keyboard focus and keystrokes are
  delivered to it rather than to any session

#### Scenario: Click sidebar switches selection without focus

- **WHEN** the user clicks a sidebar agent row to switch the active session
- **THEN** the console tab renders that agent and its pane claims keyboard
  focus in the same step; no additional click inside the pane area is
  required (the two-step behavior this scenario's name refers to is
  superseded by the one-step model)

#### Scenario: Active vs inactive pane stroke

- **WHEN** an agent session is selected for display in the console tab
- **THEN** the pane border is drawn in the active color (blue); when no
  agent is selected, the pane is not rendered as an interactive surface
