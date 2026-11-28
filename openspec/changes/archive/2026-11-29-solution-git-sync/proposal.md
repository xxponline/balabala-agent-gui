# Proposal

## Why

`solution-git-worktree` shipped pull/push, worktree create/delete, and the
offload change made git work non-blocking — but the reason worktrees exist is
still missing: the two everyday code-sync directions. A feature branch living
in a worktree cannot pull the project's latest commits, and the project cannot
take the worktree's commits back — both currently require the user to open a
terminal and merge by hand. On top of that, no repository status is visible
anywhere in the overview: precondition refusals and merge conflicts only ever
arrive as one-shot notice text, and the row itself never shows that a
repository is dirty or mid-conflict.

## What Changes

- **Worktree row action "同步 Project 代码"** (`worktree.sync_from_project`):
  merges the project's current HEAD into the worktree. Gate = the worktree's
  tracked state is clean (the project's working tree is untouched by this
  direction, so it is not gated). A real merge — conflicts are possible; a
  conflict surfaces as a failure notice listing the conflicted files plus a
  conflict status on the row, with the repository left in its unfinished-merge
  state for the user to resolve.
- **Project row submenu "从 worktree 同步"** (`project.sync_from_worktree`):
  one child item per registered worktree. Fast-forward only: four gates —
  project clean, worktree clean, worktree contains the project's HEAD, and is
  ahead of it by at least one commit. A refusal because the worktree does not
  contain the project's HEAD directs the user back to the worktree-side sync;
  equal heads is refused as a no-op. Structurally conflict-free (`--ff-only`).
- **Declarative submenu children**: `ActionDesc` (shared `contracts` crate)
  gains `children: Arc<[ActionChild]>` where `ActionChild { label, node }`.
  The service declares the sync descriptor with one child per worktree; the
  view renders any descriptor with children as a submenu generically. Flat
  descriptors and existing consumers (asset rows) are unaffected.
- **Repository status badges**: the git outcome event carries a structured
  repository status (clean / dirty / conflict) learned during the background
  run — gate refusals already know it, merge conflicts are determinable from
  the merge result without an extra status call. Rows render it as a badge;
  the initial state is unknown until an outcome is observed (a background
  fill sweep is explicitly out of scope).
- **Notice text names actions by label**: `KIND_GIT_OP_FINISHED` notices map
  the action id to its Chinese label instead of printing the raw id — this
  also fixes the existing four actions' notices (`git.pull 完成: …`).
- All new git subprocess work uses the async-only module exits, as the
  "Git work does not delay other services" requirement already mandates.

## Capabilities

### New Capabilities

(none — this extends an existing capability rather than introducing a new one)

### Modified Capabilities

- `solution-service`:
  - **ADDED** "Worktree syncs from its project" — clean-gated, out-of-band,
    conflict surfaces as notice + row status.
  - **ADDED** "Project fast-forwards from a worktree" — four gates,
    `--ff-only`, refusals direct back to the worktree-side sync.
  - **ADDED** "Repository status surfaces on nodes" — outcome events carry a
    structured status; rows badge clean/dirty/conflict; initial state unknown;
    conflict is distinct from dirty.
  - **MODIFIED** "Git operations complete out of band and report by action
    id" — the enumerated invokes gain the two sync actions, and the outcome
    event additionally carries the post-run repository status.
  - **MODIFIED** scenario "Project and worktree nodes declare their actions"
    (under "Solution and project state lives in the service") — the worktree
    node also carries its sync descriptor, the project node its sync
    descriptor with per-worktree children.

## Impact

- **Contracts** (`crates/contracts/src/shared.rs`): `ActionDesc` gains a
  `children` field (+ new `ActionChild` type) — 7 construction sites across
  solution-service/asset-service/app gain an empty `children`; renderers that
  ignore children are untouched. `GitOpFinished` (solution-service contract)
  gains an optional structured status field — in-process `Payload`, no serde
  surface.
- **Service** (`crates/solution-service/`): contract action ids/labels +
  `project_actions`/`worktree_actions` gain the two descriptors; `service.rs`
  gains two async handlers following the pull/push shape (cheap checks at
  handle, gates + `git merge` in the blocking pool, outcome via
  `KIND_GIT_OP_FINISHED`); `git` module gains merge helpers behind async
  exits.
- **View** (`src/solution_overview/mod.rs`, `src/service_layer.rs`,
  `src/app.rs`): submenu rendering from descriptor children; per-row status
  badge store fed by outcome events; finish-notice label mapping.
- **Behavior elsewhere**: pull/push/create/delete notices change text only
  (raw id → label). No message kinds change; no spec outside
  `solution-service` changes.
