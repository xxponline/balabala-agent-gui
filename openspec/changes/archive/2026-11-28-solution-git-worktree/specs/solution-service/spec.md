# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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

Because network git operations exceed the request timeout, the service SHALL
acknowledge a pull or push invoke immediately and run the operation off the
driver's sequential dispatch path. On completion the service SHALL publish
an outcome event carrying the action id and, on failure, the failure reason;
the event SHALL NOT depend on the original request correlation, which is
already settled. The outcome SHALL surface as a notice naming the action.

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
