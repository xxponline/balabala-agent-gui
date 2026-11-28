# Proposal

## Why

The service layer runs on a single-worker tokio runtime, and solution-service
runs every system-git subprocess synchronously inside `handle()` (tracked-clean
`git status` gate, `git worktree list` on every snapshot, `worktree add`/
`remove`, and `git pull`/`push` inside `spawn_task`). On a large repository a
single `git status` occupies the only worker thread for seconds to tens of
seconds, freezing **every** service in the layer — agents, asset enumeration,
timers, and event delivery — not just the git action that started it. The
project already established this rule for asset enumeration ("Enumeration does
not delay other services", enforced by an offload test); solution-service's git
calls violate it and no equivalent requirement exists for git.

## What Changes

- All system-git subprocesses leave the handler thread: the synchronous
  `git` module entry points become internal-only, and the module exposes
  async entry points that run the subprocess via `spawn_blocking`.
- The tracked-clean gate for pull/push no longer runs inside `handle()`; the
  invoke is acknowledged immediately and the gate runs in the background.
  A gate refusal surfaces as the action's failure notice after execution,
  instead of a synchronous error reply.
- Worktree create/delete and the snapshot's `git worktree list` (per project,
  on every load/get_tree/action reply) move to the same async entry points —
  this change fixes all five blocking call sites, not only the status gate.
- Feedback is genuinely out-of-band: the user hears back when the work
  finishes (success or refusal notice), not from a synchronous reply. The
  immediate `accepted` reply remains a transport acknowledgement only
  (it settles the correlation within the 5s request timeout).
- A regression test pins the rule the way asset-service does: a latched git
  call must execute on a thread different from the driver's worker thread.
- The solution-service spec gains the missing requirement: git work does not
  delay other services.

## Capabilities

### New Capabilities

(none — this hardens an existing capability rather than introducing a new one)

### Modified Capabilities

- `solution-service`: add a requirement that git subprocess work never runs
  on the layer's handler thread (mirroring asset-service's "Enumeration does
  not delay other services"), and extend the out-of-band completion requirement
  so the tracked-clean gate and worktree create/delete also complete
  out of band with refusals reported as notices.

## Impact

- **Code**: `crates/solution-service/src/git/` (module surface: sync entry
  points become private, new async wrappers), `crates/solution-service/src/
  service.rs` (5 call sites: `on_git_pull_push` gate, `snapshot`'s
  `worktree_layer`, `on_create_worktree`, `on_delete_worktree`,
  `spawn_git_op`), new offload test alongside them.
- **Contracts**: no message kinds or payload types change; reply timing for
  create/delete/gate shifts from synchronous to out-of-band (notices already
  exist for this path: `KIND_GIT_OP_FINISHED` and `TreeChanged`).
- **Specs**: `openspec/specs/solution-service/spec.md` gains one requirement
  and revises one existing requirement (delta in this change).
- **Runtime behavior**: a slow repo can no longer stall agent sessions, asset
  enumeration, timers, or other services; solution-service's own driver also
  stops back-pressuring `load`/`get_tree` behind a git call.
