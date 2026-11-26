# lua-language-service Specification

## Purpose

Owns the Lua indexing shell: a process-level component that indexes Lua
workspaces, answers symbol-search and diagnostics queries for the
workbench and the agent, and exposes a neutral public API independent of
any specific analysis backend.

## Requirements

### Requirement: Root injection replaces the workspace set

The service SHALL accept a list of workspace root paths and SHALL treat
each accepted list as a full replacement of the previously injected set:
roots removed from the list SHALL no longer contribute files, symbols,
or diagnostics to any query result. An empty list SHALL clear the
workspace entirely.

#### Scenario: Injecting a new root set drops the old one

- **WHEN** a root set of `/a` is injected, then a root set of `/b` is
  injected
- **THEN** queries return only symbols and diagnostics under `/b`, and
  nothing under `/a`

#### Scenario: Empty injection clears everything

- **WHEN** an empty root list is injected after a non-empty one
- **THEN** symbol search and diagnostics return empty results

### Requirement: Symbol search answers by name

The service SHALL provide symbol search that accepts a query string and
returns matching symbols as hits carrying at minimum the symbol name, a
kind (function, variable, field, module, or equivalent classification),
the containing file path, and the line and column of the definition.
Matching SHALL include at least exact and prefix matches on the symbol
name. With no workspace injected, search SHALL return an empty list
rather than failing.

#### Scenario: Prefix query returns definitions

- **WHEN** a workspace containing `on_ready` in `src/net/client.lua` is
  indexed and the user searches for `on_`
- **THEN** a hit for `on_ready` is returned with its file, line, and
  column

#### Scenario: Search with no workspace returns empty

- **WHEN** no root has ever been injected and a search is issued
- **THEN** the result is an empty list and no error is surfaced

### Requirement: Workspace diagnostics cover all indexed files

The service SHALL provide a diagnostics query that returns lint
diagnostics for every indexed file in the injected workspace, each
diagnostic carrying at minimum the file path, position, severity, and
message. With no workspace injected, the result SHALL be an empty list.

#### Scenario: Diagnostics after indexing

- **WHEN** a workspace containing a syntax error is indexed and the
  diagnostics query runs
- **THEN** a diagnostic for that file is returned with its severity and
  message

### Requirement: Analysis runs off the UI thread

Indexing and diagnostics computation SHALL run on a background
thread; queries SHALL read the current indexed state without blocking
the calling thread on an in-progress analysis run.

#### Scenario: Querying during indexing

- **WHEN** a large workspace is still being indexed and a symbol search
  is issued
- **THEN** the query completes against the state indexed so far without
  waiting for indexing to finish

### Requirement: Public API exposes backend-neutral types

The service's public API SHALL NOT expose types originating from the
analysis backend dependency; all returned data (hits, diagnostics,
statuses) SHALL be expressed in the service's own types. Swapping the
backend implementation SHALL not require changes to callers.

#### Scenario: Callers see only service types

- **WHEN** a caller uses symbol search and diagnostics
- **THEN** every returned type is defined by this service, with no
  backend crate types in the signature
