# Spec Delta

## RENAMED Requirements

### Requirement: Sidebar session item display
**FROM:** Sidebar session item display
**TO:** Session item display

## MODIFIED Requirements

### Requirement: Sidebar session item display

Each agent item in the solution-overview SHALL be indented under its parent
project row and SHALL display the session's prefixed name (e.g. `foo/pi-1`),
a status badge, and a close button that is visible on hover. The status badge
SHALL reflect one of three states: `starting` (session created but no output
yet), `running` (session has produced output and its agent connection is
still open), or `exited` (the session's `pi` process has ended its agent
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

Selecting an agent item in the solution-overview SHALL switch to the console
tab and make that session's pane the recipient of keyboard input AND IME
input in the same step: the system SHALL claim IME ownership for the pane
immediately upon selection, with no additional click required inside the
pane area. While the session is selected, IME-committed strings SHALL be
routed to that session. The macOS-level application losing focus SHALL
remain the only condition under which keystrokes and IME commits are not
delivered to the active tab's panel.

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

- **WHEN** the user clicks a different agent item in the solution-overview
  and then composes text with an IME without clicking inside the pane area
- **THEN** the committed text is delivered to the newly selected session,
  and no input is delivered to the previously selected session

### Requirement: Manual session close

The user SHALL be able to close a session by clicking its close button in
the solution-overview. Closing SHALL immediately shut down the embedded
session (terminating its `pi` process), remove the item from the
solution-overview, and — if the closed session was active — switch the main
pane to the next remaining session or display an empty placeholder if none
remain.

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
item in the solution-overview until the user explicitly closes it.

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
rendered contents) SHALL claim keyboard focus for that panel. Clicking an
agent row in the solution-overview SHALL switch to the console tab, set the
active selection, AND claim pane keyboard focus for that session in the same
step (one-step focus, consistent with Keyboard input forwarding). While no
pane has focus, the system SHALL NOT enable IME input routing to any
session.

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

### Requirement: Project row and session selection focus model

The center slot SHALL render as a tab group whose content is the active tab;
the console tab SHALL always render the currently selected agent session.
Clicking an agent item in the solution-overview SHALL switch to the console
tab, display that session, and make it the recipient of keyboard input.
Clicking a project row SHALL switch only that row's expanded state and SHALL
NOT switch the active tab.

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

### Requirement: Session control failure feedback

Session control requests initiated from the solution-overview — selecting,
creating, or closing a session — SHALL give visible feedback when a request
fails or times out before taking effect: session state, active selection,
and displayed content SHALL remain as they were, and a notice SHALL name the
failed action. Input injection (keystrokes, wheel, clicks) is
fire-and-forget and SHALL NOT emit failure notices.

#### Scenario: Failed close keeps the session

- **WHEN** the user closes a session but the control request fails or times
  out before the session is closed
- **THEN** the session remains in the sidebar list, the active session and
  console tab content are unchanged, and a notice names the failed close

#### Scenario: Failed selection keeps the active session

- **WHEN** the user clicks a different sidebar session item but the
  selection request fails or times out before it takes effect
- **THEN** the previously active session remains active and keeps receiving
  keystrokes, and a notice names the failed selection

#### Scenario: Failed create shows a notice without a new row

- **WHEN** the user clicks `[+ agent]` but creating the session fails (for
  example the `pi` binary cannot be started)
- **THEN** no new session row appears and a notice names the failed creation

## ADDED Requirements

### Requirement: Solution rows group their projects

Each solution SHALL render as a row above its projects in the
solution-overview, and rows SHALL be ordered by their position in the
solution configuration.

#### Scenario: Solution row sits above its project

- **WHEN** the workspace has one `simple` solution at `/Users/me/code/foo`
  resolving to project `foo`
- **THEN** a solution row renders directly above the project row `foo`

#### Scenario: Multiple solutions order by configuration

- **WHEN** the solutions file lists two solutions
- **THEN** their rows appear in the same order as the file lists them

### Requirement: Solution row identity and label

A solution row SHALL be labelled with the solution's name and its type
marker, so it is distinguishable from the project row beneath it — v1
`simple` solutions resolve to a project rooted at the solution's own path,
so the two rows otherwise carry the same directory name. Its identity SHALL
use a namespace distinct from any path-based row identity, so a solution
and its project never collide as tree entries.

#### Scenario: Type marker distinguishes the twin rows

- **WHEN** a `simple` solution at `/Users/me/code/foo` and its project `foo`
  are both rendered
- **THEN** the solution row's label carries a type marker the project row
  does not

#### Scenario: Solution and project rows are distinct entries

- **WHEN** the solution and its project are both present
- **THEN** they occupy separate rows with distinct identities, and neither
  shadows the other's selection, expansion, or menu

### Requirement: Solution row collapse toggle

Each solution row SHALL toggle the visibility of its projects (and their
agent rows) when clicked. Toggling SHALL NOT change the active session,
SHALL NOT switch the active tab, and SHALL NOT steal keyboard focus from the
current pane. Collapse state SHALL be held per solution in the view and SHALL
NOT be persisted across restarts.

#### Scenario: Collapsed solution hides its projects

- **WHEN** the user collapses a solution row whose projects list agent items
- **THEN** those project and agent rows are hidden while the solution row
  remains visible

#### Scenario: Toggle preserves active session

- **WHEN** the user collapses the solution owning the active session
- **THEN** the main pane still renders that session and keystrokes still
  reach it

#### Scenario: Collapse state does not survive restart

- **WHEN** the user collapses a solution row and restarts the application
- **THEN** the solution row is expanded again after the workspace opens

### Requirement: Solution row context menu offers declared actions

Right-clicking a solution row SHALL open a context menu whose entries are
the actions the solution service declared for that node, built from the
snapshot without contacting the service. Activating an entry SHALL send the
invoke message for that action and node; the solution row SHALL have no
default-open behavior on left click beyond the collapse toggle.

#### Scenario: Menu comes from the service's declarations

- **WHEN** the user right-clicks a solution row for which the service
  declared configuration actions
- **THEN** a menu lists exactly those actions with the service's labels

#### Scenario: Left click toggles rather than opens

- **WHEN** the user left-clicks a solution row
- **THEN** its collapse state toggles and no tab, viewer, or external
  program is opened
