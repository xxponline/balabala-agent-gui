# agent-shell Specification

## Purpose

Provides a desktop GUI shell around `pi` agent sessions: a sidebar manages
multiple sessions (create, select, close, observe lifecycle), and a main pane
renders the active session's terminal output and forwards user input.

## Requirements


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
### Requirement: Keyboard input forwarding

While the application window is focused, the system SHALL deliver input to
the panel of the active center tab. When the console tab is active, that
panel is the active session's input, and the system SHALL deliver: printable
characters (including IME-committed strings), Enter, Backspace, Tab, the four
arrow keys, PageUp, and PageDown. The system SHALL deliver an IME-committed
string as committed text (its UTF-8 characters), not as raw terminal bytes.
Other IME events (preedit composition, surrounding-text deletion) SHALL NOT
be delivered. Ctrl-combination keys SHALL NOT be delivered. When another tab
is active, its panel receives the keystrokes and IME input instead of the
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

#### Scenario: Page keys forwarded for history scrollback

- **WHEN** the user presses PageUp or PageDown while the console tab is
  active and the window is focused
- **THEN** the corresponding paging key is delivered to the active session,
  which pages its message history

#### Scenario: Window unfocused does not forward

- **WHEN** the macOS application window is not the focused window
- **THEN** keystrokes are not delivered to any panel

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
### Requirement: Adaptive terminal grid sizing

The terminal grid's column and row counts SHALL be derived from the measured
pixel area available to the grid and the pane's monospace metrics (cell
advance width and line height), using floor division clamped to a safe
minimum (never zero or an invalid size).

#### Scenario: Window resize changes grid resolution

- **WHEN** the user enlarges the application window while a session is shown
  in the main pane
- **THEN** the grid renders more columns and/or rows in the freed space

#### Scenario: Tiny pane clamps to minimum size

- **WHEN** the pane is shrunk so that the pixel area would compute fewer than
  the minimum columns or rows
- **THEN** the grid uses the minimum dimensions instead of zero or an
  invalid size
### Requirement: Grid resize propagation

When the computed grid dimensions differ from the current dimensions, the
system SHALL propagate the new size to both the embedded interface (so its
TUI re-lays out and redraws) and the session's vt100 screen (so the parsed
screen reflows), in the same step. When dimensions are unchanged, no resize
SHALL be propagated. Sessions in the `exited` state SHALL NOT be resized, so
their preserved last rendered screen is not reflowed.

#### Scenario: Resize reaches the embedded TUI

- **WHEN** a session's grid dimensions change while it is running
- **THEN** the embedded TUI redraws its layout for the new size and the
  following parsed screen reflects the new dimensions

#### Scenario: Unchanged layout propagates nothing

- **WHEN** repaints occur while the grid's computed (columns, rows) equal its
  current dimensions
- **THEN** no resize is propagated to the embedded interface or the vt100
  screen

#### Scenario: Exited session screen preserved

- **WHEN** the window is resized while the active session (or a background
  session) is in the `exited` state
- **THEN** that session's vt100 screen and last rendered frame are unchanged
### Requirement: Mouse wheel scrolls message history

While the pointer hovers the console tab's terminal area, the system SHALL
convert wheel input into a signed line count — negative for scrolling up
into earlier history, positive for scrolling down toward newer content — and
deliver it to the active session.

#### Scenario: Wheel up pages into history

- **WHEN** the user scrolls up over the terminal area while the console tab
  is active
- **THEN** the session receives negative line counts and its message history
  scrolls up

#### Scenario: Wheel down scrolls toward newer content

- **WHEN** the user scrolls down while partway up the history
- **THEN** the session receives positive line counts and the view moves
  toward the newest content
### Requirement: Wheel input follows hover and stops propagation

Wheel input SHALL follow pointer hover, not keyboard focus: nothing SHALL
be delivered when the console panel is not rendered (for example while a
viewer tab is active). Wheel events SHALL stop propagating so outer scroll
containers do not scroll instead.

#### Scenario: Wheel while a viewer tab is active reaches no session

- **WHEN** the user scrolls over a viewer tab (console panel not rendered)
- **THEN** no session receives any wheel input

#### Scenario: Outer container does not scroll instead

- **WHEN** the user scrolls over the terminal area of a panel whose group
  also offers an outer scroll container
- **THEN** the wheel input reaches the session and the outer container's
  scroll position is unchanged
### Requirement: Wheel deltas accumulate to whole lines

Pixel wheel deltas SHALL accumulate across events until a whole line is
formed; fractional remainders SHALL be carried into later events rather
than dropped, so slow scrolling always advances. Line deltas SHALL be
delivered as-is. The accumulator SHALL reset its remainder whenever the
terminal's line height changes.

#### Scenario: Slow pixel scroll advances line by line

- **WHEN** the user performs a slow trackpad scroll whose per-event pixel
  deltas are smaller than one line height
- **THEN** lines are delivered once the accumulated deltas reach a whole
  line, and fractional remainders are carried to the next events

#### Scenario: Remainder resets on line height change

- **WHEN** the terminal grid's line height changes (window resize or font
  metrics) while a fractional remainder is held
- **THEN** the held remainder is discarded rather than applied at the new
  line height
### Requirement: Alt multiplies wheel input

Holding Alt while scrolling SHALL multiply the delivered line count by the
session's alt-wheel multiplier, matching standalone terminal behavior.

#### Scenario: Alt multiplies wheel speed

- **WHEN** the user scrolls one wheel notch while holding Alt
- **THEN** the delivered line count is five times the unmodified count
### Requirement: Terminal area click coordinates forwarded

Left-clicking inside the terminal grid area SHALL deliver the click's
0-based cell coordinate (column, row) to the active session, in the same
coordinate space as the session's rendered frame, unless the gesture is a
drag selection: a left-button press followed by pointer movement beyond the
drag threshold SHALL be treated as a text-selection gesture instead and
SHALL NOT deliver any coordinate. Clicks outside the grid area (status bar,
separators, borders) SHALL NOT deliver a coordinate. The session performs
its own hit-testing on the coordinate; the GUI SHALL NOT interpret what the
coordinate means.

#### Scenario: Click inside the grid delivers a cell coordinate

- **WHEN** the user left-clicks a position inside the terminal grid while
  the console tab is active
- **THEN** the active session receives the corresponding 0-based (column,
  row) cell coordinate

#### Scenario: Drag selection delivers no coordinate

- **WHEN** the user presses inside the terminal grid and drags beyond the
  drag threshold before releasing
- **THEN** a text selection is established (or extended) and the session
  receives no click coordinate for that gesture

#### Scenario: Click outside the grid delivers nothing

- **WHEN** the user left-clicks the status bar or a panel border
- **THEN** no coordinate is delivered to the session

#### Scenario: Clicking a tool card toggles it end to end

- **WHEN** the user left-clicks an expandable tool card rendered in the
  terminal area
- **THEN** the card toggles between its collapsed and expanded states
### Requirement: Terminal area text selection

While the pointer hovers the terminal grid, a left-button press SHALL
anchor a selection at that cell; dragging SHALL extend a focus cell that
follows the pointer, and the visible selection SHALL be rendered as an
overlay highlight covering the cell rectangle between anchor and focus
(normalized so start precedes end, across rows). Coordinates outside the
grid SHALL clamp to the grid edge. The selection state SHALL NOT alter
keyboard focus, and SHALL NOT be forwarded to the session in any form.

#### Scenario: Drag establishes and extends a selection

- **WHEN** the user presses inside the grid and drags to another cell
- **THEN** an overlay highlight covers every cell rectangle from the anchor
  cell to the focus cell, including intermediate rows

#### Scenario: Dragging outside the grid clamps to the edge

- **WHEN** the user drags past the grid's right or bottom edge
- **THEN** the focus cell stops at the last column or row of the grid

#### Scenario: Selection does not steal keyboard focus

- **WHEN** the user completes a drag selection inside the grid
- **THEN** keyboard focus remains on the console pane as it was after any
  other press in the pane area
### Requirement: Selection cleared when its content changes

Selections SHALL be anchored to the visible frame, not to a scrollback
offset. The system SHALL snapshot the selected text while a selection is
established and SHALL compare it against the live frame whenever a new
frame arrives; when the text at the selection's coordinates differs from
the snapshot, the selection SHALL be cleared. This single check covers
every drift source (wheel, paging, streaming push, card expansion, resize,
session switch).

#### Scenario: Streaming output clears a displaced selection

- **WHEN** the user selects a region and the session's streaming output
  pushes that region to different screen coordinates
- **THEN** the next arriving frame clears the selection, so no stale
  highlight remains over wrong content

#### Scenario: Wheel scroll clears the selection when the region moves

- **WHEN** the user scrolls the history while a selection is active and
  the scroll changes the selected region's content
- **THEN** the selection is cleared on the next frame
### Requirement: Selection survives unrelated frame updates

When a new frame arrives and the text at the selection's coordinates still
matches the snapshot, the selection SHALL be preserved — output outside
the selected region SHALL NOT disturb it.

#### Scenario: Unrelated output keeps the selection

- **WHEN** the user has a selection and new output arrives that does not
  change the text at the selection's coordinates
- **THEN** the selection and its overlay highlight remain
### Requirement: Copy selection with Cmd+C

While a selection is active and the console pane has focus, pressing Cmd+C
SHALL copy the selected text to the system clipboard and SHALL NOT modify
the selection. The copied text SHALL be assembled per row from the live
frame — skipping wide-character continuation cells — with trailing
whitespace per row removed and rows joined by newlines. When no selection
is active, Cmd+C SHALL NOT write to the clipboard.

#### Scenario: Copy selected text

- **WHEN** the user selects two lines containing ASCII and CJK text and
  presses Cmd+C
- **THEN** the clipboard contains exactly those cells' text, CJK characters
  appear once (not duplicated by their continuation cell), trailing spaces
  are trimmed, and the lines are joined with a newline

#### Scenario: Cmd+C without selection leaves clipboard untouched

- **WHEN** the user presses Cmd+C while no selection is active
- **THEN** the system clipboard content is unchanged
### Requirement: Cross-platform monospace font resolution

The terminal grid SHALL resolve its monospace font from an ordered list of
font families guaranteed to exist on each supported platform (Windows, macOS,
Linux) rather than assuming a single vendor's font. The same resolved family
SHALL be used for both cell-width measurement and glyph painting, so the two
can never disagree.

#### Scenario: Menlo unavailable on Windows

- **WHEN** the application runs on a Windows host where `Menlo` is not
  installed
- **THEN** the grid resolves a monospace family from the fallback list and
  both cell measurement and text painting use that same resolved family

#### Scenario: Measurement and painting use one resolution

- **WHEN** the grid computes its cell width and later paints a row of text
- **THEN** both operations resolve the font through the same ordered list, so
  the cell width (quantized to the layout pixel grid) is exactly the grid
  the painted glyph positions land on — measurement and painting cannot
  disagree
### Requirement: Resolved font verified as monospace

On first cell-width measurement the system SHALL verify that representative
glyph classes (uppercase letter, lowercase letter, space, and an em dash) share the same advance within a small tolerance, and SHALL emit a
clear diagnostic naming the resolved font when they do not — a non-monospace
fallback must fail loudly at startup instead of drifting silently at render
time.

#### Scenario: Non-monospace fallback is reported

- **WHEN** the resolved font reports different advances for `M`, `a`, the
  space character, and `—`
- **THEN** the application logs a startup diagnostic naming the resolved font
  and the diverging advances, and rendering continues with the drift made
  visible in the log

#### Scenario: Monospace font passes silently

- **WHEN** the resolved font reports equal advances for all sampled glyph
  classes
- **THEN** verification passes with no diagnostic and the grid renders
  normally
### Requirement: No horizontal drift inside the cell grid

For every rendered row, the horizontal position of the glyph painted for
column N SHALL equal N times the cell width (within sub-pixel tolerance),
regardless of which glyph classes precede it on that row. This holds for the
terminal cursor block and for box-drawing borders: both SHALL land exactly on
cell boundaries.

#### Scenario: Cursor stays glued to typed text

- **WHEN** the user types ASCII characters into the session input box
- **THEN** the inverted cursor cell begins exactly one cell after the last
  typed character, with no gap between the text and the cursor block

#### Scenario: Box borders align across mixed-content rows

- **WHEN** a frame contains a bordered box whose rows mix ASCII text, spaces,
  and box-drawing characters
- **THEN** the right border of every row lands on the same cell column, so
  the box renders as a rectangle

#### Scenario: Drift-free rendering with a correct font

- **WHEN** the resolved font is monospace and the grid renders any row
- **THEN** every glyph's painted x position equals its cell index times the
  cell width within sub-pixel tolerance
### Requirement: Real-font metrics regression coverage

The test suite SHALL include at least one test that measures glyph advances
through a real text system (not a mock with uniform advances) and fails when
the resolved font is non-monospace, so a broken font fallback cannot pass CI.

#### Scenario: Non-monospace resolution fails the test suite

- **WHEN** the font resolution under test yields unequal advances for the
  sampled glyph classes
- **THEN** the regression test fails, naming the resolved font

#### Scenario: Mock-based tests are not treated as font evidence

- **WHEN** existing cell-width tests run against the mock text system
- **THEN** they still pass for their own contract but provide no verdict on
  real-font correctness, which the real-metrics test owns
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

### Requirement: Session control failure feedback

Session control requests initiated from the sidebar — selecting, creating, or
closing a session — SHALL give visible feedback when a request fails or times
out before taking effect: session state, active selection, and displayed
content SHALL remain as they were, and a notice SHALL name the failed action.
Input injection (keystrokes, wheel, clicks) is fire-and-forget and SHALL NOT
emit failure notices.

#### Scenario: Failed close keeps the session

- **WHEN** the user closes a session but the control request fails or times
  out before the session is closed
- **THEN** the session remains in the sidebar list, the active session and
  console tab content are unchanged, and a notice names the failed close

#### Scenario: Failed selection keeps the active session

- **WHEN** the user clicks a different sidebar session item but the selection
  request fails or times out before it takes effect
- **THEN** the previously active session remains active and keeps receiving
  keystrokes, and a notice names the failed selection

#### Scenario: Failed create shows a notice without a new row

- **WHEN** the user clicks `[+ agent]` but creating the session fails (for
  example the `pi` binary cannot be started)
- **THEN** no new session row appears and a notice names the failed creation
