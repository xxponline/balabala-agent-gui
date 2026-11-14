# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
