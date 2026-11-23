# debug-console Specification

## Purpose

Owns the debug-only console: a registry where in-process GUI modules register
their debug sections, the Debug menu that opens them as center tabs, and the
data contract of the first two sections (term-rate mirror; thread count and
frame-pipeline depth metrics).

## Requirements

### Requirement: Debug sections register only in debug builds

The debug section registry SHALL exist only in debug builds: debug builds
collect every section registered by a module at startup, and release builds
SHALL compile the registry empty with no debug section code retained.
Registration SHALL be scoped to in-process GUI modules; no cross-process or
plugin-boundary registration is provided.

#### Scenario: Debug build lists registered sections

- **WHEN** a debug build starts and modules have registered their sections
- **THEN** the registry contains each registered section with its id and title

#### Scenario: Release build has no debug sections

- **WHEN** a release build starts
- **THEN** the registry is empty and no debug section code is present in the binary

### Requirement: Debug menu opens sections as center tabs

The menu bar SHALL carry a Debug menu — present only in debug builds — whose
items are the registered debug sections. Selecting an item SHALL open that
section as a center tab appended after the console, or focus the existing tab
if it is already open. The Debug menu SHALL be absent from release builds
entirely.

#### Scenario: Opening a section from the menu

- **WHEN** the user selects a section from the Debug menu while its tab is closed
- **THEN** the section opens as a new center tab after the console and becomes active

#### Scenario: Re-selecting an open section focuses it

- **WHEN** the user selects a section from the Debug menu while its tab is open
- **THEN** no second tab is created and the existing tab becomes active

#### Scenario: No Debug menu in release

- **WHEN** the user inspects the menu bar of a release build
- **THEN** no Debug menu is present

### Requirement: Term-rate section mirrors the stderr metrics

The term-rate debug section SHALL display, refreshed at least once per second
while visible, the same metrics as the periodic stderr report: input-to-frame
echo latency (average, max, sample count, misses), frame-to-paint latency
(average, max, sample count), feed and render rates, frame size (average and
max KB), time since last feed, and average render cost.

#### Scenario: Section shows current metrics

- **WHEN** the user opens the term-rate section while the app is rendering frames
- **THEN** the section shows non-zero feed and render rates matching the stderr report

### Requirement: Term-rate section reads without consuming

The section SHALL read a snapshot produced by the reporting path; it SHALL
NOT consume or reset the underlying counters, and the periodic stderr report
SHALL continue unchanged while the section is open.

#### Scenario: Opening the section does not disturb counting

- **WHEN** the user opens and closes the term-rate section repeatedly
- **THEN** the stderr report continues with uninterrupted sample windows

### Requirement: Threads and pipeline section reports OS truth and local depths

The threads/pipeline debug section SHALL show: the process's actual OS-reported
thread count (macOS, Windows, and Linux each via their native mechanism; shown
as unavailable when the platform query fails), the frame drop count reported by
the agent service, the router's in-flight request count, the GUI mailbox
backlog, and the current session count. Values SHALL be sampled only while the
section is visible.

#### Scenario: Thread count reflects spawned sessions

- **WHEN** the user opens the section with several sessions running
- **THEN** the displayed thread count matches the process's real OS thread count and rises with session count

#### Scenario: Metrics refresh only while visible

- **WHEN** the section's tab is closed or inactive
- **THEN** no sampling of OS thread count or pipeline metrics occurs

#### Scenario: Platform query failure degrades to unavailable

- **WHEN** the OS thread count query fails on the current platform
- **THEN** the section displays an unavailable marker instead of a stale or fabricated number
