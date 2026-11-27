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

### Requirement: Lua index section injects roots and shows index state

The Debug menu SHALL list a `lua-index` debug section (debug builds
only, via the existing section registry). The section SHALL offer a root
path injection control whose submission replaces the Lua service's
workspace root set, and SHALL display the current index state: the
number of indexed files, the elapsed indexing time, and whether indexing
is in progress, idle, or failed.

#### Scenario: Injecting a root updates the section

- **WHEN** the user submits a directory path in the section's injection
  control
- **THEN** the Lua service indexes that directory and the section shows
  its file count and elapsed time once indexing completes

#### Scenario: Section appears only in debug builds

- **WHEN** a release build starts
- **THEN** the `lua-index` section is absent from the Debug menu along
  with every other section

### Requirement: Lua index section offers a system folder picker

The lua-index section's root injection control SHALL include a browse
entry point that opens the operating system's default directory selection
dialog. Confirming a directory in the dialog SHALL fill the root input
with the selected path and submit it as the workspace root set. Canceling
the dialog SHALL leave both the root input and the injected workspace
unchanged.

#### Scenario: Picked directory is filled and injected

- **WHEN** the user activates the browse entry point and confirms a
  directory in the system dialog
- **THEN** the root input shows the picked path and the Lua service
  begins indexing it as the workspace root set

#### Scenario: Canceling the dialog writes nothing

- **WHEN** the user cancels the system directory dialog
- **THEN** the root input text and the injected workspace remain
  exactly as they were before

### Requirement: Lua index section runs live symbol search

The section SHALL contain a search input that queries the Lua service's
symbol search as the user types and lists the returned hits (name, kind,
file, line, column) in the section body. With no workspace injected,
the hit list SHALL be empty.

#### Scenario: Typing a query lists hits

- **WHEN** a workspace is injected and the user types a symbol prefix
  into the search input
- **THEN** matching hits appear in the list without any explicit submit
  action

#### Scenario: Search before injection shows nothing

- **WHEN** the section is opened and no root has been injected
- **THEN** the hit list is empty and no error is shown

### Requirement: Lua index section shows workspace diagnostics

The section SHALL display the diagnostics reported by the Lua service
for the injected workspace (file, position, severity, message), refreshed
after each root injection. Diagnostics SHALL be computed in the
background; the section SHALL show its current best state while a
computation is in progress rather than blocking interaction.

#### Scenario: Diagnostics appear after injection

- **WHEN** the user injects a workspace containing lint issues
- **THEN** the diagnostics list populates with those issues once the
  background computation lands

#### Scenario: Interacting while diagnostics compute

- **WHEN** diagnostics for a large workspace are still being computed
- **THEN** the injection control and search input remain responsive
