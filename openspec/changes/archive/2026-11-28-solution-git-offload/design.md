# Design

## Context

See proposal.md - Why. The facts that shape this design:

- `LayerRuntime` is `worker_threads(1)` by explicit decision (D9): the
  layer chose the smallest runtime *because* the rule is "blocking work
  goes to `spawn_blocking`" (runtime.rs:9). Every driver, the router,
  ask-timeout timers, and `Messaging::spawn_task` futures share that one
  thread.
- solution-service runs 5 git call sites synchronously in that context:
  the tracked-clean gate (`service.rs:238`), `git worktree list` inside
  `snapshot()` on every load/get_tree/action reply (per git project),
  `worktree add`, `worktree remove` (+ another `list`), and
  `spawn_git_op`'s `git pull`/`push` — the last one is spawned but still
  executes `Command::output()` on the same single worker, so the design
  comment "不占 driver 顺序上下文" only holds logically, not thread-wise.
- asset-service already established the house pattern: synchronous fs off
  the worker via `spawn_task(async { spawn_blocking(...) })`, a latched
  offload test asserting thread identity (offload_tests.rs), and a spec
  requirement "Enumeration does not delay other services".
- The archived change `solution-git-worktree` fixed the *shape* of pull/push
  (D1: instant ack + no-corr completion event, because `DEFAULT_CALL_TIMEOUT
  = 5s` turns any late corr reply into a dead letter) but left all git
  subprocesses on the handler path (D2 gate) and on the worker (spawned
  git ops).

## Goals / Non-Goals

**Goals:**

- No system-git subprocess ever executes on the layer's worker thread:
  compile-time surface (sync entry points private) + regression test.
- All five call sites converted; the tracked-clean gate and worktree
  create/delete give feedback only after execution (out-of-band notices).
- A spec requirement pinning "git work does not delay other services",
  mirroring asset-service's wording.

**Non-Goals:**

- Caching or reducing how often `git worktree list` runs (frequency is a
  perf question; this change only moves it off the thread).
- Raising worker thread count (violates D9's premise; band-aid).
- Auditing other services for blocking work in `handle()` (agent-service
  spawns `pi` processes; separate investigation).
- Changing router timeouts or the `accepted`/notice contract kinds.

## Decisions

### D1 git module surface: sync internals, async-only public API

`git/mod.rs` and `git/worktree.rs` keep their synchronous implementations
(`run`, `run_ok`, `is_tracked_clean`, `worktree::{list, add, remove}`) but
make them module-private; the public surface becomes async entry points
that wrap each subprocess in `tokio::task::spawn_blocking` and await the
result. Callers outside the module physically cannot run git on the
worker.

- *Alternative (a):* keep sync API, rely on discipline + test. Rejected —
  the whole problem is that the current comments already state the intent
  ("不占 driver 顺序上下文") and the code still blocks; a rule without a
  type-system wall regresses the same way.
- *Alternative (b):* `async fn` wrappers only at the 5 call sites, module
  untouched. Rejected — same wall-less problem for the next contributor.
- Note: `spawn_blocking` runs on tokio's blocking pool (dedicated
  threads), so a 60s `git status` occupies neither the worker nor the
  driver. Drop of `LayerRuntime` already refuses to wait on in-flight
  `spawn_blocking` (runtime.rs), so shutdown behavior is unchanged.

### D2 The gate moves into the background; feedback is the notice

`on_git_pull_push` keeps only cheap, deterministic validation in
`handle()` (node format → reject, no such solution → `NoSuch`, `.git`
existence → non-git refusal: an fs metadata probe, not a git run). The
tracked-clean `git status` moves into the background task:

```
view --ask(corr)--> handle: validate (in-memory/fs probe)
        |-- reply Ok(current snapshot)   <-- transport ack, settles corr < 5s
        +-- spawn_blocking:
              git status -uno
                 |-- dirty/err: GIT_OP_FINISHED{ok:false, reason} --> notice  <-- user feedback
                 \-- clean: git pull/push --> GIT_OP_FINISHED{ok, reason}    <-- user feedback
```

This revises D2 of `solution-git-worktree` ("不过 → 立即 `OpError` 回执,
不 spawn"): on a large repo the status itself can exceed 5s, so a
synchronous refusal reply is physically impossible — the refusal must
travel the completion-event path that D1 already built. The spec wording
already supports this: the tracked-clean requirement says refusals
"surface as that action's failure notice", never "as an immediate error
reply".

- *Alternative:* keep the gate synchronous and only offload execution.
  Rejected — that is exactly the reported freeze (status is the slowest
  call site of the five).

### D3 Immediate ack carries the pre-operation snapshot; state lands via events

For worktree create/delete the immediate reply keeps returning
`OpResult::Ok(snapshot)` — the snapshot as it is *before* the operation
(state has not changed yet).

`snapshot()` itself runs `git worktree list` per git project, so it
becomes async too: the listing executes off-thread and `handle` awaits
its result before acking. The ack is therefore delayed by worktree-list
latency (O(worktrees) metadata reads, not O(files)) — bounded well under
the 5s timeout. Honest scope: solution-service's *own* queued messages
wait for that await (the driver is sequential by design); what this
change guarantees is that no other service, timer, or the worker thread
itself waits. Completion publishes the real change:
create/delete → existing `TreeChanged` (already emitted today on success;
built in the background unit from post-operation state cloned before
spawn), pull/push → existing `KIND_GIT_OP_FINISHED`. No new message
kinds, no new payload fields. Ordering guarantee from D1 of the archived change is
preserved: the ack is sent synchronously before the background task is
spawned, so the completion event can never beat its ack.

### D4 One background task per invoke: gate and execution back-to-back

The status check and the git operation run in a single `spawn_blocking`
unit (or a single background task) with no yield between them, keeping the
gate-to-execution window as small as today's handle-to-spawn window. The
remaining race (user edits between ack and gate) resolves the same way it
does now: the gate itself re-decides, and git's own safety applies
downstream. Worktree create/delete likewise run their git call and the
subsequent snapshot assembly in one background unit; the snapshot is built
from the post-operation state and delivered through `TreeChanged`.

### D5 Regression test: latched git call, thread-identity assertion

Modeled on `asset-service/offload_tests.rs`: inject a latch into the git
path (the module already supports program injection in `run_program`),
start a pull/push or a snapshot containing a worktree listing, and assert
that the thread executing the subprocess is **not** the driver's worker
thread — with a watchdog that auto-releases so a broken implementation
fails instead of hanging. This is the test that fails if someone "simplifies"
the async entry points back to sync calls.

## Risks / Trade-offs

- [Ack-then-refusal changes what the user sees: a pull click can be
  acknowledged and then refused a few seconds later] → the refusal notice
  names the action (existing `node-actions` notice chain), which is the
  same surface a synchronous refusal produced; the spec's tracked-clean
  scenarios already describe feedback as a notice.
- [Two messages where there was one (ack + notice) — views must tolerate
  the ack carrying a pre-operation snapshot] → views already render
  `TreeChanged`/notices as the source of truth for changes; the ack
  snapshot is the same payload shape they handle today.
- [Bigger ack-to-gate race window: user can edit between ack and the
  background status] → unchanged in kind from the current ack-to-spawn
  window; worst case the gate refuses (repo untouched) — the safe
  direction.
- [Blocking pool saturation if many git invokes queue at once] → one
  background unit per user invoke; tokio's default blocking pool (512)
  is far above any realistic concurrency here.
- [A stopped/unloaded instance may still have a git subprocess in
  flight] → existing stance from `spawn_git_op` applies: the operation
  always executes; the completion event simply finds no subscriber.

## Migration Plan

1. Module surface flip (D1) — compiler enumerates every call site;
2. convert the 5 sites in dependency order: gate + pull/push (D2),
   snapshot listing, create, delete (D3/D4);
3. add the offload test (D5);
4. spec delta lands with this change; archive applies it.

Rollback: revert the commits; no data format, contract kind, or persisted
state changes are involved.

## Open Questions

None — feedback timing (async, report after execution) and scope (all five
call sites including snapshot listing) were decided in exploration.
