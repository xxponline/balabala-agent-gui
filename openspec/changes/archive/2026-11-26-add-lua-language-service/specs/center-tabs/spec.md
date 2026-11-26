# Spec Delta

## ADDED Requirements

### Requirement: Read-only Lua viewer handler

A registered handler SHALL claim files whose extension is `lua` and open
them in a read-only code editor tab titled with the file's name. The
editor SHALL display the file's contents with Lua syntax highlighting
and SHALL NOT permit modifying the buffer or saving.

#### Scenario: Opening a Lua file highlights it

- **WHEN** a wrapped object for `/a/proj/src/net.lua` containing Lua
  source is submitted to the default-open backend
- **THEN** a read-only editor tab titled `net.lua` opens displaying the
  contents with Lua syntax highlighting

#### Scenario: Lua tab is read-only and deduplicated

- **WHEN** the `net.lua` tab is open and the same object is submitted
  again
- **THEN** no second tab is created, the existing tab is selected, and
  the buffer remains unmodifiable
