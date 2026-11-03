# Tasks

## 1. Module scaffolding

- [x] 1.1 Replace `src/main.rs` with a thin eframe entry point that constructs
  `TerminalApp` and calls `eframe::run_native`. Verify: `cargo check` passes
  and the app still opens a window (currently an empty panel).
- [x] 1.2 Create `src/app.rs` with an empty `TerminalApp` struct and an
  `eframe::App` impl whose `logic()` and `ui()` are stubs. Verify:
  `cargo check` passes.
- [x] 1.3 Create empty module files: `src/agent_entities/mod.rs`,
  `src/agent_entities/pi_agent_pty.rs`, `src/ui/mod.rs`,
  `src/ui/sidebar.rs`, `src/ui/pane.rs`. Declare them in `main.rs` /
  `app.rs` as needed. Verify: `cargo check` passes with no unused-module
  warnings.

## 2. `AgentEntity` trait and event type

- [x] 2.1 In `agent_entities/mod.rs`, define `pub enum AgentEvent { Output(Vec<u8>), Exited }`
  and `pub enum SessionStatus { Starting, Running, Exited }`. Verify:
  `cargo check` passes.
- [x] 2.2 Define `pub trait AgentEntity` with methods: `name(&self) -> &str`,
  `screen(&self) -> &vt100::Screen`, `poll(&mut self)`,
  `send_input(&mut self, bytes: &[u8])`, `shutdown(&mut self)`,
  `status(&self) -> SessionStatus`. Verify: `cargo check` passes.

## 3. `PiAgentPty` implementation

- [x] 3.1 Implement `PiAgentPty::new(cmd: String)` that opens a pty of
  100×30, spawns `CommandBuilder::new(cmd)` with `TERM=xterm-256color`,
  takes the master writer, and holds a `vt100::Parser`. Verify:
  construct a `PiAgentPty("echo")` in a unit test and confirm
  `parser.screen().contents()` eventually contains the echo output.
- [x] 3.2 Spawn the reader thread that loops `reader.read(&mut buf)`,
  sends `AgentEvent::Output(bytes)` on a channel, and sends
  `AgentEvent::Exited` on EOF. Verify: unit test above observes an
  `Exited` event after the child exits.
- [x] 3.3 Implement `poll()` to drain the channel, feed output bytes into
  the parser, and transition `status` from `Starting` → `Running` on first
  output, and `Running`/`Starting` → `Exited` on the `Exited` event.
  Verify: unit test asserts the status transitions.
- [x] 3.4 Implement `send_input()` by writing bytes to the master writer
  and flushing. Implement `shutdown()` by calling `child.kill()`. Verify:
  spawn `PiAgentPty("cat")`, send `b"hi\r"`, assert the parser screen
  eventually contains `hi`; then `shutdown()` and assert the child has
  exited.

## 4. UI helpers

- [x] 4.1 In `ui/mod.rs`, add `fg_to_egui(Color) -> egui::Color32` and
  `bg_to_egui(Color) -> egui::Color32` covering Default, the 8–15
  indexed ANSI colors, and Rgb. Verify: a small unit test asserts
  `fg_to_egui(Color::Idx(1))` returns the expected red RGB.

## 5. Sidebar

- [x] 5.1 In `ui/sidebar.rs`, render a `+` button and a vertical list of
  agent items. Each item shows the name and a status badge (colored dot
  or text). Verify: run the app, click `+` three times, observe three
  entries in the sidebar.
- [x] 5.2 Highlight the active sidebar item. Verify: clicking different
  items moves the highlight.
- [x] 5.3 Show a close (`x`) button on hover of each item. Wire it to a
  callback that the app uses to remove the agent. Verify: hovering an
  item reveals `x`; clicking `x` removes the item from the sidebar.

## 6. Main pane

- [x] 6.1 In `ui/pane.rs`, render the active agent's vt100 grid row by
  row, coalescing consecutive cells of equal fg/bg into one colored
  label. Use monospace font. Verify: spawn `PiAgentPty("ls --color=auto")`
  and observe colored text in the main pane.
- [x] 6.2 Draw the static block cursor by inverting fg/bg at
  `screen.cursor_position()`. Verify: run `cat` and type a few
  characters; the cursor block visibly tracks the next input position.
- [x] 6.3 While the active agent's status is `Starting`, render a
  "waiting for pi to start…" placeholder. Remove it once status becomes
  `Running`. Verify: a freshly created agent shows the placeholder until
  first output.
- [x] 6.4 Wrap the pane in a highlighted frame / border and add a top
  status bar showing the active agent's name. Verify: the border and
  status bar visually distinguish the active pane.
- [x] 6.5 If no agent is active, render an empty placeholder ("no agent
  selected — press + to create one"). Verify: with all agents closed,
  the main pane shows this placeholder.

## 7. Keyboard input forwarding

- [x] 7.1 In `ui/pane.rs` (or `app.rs`), read `ui.input(|i| i.raw.events.clone())`
  each frame. Forward `Event::Text(s)` bytes to the active agent's
  `send_input()`. Verify: typing "hello" in a `cat` session echoes it
  back into the rendered grid.
- [x] 7.2 Forward key events: Enter → `\r`, Backspace → `\x7f`,
  Tab → `\t`, ArrowUp/Down/Left/Right → `\x1b[A`/`B`/`C`/`D`. Verify:
  pressing Enter in a `cat` session inserts a new line; pressing Backspace
  removes the last character.

## 8. App wiring

- [x] 8.1 In `app.rs`, hold `agents: Vec<Box<dyn AgentEntity>>`,
  `active_id: Option<usize>`, and `next_name_idx: u32`. Implement the `+`
  callback to push a new `PiAgentPty::new("pi".into())` named
  `pi-{next_name_idx}`, set it active, and increment the index.
  Verify: clicking `+` creates `pi-1`, then `pi-2`, etc.
- [x] 8.2 In `logic()`, iterate all agents and call `poll()`. Verify:
  bytes from subprocesses reach the parser within ~50 ms of arrival.
- [x] 8.3 Closing an agent (from sidebar) calls `agent.shutdown()`,
  removes it from the vec, and shifts `active_id`: if the closed agent was
  active, switch to the next remaining agent or set `active_id = None`.
  Verify: closing the active agent when others exist moves highlight to
  the next one; closing the last agent renders the empty placeholder.
- [x] 8.4 Request repaint every 50 ms so output and cursor stay fresh
  between OS events. Verify: a continuously emitting subprocess (e.g.
  `yes`) updates the grid without the user interacting with the window.

## 9. Manual end-to-end verification

- [x] 9.1 Launch the built app. Click `+`, observe `pi-1` appears and
  becomes active. If `pi` is on PATH, the pane eventually shows pi's
  prompt; otherwise it flips to `exited` badge. Verify: visual inspection.
- [x] 9.2 Click `+` twice more to create `pi-2` and `pi-3`. Click each
  sidebar item; confirm the main pane switches and shows that agent's
  last output. Verify: visual inspection.
- [x] 9.3 Click `x` on `pi-2`; confirm it disappears and the active
  selection moves sensibly. Verify: visual inspection.
- [x] 9.4 In an active `pi` session, type a short prompt and press Enter;
  confirm characters appear in the rendered grid. Verify: visual
  inspection.

## Workflow follow-up

- Archive the change after review requirements are satisfied.
- The deferred concerns tracked in `Docs/pending-decisions.md` are
  intentionally NOT part of this change.
