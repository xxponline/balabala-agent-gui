# Tasks

## 1. PaneView trait and AgentEntity wiring

- [x] 1.1 Add `ws_path(&self) -> &Path` to the `AgentEntity` trait in
      `src/agent_entities/mod.rs`. Verify `cargo check` passes.
- [x] 1.2 Store the spawn `cwd` in `PiAgentPty` as a new `ws_path: PathBuf`
      field, set it in `PiAgentPty::new`, and implement the new `ws_path()`
      method. Verify `cargo check` passes.
- [x] 1.3 Define `trait PaneView` in `src/ui/pane.rs` with `ws_path()`,
      `agent_name()`, `status()`, `screen()` methods. Make
      `AgentEntity: PaneView` ( blanket impl or direct trait inheritance).
      Verify `cargo check` passes.

## 2. InteractiveFrame widget

- [x] 2.1 Create `src/ui/common/mod.rs` declaring `pub mod
      interactive_frame;`. Register `mod common;` in `src/ui/mod.rs`.
      Verify `cargo check` passes.
- [x] 2.2 Implement `InteractiveFrame` in
      `src/ui/common/interactive_frame.rs` with builder methods `fill()`,
      `inner_margin()`, and a `show(ui, active, on_commit, contents)`
      method that:
      - wraps `egui::Frame::new()` with the configured fill/margin,
      - picks stroke color `(75,161,215)` when `active` else `(60,60,60)`,
      - allocates a click response covering the frame rect, calling
        `response.request_focus()` on click,
      - reads `ui.memory(|m| m.focused() == Some(Id::new("interactive_frame")))`,
      - writes `output.ime = Some(IMEOutput { .. })` only when focused,
      - iterates `ui.input(|i| i.raw.events)` and, when focused, forwards
        `Event::Ime(ImeEvent::Commit(s))` to the `on_commit` closure.
      Verify `cargo check` passes.

## 3. Pane rendering refactor

- [x] 3.1 Rewrite `src/ui/pane.rs::render` to take
      `view: Option<&dyn PaneView>` and `on_commit: &mut dyn FnMut(&str)`
      instead of the existing `Option<&Box<dyn AgentEntity>>`. Remove the
      hard-coded `egui::Frame::new().stroke(蓝色)` and replace with
      `InteractiveFrame::new().fill(深灰).inner_margin(8).show(ui, view.is_some(), on_commit, |ui| { ... status bar + vt100 grid ... })`.
      Verify `cargo check` passes.

## 4. App wiring

- [x] 4.1 In `src/app.rs`, construct the `on_commit` closure inline at the
      `pane::render` call site. The closure captures `&mut self` (or a
      pre-indexed agent) and calls `agent.send_input(s.as_bytes())`.
      Verify `cargo check` passes.
- [x] 4.2 Remove the previously-added `Event::Ime(ImeEvent::Commit(s))`
      match arm and the explicit `Event::Ime(_) => {}` arm from
      `app.rs::forward_keyboard`. Verify `cargo check` passes.

## 5. Documentation

- [x] 5.1 Append two entries to `Docs/pending-decisions.md`:
      "IME preedit preview in terminal" (updated) and "One-step pane focus
      (click sidebar row claims pane focus directly)". Verify the file
      renders cleanly.

## 6. Manual verification

- [x] 6.1 Launch the app, open a workspace, start an agent, click the pane
      area, and type "你好" using macOS Pinyin. Verify the committed text
      appears in the agent's terminal output (not raw `ni hao`). Verify
      English typing still works (no double-characters). Verify clicking a
      sidebar row switches selection but does not route IME until the pane
      is clicked.
