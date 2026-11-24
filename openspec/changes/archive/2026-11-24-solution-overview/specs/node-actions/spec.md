# Spec Delta

## Purpose

Owns how a service declares which operations a tree node offers and how the
view carries them out: the service attaches action descriptors to the nodes it
publishes, the view renders them as a context menu without knowing what any
action does, and activating one sends a message back to the service that
declared it.

## ADDED Requirements

### Requirement: Services declare the actions a node offers

A service that publishes tree nodes SHALL attach to each node the set of
actions available on it, each descriptor carrying a stable action id, a
display label, and — for actions that hold a toggleable value — the current
value so the view can render its state. The descriptor SHALL be part of the
snapshot the view already reads, so building a node's menu requires no
round trip to the service. The label is owned by the service that declares
the action.

#### Scenario: Menu is built from the snapshot

- **WHEN** the user right-clicks a node whose service published two actions
  for it
- **THEN** the context menu lists both, with their labels, without any
  message being sent to the service first

#### Scenario: A node with no declared actions has no menu

- **WHEN** the user right-clicks a node whose service declared no actions
- **THEN** no context menu opens

#### Scenario: Toggle action shows its current value

- **WHEN** a service declares a toggle action whose current value is on
- **THEN** the menu entry for it renders in the on state

### Requirement: Activating an action messages the declaring service

Activating a menu entry SHALL send a single invoke message carrying the
node's identity, the action id, and any arguments, addressed to the service
that declared the action. Arguments SHALL be absent for actions that need
none and SHALL carry the collected value for actions whose parameters a form
gathered. The view SHALL NOT encode what an action does — it forwards the id
and the node, nothing more.

#### Scenario: Parameterless action sends id and node only

- **WHEN** the user activates a toggle action that takes no arguments
- **THEN** the invoke message carries the node identity and the action id
  with no arguments

#### Scenario: Form-collected arguments ride the invoke

- **WHEN** the user completes a form that gathers a new name and confirms
- **THEN** the invoke message carries the node identity, the action id, and
  the gathered value as its arguments

#### Scenario: Unknown node or action is rejected

- **WHEN** an invoke message names a node or action id the receiving service
  does not know
- **THEN** no state changes and the failure is reported rather than silently
  ignored

### Requirement: Action failures surface as a named notice

An action whose execution fails SHALL produce an on-screen notice naming the
action that failed and SHALL leave state as it was. Routing an outcome to the
right place SHALL use the action id, never the display label, so that
changing a label cannot change behavior.

#### Scenario: Failed action names itself

- **WHEN** an action's execution fails
- **THEN** a notice appears naming that action, and the node's state is
  unchanged

#### Scenario: Renaming a label does not change routing

- **WHEN** a service changes the display label of an action whose previous
  label some view matched on
- **THEN** outcomes are still routed correctly, because routing uses the
  action id
