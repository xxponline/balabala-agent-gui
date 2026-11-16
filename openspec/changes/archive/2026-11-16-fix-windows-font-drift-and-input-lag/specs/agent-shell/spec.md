# Spec Delta

## ADDED Requirements

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
