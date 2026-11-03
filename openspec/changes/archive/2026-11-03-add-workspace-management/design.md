# Design

## Context

`TerminalApp` currently holds a flat `agents: Vec<Box<dyn AgentEntity>>` plus a
global `next_name_idx`. `PiAgentPty::new` spawns `pi` without setting `cwd`, so
the child inherits the shell's working directory. There is no persistence, no
grouping, and no notion of a project root. See `proposal.md` for the motivation
behind introducing a workspace layer; see `specs/workspace-management/spec.md`
and `specs/agent-shell/spec.md` for the behavioral contract.

## Goals / Non-Goals

**Goals:**

- Introduce a `Workspace` entity that owns an absolute path and a list of agents.
- Nested sidebar tree rendering (workspaces → agents) with collapse/expand.
- Per-workspace agent naming counter, displayed as `<dirname>/pi-N`.
- Spawn `pi` with `CommandBuilder::cwd(workspace.path)`.
- Persist workspace paths only (no agents, no collapse state) to
  `.workspaces.json` in the project root.

**Non-Goals:**

- Restoring agents across restarts (out of scope per user decision; agents are
  ephemeral).
- Migrating the persistence file to `~/.config` or platform-specific app
  support dirs (deferred to pending-decisions).
- Replacing rfd's synchronous `pick_folder()` with the async variant (deferred
  to pending-decisions).
- Unifying the sidebar action handling behind an event bus (deferred to
  pending-decisions).
- Terminal resize, Ctrl-key forwarding, graceful child shutdown (existing
  pending-decisions; unaffected by this change).

## Decisions

### Decision 1 — Data model: `Vec<Workspace>` replacing flat `Vec<AgentEntity>`

```rust
struct Workspace {
    path: PathBuf,
    collapsed: bool,
    agents: Vec<Box<dyn AgentEntity>>,
    next_agent_idx: u32,
}

struct TerminalApp {
    workspaces: Vec<Workspace>,
    active: Option<(usize, usize)>,   // (ws_idx, agent_idx)
    save_error: Option<String>,        // last save failure, shown in UI
}
```

**Rationale:** Matches the user's chosen nested-tree layout. `active` becomes a
pair so the main pane can locate both the workspace and the agent within it.
The global `next_name_idx` is removed entirely.

**Alternatives considered:**

- Keep a flat `agents` Vec and add a `workspace_id` foreign key on each agent.
  Rejected — makes sidebar grouping O(N) per render and complicates deletion.
- Use `HashMap<PathBuf, Workspace>`. Rejected — sidebar order would be
  nondeterministic; we want insertion order.

### Decision 2 — New module `src/workspace/`

Two files:

- `src/workspace/mod.rs` — `Workspace` struct, display-name helpers, and a
  `WorkspaceAction` enum returned by the sidebar render.
- `src/workspace/store.rs` — `load() -> Result<Vec<PathBuf>, String>` and
  `save(paths: &[PathBuf]) -> Result<(), String>`.

**Rationale:** Keeps persistence IO out of `app.rs` and gives the sidebar a
single place to import workspace concerns from. Mirrors the existing
`agent_entities/` module layout.

### Decision 3 — `PiAgentPty::new` gains a `cwd` parameter

```rust
impl PiAgentPty {
    pub fn new(name: String, cmd: String, cwd: PathBuf) -> Result<Self, String> {
        let mut cb = CommandBuilder::new(&cmd);
        cb.cwd(cwd);
        cb.env("TERM", "xterm-256color");
        // …
    }
}
```

**Rationale:** `portable_pty::CommandBuilder::cwd` is the canonical way to set
the child's working directory. No other changes to the pty lifecycle.

### Decision 4 — Persistence format

```json
{ "workspaces": ["/Users/me/code/foo", "/Users/me/code/bar"] }
```

Using `serde` + `serde_json` with a small struct:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct Persisted {
    workspaces: Vec<PathBuf>,
}
```

**Rationale:** Only paths are persisted (user decision Q9). Keeping the schema
trivial means future migrations (e.g. adding collapse state) are additive.

### Decision 5 — rfd integration

`rfd::FileDialog::new().pick_folder()` is called synchronously inside the
sidebar's `+ new workspace` button handler.

**Rationale:** Simplest path. The known blocking risk on macOS is recorded in
`Docs/pending-decisions.md` per user decision Q20. We do not introduce
`AsyncFileDialog` in this change.

### Decision 6 — Save-error UI surface

`TerminalApp.save_error: Option<String>`. When `store::save` returns `Err`,
app.rs stores the message. The sidebar render reads it and displays a red
colored label at the top of the sidebar below the `+ new workspace` button.

**Rationale:** Smallest viable surface for "on-screen error indicator"
required by the spec. No new modal system.

### Decision 7 — Load-failure startup path

In `TerminalApp::new()`, call `store::load()`:

- `Ok(paths)` → build `Vec<Workspace>` with `collapsed: false`, empty agents,
  `next_agent_idx: 1`.
- `Err(e)` → show a `rfd::MessageDialog` (or `eprintln!` + early exit) and
  return `Err` from `new()`. `main.rs` propagates the error and exits non-zero.

**Rationale:** User decision Q19 requires blocking startup on corrupt file.
Using a native message dialog matches the folder picker aesthetic.

## Risks / Trade-offs

- **rfd sync call blocks egui main thread** → Known. Documented in
  pending-decisions. Acceptable for MVP because the folder picker is a user
  gesture; the freeze only lasts as long as the dialog is open.
- **Two pi subprocesses in the same workspace simultaneously** → Allowed.
  User decision Q6 chose to dedupe *workspaces*, not agents. Concurrent pi
  processes in one directory may contend on `.pi/` state; documented as a
  known limitation, not enforced.
- **`.workspaces.json` written to project root** → Couples the app to the
  repo it is launched from. Documented in pending-decisions; acceptable while
  this is a demo/development tool.
- **Path canonicalization** → Dedup relies on string equality of absolute
  paths. Symlinked paths (`~/code/foo` vs `/Users/me/code/foo`) may slip
  through as duplicates. Mitigation: call `std::fs::canonicalize` on picked
  paths before comparing and persisting.
- **Save failure after in-memory change** → Spec requires we keep the
  in-memory state and surface the error. Next successful save will overwrite
  the file; user may lose the change if they quit before fixing. Accepted.

## Migration Plan

No data migration needed — there is no existing persistence. On first launch
after this change, `.workspaces.json` does not exist and the app starts with an
empty sidebar, matching the previous behavior. Existing agent sessions from
before the change are not preserved (they were never persisted).

Rollback: revert the commit; `.workspaces.json` becomes an unused file in the
project root and can be deleted manually.
