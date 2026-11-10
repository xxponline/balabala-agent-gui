# Spec Delta

## MODIFIED Requirements

### Requirement: Left-click open hook

Left-clicking a file row SHALL select it and trigger the default open action
through the unified default-open backend. When a registered handler claims
the file (v1: files with the `rs` extension), the backend SHALL open the file
in a center tab or, if already open, select its existing tab; when no handler
claims the file, the open action SHALL have no visible effect beyond the
selection highlight. Left-clicking a directory row SHALL toggle its expansion.
Row clicks SHALL NOT capture keyboard focus: keystrokes continue to be
delivered to the panel of the active center tab.

#### Scenario: File click selects only

- **WHEN** the user left-clicks a file row no handler claims (e.g.
  `README.md`)
- **THEN** the row becomes highlighted as selected and no other visible
  change occurs

#### Scenario: Rs file click opens viewer tab

- **WHEN** the user left-clicks a file row for `src/main.rs`
- **THEN** the row becomes selected and a viewer tab for `main.rs` opens and
  becomes the active center tab

#### Scenario: Directory click toggles expansion

- **WHEN** the user left-clicks a directory row
- **THEN** the directory expands if it was collapsed and collapses if it was
  expanded

#### Scenario: Keyboard stays with active session

- **WHEN** the user clicks any tree row and then types
- **THEN** the keystrokes are delivered to the active center tab's panel
  (the active session when the console tab is active), not to the tree panel
