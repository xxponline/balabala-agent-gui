# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
coordinate space as the session's rendered frame. Clicks outside the grid
area (status bar, separators, borders) SHALL NOT deliver a coordinate. The
session performs its own hit-testing on the coordinate; the GUI SHALL NOT
interpret what the coordinate means.

#### Scenario: Click inside the grid delivers a cell coordinate

- **WHEN** the user left-clicks a position inside the terminal grid while
  the console tab is active
- **THEN** the active session receives the corresponding 0-based (column,
  row) cell coordinate

#### Scenario: Click outside the grid delivers nothing

- **WHEN** the user left-clicks the status bar or a panel border
- **THEN** no coordinate is delivered to the session

#### Scenario: Clicking a tool card toggles it end to end

- **WHEN** the user left-clicks an expandable tool card rendered in the
  terminal area
- **THEN** the card toggles between its collapsed and expanded states
