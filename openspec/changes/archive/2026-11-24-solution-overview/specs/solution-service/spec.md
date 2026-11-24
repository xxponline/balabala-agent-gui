# Spec Delta

## MODIFIED Requirements

### Requirement: Solution and project state lives in the service

The solution service SHALL own the workspace's solution and project state on
the service side: loading `balabala.solutions.json` and resolving its items
into projects. View layers SHALL read this state through the service
contract and SHALL NOT keep a parallel copy of solutions or projects. The
published snapshot SHALL be hierarchical — each solution with the projects it
resolved — and SHALL be the single expanded source the view renders; the flat
project set and the asset-registration handoff SHALL be derived from it
rather than published alongside it. The service's knowledge SHALL stop at
projects: it hands off project descriptors and SHALL NOT discover modules or
enumerate assets. Each node SHALL carry the actions the service declares for
it. Binding a workspace through a load SHALL also record where its
configuration lives, so later writes need not be told again. When solutions
or projects change the service SHALL publish a change notification so views
refresh without restarting the application.

#### Scenario: Project rows come from the service

- **WHEN** a workspace opens
- **THEN** the project rows shown in views are the projects the service
  resolved, with no view-side re-resolution of the solutions file

#### Scenario: Handoff stops at projects

- **WHEN** the service hands its project tree to the asset side
- **THEN** the tree carries project descriptors only — no module or file
  rows — and the asset side derives everything below from them

#### Scenario: Snapshot nests projects under their solution

- **WHEN** a workspace with two solutions opens
- **THEN** the published snapshot lists each solution with its own resolved
  projects beneath it, and the view derives its flat project list from that
  structure

#### Scenario: Nodes declare their actions

- **WHEN** the view reads a snapshot in which the service declared
  configuration actions on solution nodes
- **THEN** those solution nodes carry the action descriptors, with labels,
  alongside their identity

#### Scenario: Writes reuse the bound workspace

- **WHEN** a workspace has been loaded and a later create or configure
  operation succeeds
- **THEN** the operation applies to that workspace's configuration file
  without the message re-supplying its directory

### Requirement: Create and configure operations are idempotent

Operations that create or configure a solution SHALL be idempotent: re-sending
the same create request for an existing solution SHALL NOT duplicate it, and
re-applying the same configuration SHALL leave state unchanged and SHALL NOT
re-trigger handoffs. The service SHALL expose these idempotent checks as
shared utilities that view layers use for the same guarantees before sending.
All user-initiated solution writes SHALL travel through a single invoke
message carrying the action id, the target node, and any arguments; a
configure operation's target SHALL be resolved from that node rather than
supplied separately, and an unresolvable node or unknown action id SHALL be
rejected without changing state.

#### Scenario: Re-adding an existing solution changes nothing

- **WHEN** a create request names a solution path that already exists in
  the workspace
- **THEN** the solution set is unchanged and no duplicate project or
  handoff results

#### Scenario: Re-applying the same configuration is a no-op

- **WHEN** a configure request repeats the solution's current flag values
- **THEN** state, the persisted file, and the handoff tree are unchanged

#### Scenario: Target comes from the invoke node

- **WHEN** the view invokes a configuration action naming a solution node
- **THEN** the configuration applies to that node's solution, with no
  separate target field in the message

#### Scenario: Unknown action id changes nothing

- **WHEN** an invoke message carries an action id the service did not
  declare
- **THEN** no state or file changes and the failure is reported
