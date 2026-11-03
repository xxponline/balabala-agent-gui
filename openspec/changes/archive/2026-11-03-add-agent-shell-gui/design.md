# Design

## Context

The current `src/main.rs` is a single-file egui + portable-pty + vt100 demo
spawning `/bin/bash`. It proves the per-byte pipeline (pty reader thread →
mpsc → UI thread → vt100::Parser → egui render → keyboard bytes back) but
contains no multi-session management, no sidebar, no abstraction over the
backend. See `proposal.md` for motivation.

The project has no existing specs, so this design introduces the first real
architecture boundary: a `AgentEntity` trait that hides the pty + vt100
combo behind a stable interface.

## Goals / Non-Goals

**Goals:**

- A swap-friendly `AgentEntity` trait so the pty backend can be replaced
  (e.g. by a mock or a remote agent) without touching UI code.
- Clean module split: `agent_entities/` owns backend concerns, `ui/` owns
  egui rendering, `app.rs` wires them together.
- Render the vt100 grid cell-by-cell with colors and a static block cursor.
- Forward a minimal set of keystrokes to the active session.
- Observe subprocess exit via a typed event channel.

**Non-Goals:**

- Terminal resize synchronization (deferred, see `Docs/pending-decisions.md`).
- Graceful shutdown on close (hard kill only).
- Ctrl-combination keys (Ctrl+C / D / R).
- Mouse events, scrollback paging, IME preedit rendering.
- Persistence across GUI restarts.
- Multiple simultaneous parser instances per agent.

## Decisions

### D1: `AgentEntity` trait lives in `agent_entities/mod.rs`

```rust
pub enum AgentEvent { Output(Vec<u8>), Exited }

pub trait AgentEntity {
    fn name(&self) -> &str;
    fn screen(&self) -> &vt100::Screen;
    fn poll(&mut self);                       // drain event channel
    fn send_input(&mut self, bytes: &[u8]);
    fn shutdown(&mut self);                    // hard kill
    fn status(&self) -> SessionStatus;         // Starting / Running / Exited
}
```

Rationale: `screen()` exposed directly (not bytes) lets the UI read the
parsed grid without maintaining its own parser. Alternatives considered:
returning raw bytes from `poll()` and keeping the parser in UI layer —
rejected because it forced UI to own parser lifecycle and blocked future
"same agent, two views" scenarios.

### D2: `PiAgentPty` owns pty + parser + reader thread

`PiAgentPty::new(cmd: String)`:
- opens `native_pty_system().openpty(PtySize{rows:30, cols:100, ..})`
- spawns `CommandBuilder::new(cmd)` with `TERM=xterm-256color`
- takes `master.take_writer()` for stdin
- spawns a reader thread that loops `reader.read(...)` and sends
  `AgentEvent::Output(bytes)` over an `mpsc::Sender<AgentEvent>`
- on EOF sends `AgentEvent::Exited` then exits the thread
- holds a `vt100::Parser` internally; `poll()` feeds received bytes into it

Rationale: every backend (pty, future remote) will likely want its own
event source; a typed `AgentEvent` enum leaves room for future variants
(`Resize`, `TitleChange`) without breaking the channel contract.

### D3: `TerminalApp` holds `Vec<Box<dyn AgentEntity>>` + active index

```rust
struct TerminalApp {
    agents: Vec<Box<dyn AgentEntity>>,
    active_id: Option<usize>,
    next_name_idx: u32,
}
```

Rationale: `Box<dyn AgentEntity>` is runtime polymorphism; N=dozens is
cheap. Active selection is an index, not an ID, because we never need to
address an agent from outside the app. Closing an agent shifts the active
index accordingly (next item becomes active; `None` if empty).

### D4: egui 0.37 split `logic()` / `ui()`

- `logic()` iterates all agents and calls `poll()`. This is where bytes
  move from the channel into the vt100 parser.
- `ui()` draws sidebar + main pane. It must not mutate parser state.

Rationale: egui 0.37 separates logic from UI; abusing `ui()` for side
effects violates the contract. `poll()` is intentionally cheap (drain +
`parser.process`).

### D5: Rendering strategy — line-by-line, color-run coalescing

Walk `screen.cell(r, c)` row by row; merge consecutive cells with the same
fg/bg into one `ui.colored_label(...)`. For the active row, invert fg/bg at
the cursor column.

Rationale: 100×30 = 3000 raw Labels per frame would be slow. Coalescing
color runs drops this to ~rows×segments per frame.

### D6: Keyboard input forwarding

In `ui()`, read `ui.input(|i| i.raw.events.clone())`:
- `Event::Text(s)` → `agent.send_input(s.as_bytes())`
- `Event::Key { key, pressed: true, .. }` match:
  Enter → `\r`, Backspace → `\x7f`, Tab → `\t`,
  ArrowUp/Down/Left/Right → `\x1b[A`/`B`/`C`/`D`

No Ctrl handling (deferred). Window-level focus loss is naturally handled
by eframe not delivering key events when the OS window is unfocused.

### D7: Status badge transitions

`SessionStatus::Starting` until first `AgentEvent::Output` arrives →
`Running`. `AgentEvent::Exited` flips to `Exited`. The UI must not auto-
remove exited sessions; the user clicks the close button to do so.

### D8: Module layout

```
src/
  main.rs
  app.rs
  agent_entities/
    mod.rs            // trait + enum + SessionStatus
    pi_agent_pty.rs    // PiAgentPty
  ui/
    mod.rs            // fg_to_egui / bg_to_egui helpers
    sidebar.rs
    pane.rs
```

Rationale: `agent_entities/` (user-requested name) vs a shorter `agent/`;
trait lives alongside its reference implementation, future backends
(`mock.rs`, `remote.rs`) drop into the same directory.

## Risks / Trade-offs

- **Unbounded mpsc channel** → pty output flooding (e.g. `cat hugefile`)
  could grow memory. Mitigation: documented in
  `Docs/pending-decisions.md`; acceptable for MVP scale.
- **Hard kill on close** → could lose in-flight `pi` work. Mitigation:
  documented in `Docs/pending-decisions.md`; graceful shutdown deferred.
- **No resize** → full-screen programs (`htop`, `vim`) will render at a
  fixed 100×30 grid regardless of window size. Mitigation: documented in
  `Docs/pending-decisions.md`.
- **Static block cursor (no blink)** → less terminal-like, but visually
  unambiguous. Acceptable for MVP.
- **Only printable + basic control keys forwarded** → Ctrl+C cannot
  interrupt `pi`. Mitigation: documented in `Docs/pending-decisions.md`;
  user can close the session from the sidebar instead.
- **`pi` CLI not on `PATH`** → sessions immediately show `exited` badge.
  Mitigation: the `starting` → `exited` transition makes the failure mode
  visible; no extra error UI in this change.

## Open Questions

- None. All material decisions were resolved during exploration and are
  recorded either in this design or in `Docs/pending-decisions.md`.
