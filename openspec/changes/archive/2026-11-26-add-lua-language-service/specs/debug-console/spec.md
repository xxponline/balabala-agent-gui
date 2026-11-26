# Spec Delta

## ADDED Requirements

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
