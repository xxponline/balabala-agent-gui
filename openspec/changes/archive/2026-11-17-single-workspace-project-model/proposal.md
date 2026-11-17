# Proposal

## Why

The app today models workspaces as a flat, app-global list of bare paths with
agents attached directly to workspace rows — there is no notion of the
project/module structure that real work happens in, and workspace config
lives in the app's own root rather than with the workspace. This change
collapses the multi-workspace list into a single open workspace whose
directory carries its own configuration, and introduces the
Solution → Project → Module model that becomes the anchor for agent
mounting (cwd = project root) and, later, LSP partitioning, worktrees, and
service injection.

## What Changes

- **BREAKING: single workspace per instance.** The sidebar's workspace list
  (add / delete / collapse / dedup) is removed. On startup the main window
  opens first: a stored most-recent workspace is restored directly; with no
  history, an in-window empty state offers a directory picker. Opening a
  directory means opening that workspace; a workspace switch control at the
  top of the left slot follows the same picker path.
- **Workspace file contract.** A valid workspace directory carries
  `.balabala.workspace.json` (identity + config, including the `file_tree`
  filter rules, unkeyed now that there is exactly one workspace) and
  `balabala.solutions.json` (solution items). Missing marker → confirm and
  create both files; corrupt marker → error dialog and exit; missing
  solutions file → legal empty workspace; corrupt solutions file or a
  solution item whose path does not exist on disk → error dialog and exit
  (error repair/parse assistance is a later addition). Cache and future
  worktrees also live under the workspace directory.
- **Persistence relocation.** App-root `.workspaces.json` is retired: its
  workspace list seeds the most-recent-workspace history once (stored in a
  global app directory), and its `file_tree` rules migrate into the
  workspace's own config file.
- **New domain model: Solution → Project → Module.** Solution items in
  `balabala.solutions.json` carry a `SolutionType` resolved through a
  registry; v1 ships `SimpleSolution` only — one item, one path, one
  Project, one root Module of the same name. Project dependencies and
  Module references are DAG-constrained; the v1 single-node output only
  establishes the shape, and validation activates when multi-output
  solutions arrive. Agent sessions mount under Projects (cwd = project
  root) and remain non-persisted.
- **Left slot becomes a Project/AgentEntity tree.** Workspace rows are gone;
  top-level rows are Projects (created from solution items), agent rows
  nest beneath them, and `[+ agent]` moves under the active project's row.
  An empty workspace offers a folder-picker action that adds a
  `SimpleSolution` (a full solution-management UI is deferred).
- **Gated workspace switching.** Switching while any session is
  `starting`/`running` is blocked until those sessions are closed; with only
  `exited` sessions remaining a confirmation notes their terminal screens
  are lost; with no sessions the switch proceeds directly.
- **Right slot unchanged in this change.** The file tree keeps its current
  workspace-root behavior and filtering semantics (only the rules'
  storage location changes); re-rooting it to Project/Module/File follows
  in a later change.

## Capabilities

### New Capabilities

- `project-model`: workspace solution configuration (`balabala.solutions.json`)
  → Solution registry → Project/Module structure: item schema, SimpleSolution
  output (one Project + same-named root Module), load-time validation,
  DAG-shaped dependency structure, and the empty-workspace solution-creation
  path.

### Modified Capabilities

- `workspace-management`: replace the workspace-list requirements with the
  single-instance lifecycle — startup restore/picker flow, marker-based
  identity and creation, config file location, switch-entry and gated
  switching semantics.
- `agent-shell`: sidebar tree re-roots from workspace rows to Project rows
  (display, creation button placement, no-project case); session cwd
  becomes the project root; selection/focus/status-bar clauses reference
  project nodes instead of workspace rows.
- `file-tree`: filter rules source moves from `.workspaces.json`'s
  `file_tree` section (keyed by workspace path) to the workspace's own
  `.balabala.workspace.json` (no keying).
- `dock-layout`: sidebar control references in the non-closable-left-dock
  requirement change (`+ new workspace` → workspace switch entry).
- `theming`: sidebar row hover/selection scenario references workspace rows
  that no longer exist (project rows replace them).

## Impact

- **Code**: `src/workspace/` (lifecycle, new file contract, MRU store),
  `src/sidebar/` (project tree, switch entry, empty states), `src/app.rs`
  (startup flow, switch gating, solution loading), `src/agent_entities/`
  (cwd = project root, naming), new solution/project model module,
  `src/workspace_property_source/` (rules load path).
- **Persistence**: app-root `.workspaces.json` retired (one-time seed);
  new workspace-local files created on workspace creation.
- **Specs**: `workspace-management` largely rewritten; new
  `project-model` capability; deltas to `agent-shell`, `file-tree`,
  `dock-layout`, `theming`.
- **Not affected**: center-tab behavior, terminal rendering/input,
  right-dock layout mechanics, session close/exit semantics.
