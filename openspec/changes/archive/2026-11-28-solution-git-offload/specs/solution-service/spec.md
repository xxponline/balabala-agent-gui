# Spec Delta

## ADDED Requirements

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

## MODIFIED Requirements

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
