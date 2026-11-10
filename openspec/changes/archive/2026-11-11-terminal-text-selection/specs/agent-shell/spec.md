# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
