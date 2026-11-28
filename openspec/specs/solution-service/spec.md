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
resolved, and each git-managed project with the worktrees registered for it —
and SHALL be the single expanded source the view renders; the flat
project set and the asset-registration handoff SHALL be derived from it
rather than published alongside it. The asset handoff's knowledge SHALL stop
at projects: it hands off project descriptors and SHALL NOT discover modules
or enumerate assets. Each node SHALL carry the actions the service declares
for it. Binding a workspace through a load SHALL also record where its
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

#### Scenario: Snapshot nests worktrees under their project

- **WHEN** a git-managed project has two registered worktrees
- **THEN** the published snapshot lists those worktree nodes beneath that
  project, and a project without worktrees carries an empty worktree layer

#### Scenario: Nodes declare their actions

- **WHEN** the view reads a snapshot in which the service declared
  configuration actions on solution nodes
- **THEN** those solution nodes carry the action descriptors, with labels,
  alongside their identity

#### Scenario: Project and worktree nodes declare their actions

- **WHEN** the view reads a snapshot for a git-managed project that has a
  worktree
- **THEN** the project node carries its git action descriptors and the
  worktree node carries its delete descriptor, each with its label

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

### Requirement: Pull and push require a git-managed tracked-clean project

Pull and push SHALL be offered on project nodes and executed only when the
project resolves as git-managed and its tracked state is clean — no staged
or unstaged changes to tracked files; untracked files SHALL NOT block
execution. A refused execution SHALL surface as that action's failure
notice and SHALL leave the repository untouched.

#### Scenario: Tracked-clean git-managed project pulls

- **WHEN** the user invokes pull on a git-managed project with no tracked
  changes
- **THEN** the pull runs against the project's upstream

#### Scenario: Uncommitted tracked changes block pull

- **WHEN** the user invokes pull on a project with unstaged edits to a
  tracked file
- **THEN** execution is refused with a failure notice naming the action,
  and the working tree is unchanged

#### Scenario: Untracked files do not block push

- **WHEN** the user invokes push on a project whose only outstanding files
  are untracked
- **THEN** the push runs, because untracked files are not uncommitted
  changes to tracked state

#### Scenario: Non-git project cannot pull or push

- **WHEN** the user invokes pull or push on a project whose root is not a
  git directory or whose solution disabled git management
- **THEN** execution is refused with a failure notice naming the action

### Requirement: Git operations complete out of band and report by action id

Because git subprocesses exceed the request timeout, the service SHALL
acknowledge a pull, push, worktree create, or worktree delete invoke
immediately and run the tracked-clean gate and the git work off the
driver's sequential dispatch path. On completion the service SHALL publish
an outcome event carrying the action id and, on failure, the refusal or
failure reason; the event SHALL NOT depend on the original request
correlation, which is already settled. The outcome SHALL surface as a
notice naming the action, including gate refusals discovered during the
background run.

#### Scenario: Slow pull does not time out the invoke

- **WHEN** the user invokes pull and the network operation runs longer than
  the request timeout
- **THEN** the invoke is acknowledged immediately, the service keeps
  processing other messages while it runs, and no late-reply failure is
  recorded

#### Scenario: Failed push surfaces its reason

- **WHEN** a background push is rejected by the remote
- **THEN** a notice appears naming the push action with the rejection
  reason, and no repository state changed

#### Scenario: Tracked changes refuse out of band

- **WHEN** the user invokes pull on a project with uncommitted tracked
  changes and the background tracked-clean check finds them
- **THEN** the invoke is acknowledged immediately, a failure notice naming
  the pull action arrives once the check completes, and the repository is
  untouched

#### Scenario: Worktree creation completes out of band

- **WHEN** the user creates a worktree for a git-managed project and the
  git worktree add runs long
- **THEN** the invoke is acknowledged immediately, the worktree appears in
  the snapshot with a change notification once the add completes, and other
  messages are processed while it runs

### Requirement: Git work does not delay other services

System git subprocesses SHALL NOT run on the layer's handler thread: the
work SHALL be handed off so that messages for other services continue to be
processed while a git command runs. A git command against a repository with
hundreds of thousands of files in flight SHALL NOT delay a message sent to
another service by more than normal handler latency.

#### Scenario: Another service stays responsive during a large-repo status

- **WHEN** the tracked-clean check of a large repository is in flight
- **THEN** messages sent to other services are processed with normal
  handler latency, agent output keeps flowing, and the layer's timers keep
  firing while the check runs

#### Scenario: Snapshot assembly does not hold the handler thread

- **WHEN** a snapshot is produced for a workspace whose projects need git
  worktree listing
- **THEN** messages to other services are processed while the listing
  runs, and the listing result still reaches the snapshot that requested it

### Requirement: Worktrees are created under the workspace directory

Creating a worktree for a git-managed project SHALL run the system git
worktree add for that project and place the working copy under the
workspace's `.balabala.worktrees` directory with a name derived from the
requested ref and sanitized for path safety. The worktree SHALL appear in
the snapshot beneath its project, discovered from git's own worktree
metadata rather than a parallel manifest. Creation SHALL be refused for a
project that is not git-managed.

#### Scenario: Created worktree appears under its project

- **WHEN** the user creates a worktree for a git-managed project
- **THEN** the working copy exists under the workspace's
  `.balabala.worktrees` directory and the snapshot lists the worktree
  beneath that project

#### Scenario: Non-git project cannot create a worktree

- **WHEN** the user invokes create worktree on a project whose root is not
  a git directory
- **THEN** execution is refused with a failure notice naming the action

#### Scenario: Worktree list comes from git metadata

- **WHEN** a worktree was registered for the project outside this
  application and the snapshot is read
- **THEN** that worktree is listed beneath its project, because the
  registered set is discovered from git's own worktree metadata

### Requirement: Worktree deletion requires no running agent and git's own cleanliness check

Deleting a worktree SHALL be refused while an agent session for the owning
project is starting or running. Otherwise it SHALL run the system git
worktree remove without force, so git's own cleanliness check decides: a
worktree with modified or untracked files stays in place, with the refusal
surfaced as the action's failure notice. A successful deletion removes the
working copy and its git worktree metadata.

#### Scenario: Running agent blocks deletion

- **WHEN** the user deletes a worktree of a project that has a session in
  the starting or running state
- **THEN** deletion is refused with a notice and the worktree is untouched

#### Scenario: Dirty worktree is refused by git's check

- **WHEN** the user deletes a worktree that contains an uncommitted file
- **THEN** git's cleanliness check refuses the removal, the directory and
  its metadata remain, and the failure is surfaced as the action's notice

#### Scenario: Clean idle worktree is removed

- **WHEN** the user deletes a clean worktree of a project with no starting
  or running session
- **THEN** the working copy and its git worktree metadata are removed and
  the snapshot no longer lists it
