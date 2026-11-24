# Spec Delta

## MODIFIED Requirements

### Requirement: Services declare the actions a node offers

A service that publishes tree nodes SHALL attach to each node the set of
actions available on it, each descriptor carrying a stable action id, a
display label, an explicit declaration of which layer executes it, and —
for actions that hold a toggleable value — the current value so the view
can render its state. The execution declaration SHALL be present on every
descriptor with no implicit default, so an omitted declaration cannot
silently change who runs the action. The descriptor SHALL be part of the
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

#### Scenario: Every descriptor declares its executor

- **WHEN** a service publishes a node carrying any action descriptor
- **THEN** that descriptor states whether the service or the view executes
  it, with no default assumed when the field is absent

### Requirement: Activating an action messages the declaring service

Activating a menu entry declared as executed by the service SHALL send a
single invoke message carrying the node's identity, the action id, and any
arguments, addressed to the service that declared the action. Arguments
SHALL be absent for actions that need none and SHALL carry the collected
value for actions whose parameters a form gathered. The view SHALL NOT
encode what such an action does — it forwards the id and the node, nothing
more. An entry declared as executed by the view SHALL NOT send this
message; see the view-executed action requirement.

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

#### Scenario: Service-declared entry does send an invoke

- **WHEN** the user activates an entry whose descriptor declares the
  service as its executor
- **THEN** exactly one invoke message reaches that service carrying the
  node identity and the action id

## ADDED Requirements

### Requirement: View-executed actions produce no message

A menu entry whose descriptor declares the view as its executor SHALL be
carried out entirely by the view and SHALL send the declaring service no
invoke message. The descriptor only tells the view the capability is
offered on that node and supplies its label; the view supplies the
capability itself — a view-layer one such as writing the clipboard or
asking the OS to reveal a path. The view SHALL still route any failure it
produces by the action id, never the display label.

#### Scenario: View-executed entry sends nothing

- **WHEN** the user activates an entry whose descriptor declares the view
  as its executor
- **THEN** no invoke message is sent to any service and the capability
  takes effect

#### Scenario: Menu still lists view-executed entries

- **WHEN** a service publishes a node whose descriptors are all declared
  view-executed
- **THEN** the context menu lists them with their labels, built from the
  snapshot without a round trip

#### Scenario: Failure of a view-executed action is routed by id

- **WHEN** a view-executed action fails and the view reports it
- **THEN** the report names the action by its id, so renaming the label
  cannot change what is reported
