# Design

## Context

See proposal.md - Why. The facts that shape this design:

- `solution-git-worktree` established the two new menu surfaces (project-row
  pull/push/create, worktree-row delete) and the out-of-band
  `KIND_GIT_OP_FINISHED` notice funnel; `solution-git-offload` then made the
  async-only git module exits a compile-time wall and moved every git
  subprocess off the layer worker. The spec now hard-requires this ("Git work
  does not delay other services"), so new git work has exactly one legal
  shape.
- The current handler shape (post-offload `on_delete_worktree`) is: cheap
  node-format check in `handle()` -> immediate `accepted` reply with the
  pre-op snapshot -> background task owns owner-resolution, gates, and the
  git work -> outcome published as `KIND_GIT_OP_FINISHED` (gate refusals
  included).
- `PopupMenuItem::submenu(label, Entity<PopupMenu>)` exists in
  gpui-component; `ProjectRow` already carries `worktrees`, and worktree rows
  address their node as a bare path that `on_delete_worktree` resolves to an
  owner project by scanning `git worktree list`.
- `ActionDesc` lives in the shared `contracts` crate (asset rows use it too)
  and has 7 construction sites. `KIND_GIT_OP_FINISHED` notices currently
  print the raw action id.

## Goals / Non-Goals

**Goals:**

- The two sync directions behave exactly as the specs demand, through the
  existing offload/out-of-band machinery with no new message kinds.
- The submenu is declared by the service (descriptors stay the single source
  of what menus contain) and rendered generically by the view.
- Repository status learned during a git run reaches the row as a badge with
  the smallest machinery that satisfies the requirement.

**Non-Goals:**

- Background sweep to fill initial badges (b+): rows badge `unknown` until
  the first outcome arrives.
- Snapshot-level status fields (the solution-git-worktree OQ1 debt): putting
  status into `TreeSnapshot` would force every `accepted` reply to await N
  `git status` runs and break the sub-second ack - that change needs its own
  two-phase design and is deliberately separate.
- Conflict-resolution UI or terminal integration: conflicts are resolved in
  the user's own terminal; the app only reports them.
- Rebase/cherry-pick alternatives, worktree creation forms (worktree change
  OQ2), and caching how often `git worktree list` runs.

## Decisions

### D1 Direction model: merge by HEAD sha, ff-only on the project side

- **worktree.sync_from_project** runs `git merge <project-HEAD-sha>` inside
  the worktree directory. Gate: the worktree's tracked state only - the
  project's working tree is never written by this direction (the merge reads
  its commit, not its files). A real merge, so conflicts are possible and are
  the reason the conflict handling exists.
- **project.sync_from_worktree** runs `git merge --ff-only <worktree-HEAD-sha>`
  inside the project directory, behind four gates in deterministic order:
  project tracked-clean -> worktree tracked-clean -> worktree HEAD contains
  project HEAD -> ahead by >= 1. `--ff-only` is belt-and-suspenders: the
  contains+ahead gates already guarantee a fast-forward, so a HEAD that moves
  between gate and merge fails cleanly instead of silently creating a merge
  commit.
- **Shas, not branch names**: works verbatim for detached-HEAD on either
  side, and skips branch resolution entirely. A detached worktree is not
  specially gated - merging into a detached HEAD is legal git, the notice
  reports the result, and the follow-up direction reads HEAD sha anyway.
- Gate-order maps to the two distinct refusals: fails-contains -> "run the
  worktree-side sync first" (covers both behind and diverged); contains but
  ahead=0 -> "nothing to sync" (equal heads).
- No session pre-gate, consistent with pull: pull already mutates the working
  tree under any running session; delete gates sessions because it removes
  the directory, not because it edits files.
- *Alternative:* merge by branch name - rejected (detached edge, extra
  resolution). *Alternative:* let the project side merge for real - rejected
  (the four gates exist precisely to make this direction conflict-free).

### D2 Handler shape: exactly the post-offload delete/pull-push shape

`handle()` does node-format validation only; everything else - owner
resolution (`worktree list_async` scan), the gates, and the merge - runs in
one background task whose results flow through `KIND_GIT_OP_FINISHED`:

```
view --ask(corr)--> handle: parse bare-path node (reject on contract bug)
        |-- reply Ok(pre-op snapshot)          <-- settles corr < 5s
        +-- background task (blocking pool via async exits):
              owner scan -> is_git -> gates -> git merge
              -> GitOpFinished { action_id, target, ok, detail, statuses }
              -> notice (+ badge updates on the view side)
```

The two handlers differ only in which gates and merge args they run; they
share the outcome-publish tail with `spawn_git_op`. No new envelope kinds,
no corr on the outcome (already settled), no changes to reply timing.

### D3 ActionDesc children: service declares, view renders generically

- Contract (`crates/contracts/src/shared.rs`): `ActionDesc` gains
  `children: Arc<[ActionChild]>`; new `ActionChild { label: String, node: String }`
  where `node` is the target node id (bare worktree path, same addressing as
  delete). All 7 construction sites add an explicit empty `children`;
  asset-side renderers ignore the field unchanged.
- Service: `project_actions(&worktrees)` appends the "从 worktree 同步"
  descriptor **iff the worktree layer is non-empty**, one child per
  worktree (label = the row's display name, node = its path). The layer is
  snapshot-derived exactly like the worktree rows beneath it, so menu and
  rows appear and vanish together.
- View rule: `children.is_empty()` -> flat item (today's path);
  otherwise -> `PopupMenuItem::submenu`, child click invokes
  `(node = child.node, action_id = parent.id, checked = None)`.
  `action_args` needs no new arm - both actions identify their target from
  the node alone.
- *Alternative (a):* view builds the submenu from `ProjectRow.worktrees`
  behind an id special-case - rejected: the view would embed product
  knowledge ("this id is a submenu of worktrees"), breaking the D3
  dumb-renderer rule, and the children would not be declared.
- *Alternative (b):* always declare the parent and render it disabled (or
  flat) when children are empty - rejected: a flat fallback makes the same
  action arrive under two node forms (`project://` vs bare path) and shows
  menu items that can only ever fail on projects without worktrees. Declaring
  iff-non-empty has no worse staleness than the rows it mirrors (invoke-time
  drift is still caught by the server-side owner/gate checks).

### D4 Badges: structured status on the existing outcome event

- Contract (solution-service): `GitOpFinished` gains
  `statuses: Vec<RepoStatus>` with `RepoStatus { path, state }` and
  `state: Clean | Dirty | Conflict`. In-process `Payload`, no serde surface.
- The background task fills it from what it already learned - **no extra
  `git status` runs**: a tracked-clean gate answers dirty; a successful merge
  leaves the merged repository clean; a conflict is detected from the merge
  exit code plus `CONFLICT` in its output (never by re-running status). Each
  direction reports every repository it examined (the project side examines
  both).
- View: a `path -> state` map held at panel/app level (survives snapshot
  refreshes; project rows keyed by root, worktree rows by bare path).
  Absent entry = `unknown` (gray). Logical refusals (not-contains,
  equal-heads) examined no dirty state and leave badges untouched unless a
  status gate actually ran.
- *Alternative:* status in `TreeSnapshot` - rejected for this change: the
  accepted reply is built from the pre-op snapshot, so awaiting per-project
  status would break the sub-second ack on large repositories; it also needs
  a staleness policy for external edits. That is the two-phase OQ1 design,
  kept separate.
- Stance: badges reflect the last observed outcome. External `git` activity
  does not flip them until the next run - accepted for (b), and the reason
  the sweep (b+) stays a non-goal.

### D5 Finish notices name actions by label

- Contract gains `action_label(id) -> Option<&'static str>` - a static table
  over the seven declared labels (children are dynamic but outcome events
  only ever carry parent ids).
- `service_layer` formats `KIND_GIT_OP_FINISHED` as
  `{label or raw id} 完成/失败: ...`, falling back to the raw id for
  unknown future ids. Text-only change; it fixes the existing four actions'
  raw-id notices in the same line.

### D6 Merge helpers are thin async wrappers

New exits in the git module, each a one-liner over the existing generic
`run_async` (the sync core stays module-private - the offload wall is not
punctured): `head_sha(cwd)`, `is_ancestor(base, tip)`,
`ahead_count(base, tip)` (`rev-list --count`), and
`merge(cwd, sha, ff_only)` returning exit code + output so conflict detection
can read `CONFLICT` without a second call.

## Risks / Trade-offs

- [The seven `ActionDesc` construction sites fail to compile after the field
  addition] -> mechanical one-line fix each; the doc example in `shared.rs`
  updates in the same commit. Renderers that ignore `children` need no
  change, so asset behavior cannot regress.
- [Tests asserting raw-id notice text (`git.pull 完成`) break under D5] ->
  expected, text-only updates; the new label assertions are stricter than
  what they replace.
- [HEAD moves between gate and merge (both directions)] -> D2 fails cleanly
  under `--ff-only` and reports it; D1 merges a sha it already resolved, so
  the worst case is syncing one commit behind - harmless and self-correcting
  on the next run.
- [A conflict leaves the repository mid-merge and every later action refuses
  with a tracked-changes notice] -> intended: the badge shows `conflict`
  (distinct from dirty), and the tracked-clean gate keeps refusing until the
  user resolves and commits in their terminal - no app-side recovery path is
  required.
- [Badge staleness after external git activity] -> accepted stance of (b);
  documented in the spec requirement ("never observed -> unknown",
  "not examined -> previous badge kept").
- [Owner scan cost (one `git worktree list` per git project per run)] ->
  already the post-offload delete pattern, runs in the blocking pool, and is
  bounded by the number of registered solutions.

## Migration Plan

None - additive behavior behind existing message kinds; in-process payloads
travel with the binary. Rollback = revert the commit.
