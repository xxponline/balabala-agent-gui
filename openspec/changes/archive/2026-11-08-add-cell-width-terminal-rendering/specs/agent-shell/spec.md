# Spec Delta

## MODIFIED Requirements

### Requirement: Terminal output rendering

The main pane SHALL render the active session's terminal screen cell-by-cell
using monospace text. It SHALL preserve foreground and background colors
produced by the session's embedded interface, and SHALL draw the terminal
cursor as a static block (foreground/background inverted at the cursor
position). Every rendered row SHALL have a pixel width exactly equal to the
grid's column count times the monospace cell width: a wide character (e.g.
CJK) SHALL contribute two cells of width, its continuation cell SHALL NOT be
painted as an extra column, and an empty cell SHALL contribute one cell of
width.

#### Scenario: ANSI colors preserved

- **WHEN** the session emits text with an ANSI color escape sequence
- **THEN** the main pane renders that text in the corresponding color

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
