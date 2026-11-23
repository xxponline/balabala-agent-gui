# Spec Delta

## ADDED Requirements

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
