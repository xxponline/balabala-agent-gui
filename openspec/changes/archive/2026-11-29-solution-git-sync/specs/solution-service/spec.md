# Spec Delta

## ADDED Requirements

### Requirement: Worktree syncs from its project

The worktree sync action SHALL merge the project's current HEAD into that
worktree, running out of band after an immediate acknowledgement, and SHALL
execute only when the worktree's tracked state is clean — the project's own
working tree does not gate this direction because the merge never modifies
it. On conflict the action SHALL fail with a notice naming the conflicted
files and leave the repository in its unfinished-merge state.

#### Scenario: Clean worktree receives the project's commits

- **WHEN** the user invokes the worktree sync on a worktree whose tracked
  state is clean and whose branch has diverged from the project
- **THEN** the invoke is acknowledged immediately, the merge runs in the
  background, a completion notice naming the action reports the merge
  result, and the worktree's HEAD contains the project's HEAD afterwards

#### Scenario: Dirty worktree refuses the sync

- **WHEN** the user invokes the worktree sync on a worktree with
  uncommitted tracked changes
- **THEN** a failure notice naming the action reports the tracked-changes
  reason, the repository is untouched, and the worktree's row badges dirty

#### Scenario: Merge conflict surfaces files and leaves the merge open

- **WHEN** the merge conflicts because both sides changed the same lines
- **THEN** a failure notice naming the action lists the conflicted files,
  the worktree's row badges conflict rather than dirty, and the repository
  remains in its unfinished-merge state for the user to resolve

#### Scenario: Non-git-managed project refuses the sync

- **WHEN** the user invokes the worktree sync for a project that is not
  git-managed or whose solution has git management turned off
- **THEN** a failure notice naming the action arrives and no repository is
  touched

### Requirement: Project fast-forwards from a worktree

The project's from-worktree sync action SHALL fast-forward the project's
branch from the selected worktree's HEAD and SHALL execute only when all
four gates hold: the project's tracked state is clean, the worktree's
tracked state is clean, the worktree's HEAD contains the project's HEAD,
and the worktree's HEAD is ahead by at least one commit. A failed gate
SHALL surface as the action's failure notice with no repository change.

#### Scenario: Ahead worktree fast-forwards the project

- **WHEN** the user picks a worktree that is tracked-clean, contains the
  project's HEAD, and is ahead of it by commits, with the project also
  tracked-clean
- **THEN** the invoke is acknowledged immediately, the background run
  fast-forwards the project's branch, a completion notice naming the action
  reports it, and both rows badge clean afterwards

#### Scenario: Missing project commits direct back to the worktree sync

- **WHEN** the picked worktree does not contain the project's HEAD because
  the project advanced or the histories diverged
- **THEN** a failure notice naming the action directs the user to run the
  worktree-side sync first, and no repository is touched

#### Scenario: Equal heads refuses as a no-op

- **WHEN** the picked worktree's HEAD equals the project's HEAD
- **THEN** a failure notice naming the action reports there is nothing to
  sync, and no repository is touched

#### Scenario: Dirty side refuses and badges dirty

- **WHEN** either the project or the picked worktree has uncommitted tracked
  changes
- **THEN** a failure notice naming the action reports which side is dirty,
  that side's row badges dirty, and no repository is touched

### Requirement: Repository status surfaces on nodes

Every git outcome event SHALL carry the status observed for each repository
the run examined — clean, dirty, or conflict — and the overview SHALL render
it as a badge on that repository's row. A row whose status has never been
observed SHALL badge as unknown, and conflict SHALL be distinct from dirty:
an unfinished merge, not merely uncommitted changes. Repositories not
examined by the run SHALL keep their previous badge.

#### Scenario: Successful run badges clean

- **WHEN** a git action completes successfully after its gates passed
- **THEN** every repository the run examined badges clean on its row

#### Scenario: Gate refusal badges dirty

- **WHEN** a tracked-clean gate refuses an action
- **THEN** the refusing repository's row badges dirty, distinct from the
  initial unknown state

#### Scenario: Conflict badges conflict

- **WHEN** a merge run ends in conflict
- **THEN** the affected row badges conflict, a state visually distinct
  from both clean and dirty

#### Scenario: Fresh overview starts unknown

- **WHEN** the overview opens before any git outcome has been observed
- **THEN** repository rows badge unknown until an outcome event arrives

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
- **THEN** the project node carries its git action descriptors — including
  the from-worktree sync descriptor with one child per registered worktree —
  and the worktree node carries its delete and project-sync descriptors,
  each with its label

#### Scenario: Writes reuse the bound workspace

- **WHEN** a workspace has been loaded and a later create or configure
  operation succeeds
- **THEN** the operation applies to that workspace's configuration file
  without the message re-supplying its directory

### Requirement: Git operations complete out of band and report by action id

Because git subprocesses exceed the request timeout, the service SHALL
acknowledge a pull, push, worktree create, worktree delete, worktree sync,
or project sync invoke immediately and run the tracked-clean gate and the
git work off the driver's sequential dispatch path. On completion the
service SHALL publish an outcome event carrying the action id, the status
observed for each repository the run examined, and, on failure, the refusal
or failure reason; the event SHALL NOT depend on the original request
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
