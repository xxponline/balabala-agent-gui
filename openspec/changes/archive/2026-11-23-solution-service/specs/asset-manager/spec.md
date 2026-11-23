# Spec Delta

## MODIFIED Requirements

### Requirement: Project registration lifecycle

Projects SHALL be registered into the asset manager when a workspace opens
(all of that workspace's projects, replacing any previous registration),
incrementally when a solution is added, and cleared when the workspace
switches. The registration source SHALL be the solution service's
already-enumerated project tree: the asset manager receives project
descriptors and SHALL NOT resolve solutions itself. Only projects whose
solution has asset registration enabled SHALL appear in the tree.
Registration changes SHALL be reflected in the manager's view without
restarting the application.

#### Scenario: Workspace open registers its projects

- **WHEN** a workspace whose solutions produce two projects opens
- **THEN** both projects are registered and their assets become the
  manager's scope

#### Scenario: Add solution registers incrementally

- **WHEN** the user adds a solution picking `/a/foo` in an open workspace
- **THEN** project `foo` is registered without a restart and its enumeration
  begins

#### Scenario: Workspace switch replaces the registration

- **WHEN** the user switches to a workspace whose solutions produce one
  project
- **THEN** previously registered projects are dropped and no stale rows
  remain in the view

#### Scenario: Registration-disabled solution stays out of the tree

- **WHEN** a solution with asset registration disabled produces one project
  and the workspace opens
- **THEN** that project is never registered and its assets never enter the
  view

### Requirement: Filter provisioning from the project root

Each module's filter set SHALL be derived from its project. At module
creation the project SHALL use its resolved git-managed flag — enabled only
when its root is a git directory and its solution enables git management —
instead of probing the filesystem; if git-managed it SHALL read the project
root's `.gitignore` and provide those rules, with the default dot ignore, to
every module it creates. A project whose flag is clear SHALL not be probed:
a `.gitignore` beneath it — and a `.git` directory beneath it when the
solution disables git management — SHALL have no effect. Modules below the
project root SHALL receive the project's filter set.

#### Scenario: Git-managed project applies its root gitignore

- **WHEN** a project root contains `.git/` and a `.gitignore` that hides
  `*.tmp`, and the project contains `cache.tmp`
- **THEN** `cache.tmp` does not appear in the view

#### Scenario: Non-git project does not read its gitignore

- **WHEN** a project root lacks `.git` but contains a `.gitignore` hiding
  `*.tmp`, and the project contains `cache.tmp`
- **THEN** `cache.tmp` still appears in the view

#### Scenario: Git directory with git management disabled does not read gitignore

- **WHEN** a project root contains `.git/` and a `.gitignore` hiding
  `*.tmp`, its solution disables git management, and the project contains
  `cache.tmp`
- **THEN** `cache.tmp` appears in the view
