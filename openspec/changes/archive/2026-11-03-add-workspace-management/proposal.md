# Proposal

## Why

Right now every `pi` agent session spawns in the shell's inherited cwd with no
notion of a project directory. Users who work across multiple codebases have no
way to tell `pi` "start in this repo" per session, and the sidebar is a flat list
of `pi-N` items with no grouping. We need a workspace layer that owns a path and
groups the agents running inside it.

## What Changes

- **BREAKING** sidebar layout: from a flat agent list to a two-level collapsible
  tree. Top level is workspaces (each owns an absolute path). Indented under each
  workspace are its agents.
- **BREAKING** agent naming: from a global `pi-N` auto-increment to a
  per-workspace counter, displayed as `<dirname>/pi-N` (e.g. `foo/pi-1`). The
  global `next_name_idx` is removed.
- New: a `[+ new workspace]` button at the top of the sidebar that opens a native
  folder picker (`rfd`), dedupes by absolute path, and appends the workspace.
- New: each expanded workspace shows a `[+ agent]` button at the bottom of its
  agent list. Clicking it spawns `pi` with `cwd = workspace.path`.
- New: workspace hover shows a close `×` button. The button is disabled while
  the workspace still has agents.
- New: workspace paths are persisted to `.workspaces.json` in the project root
  on every mutation (add/remove). Agents are **not** persisted.
- New: startup reads `.workspaces.json`. If the file is corrupted or unreadable,
  the app shows an error dialog and exits. If the file does not exist, the app
  starts with an empty workspace list.
- New: main pane status bar shows the full workspace path plus agent name, e.g.
  `/Users/me/code/foo — pi-1 [Running]`.
- Sidebar workspace row displays only the path's last component; a hover tooltip
  shows the absolute path.
- Clicking a workspace row only toggles collapse/expand. It does **not** switch
  the main pane.
- Adding `rfd`, `serde`, `serde_json` as Cargo dependencies.

## Capabilities

### New Capabilities

- `workspace-management`: Owning a list of on-disk workspaces — their creation
  via folder picker, deletion (only when empty), path dedup, collapse state,
  and JSON persistence across restarts.

### Modified Capabilities

- `agent-shell`: Sidebar session creation and display change from a flat global
  list into a per-workspace nested list with per-workspace naming. Agent
  spawn gains an explicit `cwd` inherited from the parent workspace. Main pane
  status bar gains the workspace path prefix.

## Impact

- **Code**:
  - New module `src/workspace/` (`mod.rs`, `store.rs`).
  - `src/app.rs`: `TerminalApp.agents: Vec<…>` becomes `workspaces: Vec<Workspace>`;
    `active_id: Option<usize>` becomes `Option<(usize, usize)>`.
  - `src/ui/sidebar.rs`: rewrites the render loop to walk workspaces and their
    agents; emits a richer `SidebarAction` enum.
  - `src/ui/pane.rs`: status bar gains the full path prefix.
  - `src/agent_entities/pi_agent_pty.rs`: `PiAgentPty::new` gains a `cwd`
    parameter, passed to `CommandBuilder::cwd`.
- **Dependencies**: adds `rfd`, `serde`, `serde_json`.
- **Persistence**: new `.workspaces.json` in the project root.
- **Docs**: append 3 entries to `Docs/pending-decisions.md` (unified sidebar
  event handler, rfd sync-blocking caveat, persistence path migration).
- **Specs**: the existing `agent-shell` spec's sidebar-related requirements are
  amended; a new `workspace-management` spec is introduced.
