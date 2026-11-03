# Tasks

## 1. Dependencies and module scaffolding

- [x] 1.1 Add `rfd`, `serde`, and `serde_json` to `Cargo.toml` and verify `cargo build`
      succeeds with the new dependencies resolved.
- [x] 1.2 Create `src/workspace/mod.rs` and `src/workspace/store.rs` as empty modules,
      register `mod workspace;` in `src/main.rs`, and verify `cargo check` passes.

## 2. Workspace persistence layer

- [x] 2.1 Implement `Persisted { workspaces: Vec<PathBuf> }` (de)serialization in
      `src/workspace/store.rs` and verify a round-trip write/read produces the same
      path list.
- [x] 2.2 Implement `store::load() -> Result<Vec<PathBuf>, String>` that reads
      `.workspaces.json` from the project root; return `Ok(vec![])` when the file
      does not exist, and `Err` on parse or IO failure. Verify by unit-testing with
      a temp file.
- [x] 2.3 Implement `store::save(paths: &[PathBuf]) -> Result<(), String>` that
      atomically writes the JSON file; verify by writing a known list and reading it
      back.

## 3. Workspace entity

- [x] 3.1 Define `Workspace { path, collapsed, agents, next_agent_idx }` in
      `src/workspace/mod.rs` with `display_name()` returning the last path component
      and `full_path_str()` returning the absolute path. Verify via unit tests on a
      sample `PathBuf`.
- [x] 3.2 Implement `Workspace::new_agent_name(&mut self) -> String` that returns
      `"<dirname>/pi-<N>"` and increments `next_agent_idx`. Verify the counter does
      not reuse numbers across calls.

## 4. PiAgentPty cwd support

- [x] 4.1 Change `PiAgentPty::new` signature to accept `cwd: PathBuf` and pass it to
      `CommandBuilder::cwd(...)`. Verify by spawning a test agent with a known cwd
      and confirming the child process sees that cwd (e.g. spawn `pwd` and match
      output).

## 5. TerminalApp restructure

- [x] 5.1 Replace `agents: Vec<Box<dyn AgentEntity>>` and `active_id: Option<usize>`
      with `workspaces: Vec<Workspace>` and `active: Option<(usize, usize)>` in
      `TerminalApp`. Add `save_error: Option<String>`. Verify `cargo check` passes
      with temporary stubs for the sidebar render call.
- [x] 5.2 In `TerminalApp::new()`, call `store::load()`; on `Err` show an rfd
      error dialog and propagate the failure out of `new()` so `main.rs` exits
      non-zero. Verify by pointing the app at a directory with a corrupt
      `.workspaces.json` and confirming the error dialog appears.
- [x] 5.3 Implement `create_workspace()` that calls `rfd::FileDialog::pick_folder()`,
      canonicalizes the path, dedupes against existing workspaces, appends a new
      `Workspace { collapsed: false, .. }`, and calls `store::save`. Store save
      errors in `self.save_error`. Verify by picking a folder and seeing it appear
      in the sidebar and in `.workspaces.json`.
- [x] 5.4 Implement `create_agent(ws_idx)` that calls `Workspace::new_agent_name`,
      spawns `PiAgentPty::new(name, "pi", workspace.path)`, pushes the agent, and
      sets `self.active = Some((ws_idx, agent_idx))`. Verify by clicking the new
      button and seeing `foo/pi-1` appear.
- [x] 5.5 Implement `close_workspace(ws_idx)` that is a no-op when the workspace has
      agents, otherwise removes it and calls `store::save`. Verify by trying to close
      a non-empty workspace (nothing happens) and an empty one (row disappears).
- [x] 5.6 Update `close_agent(ws_idx, agent_idx)` and `forward_keyboard` to operate
      on the `(ws_idx, agent_idx)` pair. Verify keyboard input still reaches the
      active agent after refactor.

## 6. Sidebar tree rendering

- [x] 6.1 Rewrite `src/ui/sidebar.rs` to render a `[+ new workspace]` button at the
      top, then iterate workspaces: each row shows collapse toggle, short name
      (with hover tooltip of absolute path), and a workspace close `×` (disabled
      when agents exist). Expanded workspaces indent their agents below and show a
      `[+ agent]` button at the end.
- [x] 6.2 Replace `SidebarAction` with a sum type covering: `NewWorkspace`,
      `ToggleCollapse(usize)`, `CloseWorkspace(usize)`, `NewAgent(usize)`,
      `SelectAgent(usize, usize)`, `CloseAgent(usize, usize)`. Verify the sidebar
      returns the right variant for each click target.
- [x] 6.3 Render `TerminalApp.save_error` as a red colored label below the top
      button when present. Verify by forcing a save failure (e.g. make
      `.workspaces.json` a directory) and seeing the red message.

## 7. Main pane status bar

- [x] 7.1 Update `src/ui/pane.rs` status bar to render
      `● <workspace absolute path> — pi-<N> [<status>]`. Verify the bar shows the
      full path and prefixed agent name for the active session.

## 8. Documentation

- [x] 8.1 Append three entries to `Docs/pending-decisions.md`: unified sidebar
  event handler, rfd sync-blocking caveat, and persistence-path migration. Verify
  the file renders cleanly under `cat`. Verify
  the file renders cleanly under `cat`.
- [x] 8.2 Manually verify the full UX end-to-end: launch the app, add two
  workspaces, create agents in each, confirm `pi-1` names are independent per
  workspace, restart the app, and confirm workspaces persist while agents do not.
