# solution-service Specification

## Purpose

Owns the workspace's solution and project domain as a bus service: loading
and resolving solutions into projects, solution configuration (asset
registration and git management), and the project-level handoff the asset
side builds on. View layers share its idempotent checkers instead of keeping
parallel state.

## Requirements

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
### Requirement: Solution load failures surface as structured errors

When loading fails — the solutions file cannot be parsed, an item declares an
unknown `SolutionType`, or a `simple` item's path does not exist on disk —
the service SHALL reply with the failure reason and SHALL NOT return partial
project state. Every load boundary (workspace open chosen at startup, explicit
reload) SHALL present the error dialog and SHALL NOT open the workspace.

#### Scenario: Corrupt file at startup

- **WHEN** the user chooses a workspace at startup and its
  `balabala.solutions.json` contains invalid JSON
- **THEN** the load reply carries the parse failure, an error dialog
  describes it, and the workspace is not opened

#### Scenario: Failed load yields no partial tree

- **WHEN** one item's path is missing on disk and others are valid
- **THEN** the reply is the failure only — no partial project tree is
  returned or cached
### Requirement: Asset registration flag gates project handoff

Each solution SHALL carry a flag controlling whether its projects register
into the asset service. Projects of a solution with the flag disabled SHALL
be absent from the handoff tree while remaining present in the solution's
own project set; toggling the flag SHALL take effect on the next handoff
without restarting, and the flag SHALL persist to the solutions file.

#### Scenario: Disabled solution stays out of the asset tree

- **WHEN** a solution with asset registration disabled resolves to one
  project and the workspace opens
- **THEN** the project is absent from the asset registration handoff

#### Scenario: Enabling mid-session registers on next handoff

- **WHEN** the user enables asset registration for that solution
- **THEN** the project appears in the handoff and its assets enumerate
  without restarting the application
### Requirement: Git-managed resolution combines root and solution

Each enumerated project's git-managed flag SHALL be resolved as enabled only
when its root directory is a git directory AND its solution enables git
management; the resolved flag SHALL travel with the project to the asset
side. The service SHALL NOT read `.gitignore` or any module-scoped data.

#### Scenario: Git root with git management enabled

- **WHEN** a project's root contains `.git/` and its solution enables git
  management
- **THEN** the project's resolved flag is git-managed

#### Scenario: Git root with git management disabled

- **WHEN** a project's root contains `.git/` but its solution disables git
  management
- **THEN** the project's resolved flag is not git-managed

#### Scenario: Non-git root regardless of solution setting

- **WHEN** a project's root is not a git directory
- **THEN** the project's resolved flag is not git-managed whatever the
  solution's git setting

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
