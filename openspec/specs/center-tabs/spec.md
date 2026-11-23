# center-tabs Specification

## Purpose

Owns the center slot's tab structure and the default-open system: a single
backend entry point that receives wrapped objects from other slots, dispatches
them to registered handlers, and opens deduplicated tabs while the agent
console remains the permanent first tab.

## Requirements

### Requirement: Console is the permanent first center tab

The center slot SHALL render as a tab group whose first tab is the agent
console. The console tab SHALL NOT be closable, SHALL remain present for the
application session, and every tab opened by the default-open system SHALL be
appended after it. The center slot SHALL keep a top bar visible at all
times: while the console is the only tab it SHALL show a title bar carrying
the console's title; once at least one other tab exists the tab bar SHALL
show all tabs.

#### Scenario: Opening a tab keeps console first

- **WHEN** the default-open system opens a viewer tab
- **THEN** the tab bar shows the console tab at index 0 followed by the
  viewer tab, and the console tab has no close affordance

#### Scenario: Closing a viewer tab leaves console alone

- **WHEN** the user closes the only viewer tab
- **THEN** the console tab remains as the sole center tab, the terminal
  is displayed again, and the tab bar still shows the console tab

#### Scenario: Top bar stays visible with only the console tab

- **WHEN** the center slot holds no viewer tabs
- **THEN** the top bar is visible showing the console title, and the
  console carries no close affordance

### Requirement: Default-open dispatch through a unified backend

The system SHALL provide a single backend entry point that accepts a wrapped
object (an absolute path plus its kind) from a source slot, selects the first
registered handler that claims the object, and opens the handler's panel as a
new center tab activated for display. When no registered handler claims the
object, the backend SHALL do nothing visible.

#### Scenario: .rs file opens a viewer tab

- **WHEN** a wrapped object for `/a/proj/src/main.rs` is submitted to the
  backend
- **THEN** the first claiming handler's panel opens as a new center tab and
  becomes the active tab

#### Scenario: Unclaimed object opens nothing

- **WHEN** a wrapped object for a directory or for a file no handler claims
  is submitted to the backend
- **THEN** no tab is created, no existing tab changes, and the center slot
  looks unchanged

### Requirement: Duplicate objects are not reopened

The backend SHALL identify open objects by their absolute path. Submitting an
object whose tab is already open SHALL NOT create a second tab; the backend
SHALL instead select the existing tab. Closing a tab SHALL remove its entry so
the same object can be opened again later.

#### Scenario: Same file twice yields one tab

- **WHEN** the user opens `/a/proj/src/main.rs`, switches to the console tab,
  then opens the same file again
- **THEN** still exactly one `main.rs` tab exists and it becomes the active
  tab

#### Scenario: Reopen after close creates a fresh tab

- **WHEN** the user closes the `main.rs` tab and later opens the same file
  again
- **THEN** a new `main.rs` tab opens

### Requirement: Read-only Rust viewer handler

The first registered handler SHALL claim files whose extension is `rs` and
open them in a read-only code editor tab titled with the file's name. The
editor SHALL display the file's contents with Rust syntax highlighting and
SHALL NOT permit modifying the buffer or saving.

#### Scenario: Viewer shows file content highlighted

- **WHEN** the user opens `/a/proj/src/main.rs` containing Rust source
- **THEN** a tab titled `main.rs` displays the file content with Rust syntax
  highlighting

#### Scenario: Viewer is read-only

- **WHEN** the user types printable characters while the viewer tab is active
- **THEN** the displayed file content is unchanged

#### Scenario: Non-rs file has no viewer

- **WHEN** the user left-clicks a `.toml` file row (neither `rs` nor `md`)
- **THEN** no viewer tab opens

### Requirement: Read-only markdown viewer handler

A registered handler SHALL claim files whose extension is `md` and open them
in a read-only viewer tab titled with the file's name. The viewer SHALL offer
two modes: a rendered view that displays the file's markdown as formatted
content, and a source view that displays the raw markdown with syntax
highlighting. The rendered view SHALL be the mode shown when the tab opens.
Neither mode SHALL permit modifying the buffer or saving.

#### Scenario: md file opens rendered

- **WHEN** the user opens `/a/proj/README.md` containing markdown source
- **THEN** a tab titled `README.md` displays the content as formatted
  markdown (headings, lists, and code blocks rendered), not as raw source

#### Scenario: Source mode shows raw markdown highlighted

- **WHEN** the user switches the open `README.md` tab to source view
- **THEN** the raw markdown source is displayed with markdown syntax
  highlighting and remains unchanged from the file on disk

#### Scenario: Viewer is read-only

- **WHEN** the user types printable characters in either mode of the viewer
- **THEN** the displayed file content is unchanged

### Requirement: Markdown view toggle

The markdown viewer tab SHALL provide a single toggle control in the center
tab bar that switches between rendered and source mode. Toggling SHALL
replace the visible representation in place without reopening the tab or
losing the tab's position. The mode SHALL NOT persist: reopening the file
(after closing its tab) SHALL start in rendered mode again.

#### Scenario: Toggle switches representation in place

- **WHEN** the user clicks the toggle control on the open `README.md` tab
- **THEN** the view switches from rendered to source (or back), the tab
  stays in place and remains active, and no second tab is created

#### Scenario: Reopen resets to rendered

- **WHEN** the user toggles `README.md` to source view, closes the tab, and
  opens the same file again
- **THEN** the new tab shows the rendered view

#### Scenario: Console and non-md tabs have no toggle

- **WHEN** the user looks at the console tab or a `.rs` viewer tab
- **THEN** no markdown toggle control is shown for that tab

### Requirement: Debug section tabs open through the section entry point

The system SHALL provide a center-tab entry point separate from the
file-object backend that accepts a debug section's id and opens the section's
panel as a new center tab appended after the console. Debug section tabs SHALL
be closable, and their dedup identity SHALL be the section id, independent of
the file-path identity used by the file-object backend.

#### Scenario: Section opens after the console

- **WHEN** a debug section is opened while only the console tab exists
- **THEN** the section tab appears at index 1 after the console and the console remains unclosable

#### Scenario: Section identity is independent of file identity

- **WHEN** a debug section whose id happens to equal an open file path is opened
- **THEN** a separate section tab is created; no file tab is focused or replaced

#### Scenario: Closing a section tab removes its dedup entry

- **WHEN** the user closes an open debug section tab and then reopens the same section
- **THEN** a fresh section tab opens rather than an error or a focus of a removed tab

#### Scenario: File backend does not claim section ids

- **WHEN** a debug section id is submitted to the file-object backend
- **THEN** no file tab opens, since the file backend only claims path-plus-kind objects

### Requirement: SolutionConfig tab opens through the add-solution entry

The left-slot add-solution entry SHALL open the SolutionConfig form as a
center slot tab through an entry point separate from the file-object
backend. Its dedup identity SHALL be a fixed form id, independent of file
paths and section ids. While a SolutionConfig tab is open, activating the
entry SHALL select it instead of opening a second one; the tab SHALL be
closable and SHALL reopen fresh after closing.

#### Scenario: Entry opens the form as a center tab

- **WHEN** the user activates the add-solution entry while the console is
  the only center tab
- **THEN** a SolutionConfig tab opens after the console and becomes active,
  and no system file picker appears

#### Scenario: Re-entry selects the single tab

- **WHEN** a SolutionConfig tab is open and the user activates the
  add-solution entry again
- **THEN** the existing SolutionConfig tab becomes active and exactly one
  SolutionConfig tab exists

#### Scenario: Closing the form reopens fresh

- **WHEN** the user closes the SolutionConfig tab and later activates the
  add-solution entry again
- **THEN** a fresh SolutionConfig tab opens
