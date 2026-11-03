# Design

## Context

`TerminalApp::forward_keyboard` in `src/app.rs` reads `ctx.input(|i|
i.raw.events)` each frame and matches on `egui::Event`. Investigation of
egui-winit 0.37 + egui 0.37 showed that IME input on macOS only flows into
the app when:

1. Some widget calls `memory.request_focus(id)` (i.e. `owns_ime_events(id)`
   returns `true`), **and**
2. That widget writes `PlatformOutput.ime = Some(IMEOutput { .. })` each
   frame.

Only then does egui-winit call `winit::Window::set_ime_allowed(true)` and
route macOS `WindowEvent::Ime` into `raw.events` as `Event::Ime(..)`. Our
pane is currently a pile of `Label`s wrapped in a static `egui::Frame`, so
neither condition is met. The previous "minimal fix" of adding a match arm
for `Event::Ime(Commit)` could not work — the event never arrives.

See `proposal.md` for motivation. See `specs/agent-shell/spec.md` for the
behavioral delta.

## Goals / Non-Goals

**Goals:**

- Introduce `InteractiveFrame` as a reusable egui widget that owns the
  "I want IME" contract on behalf of its contents.
- IME-committed strings (Chinese, Japanese, Korean, etc.) reach the active
  agent's pty stdin as raw UTF-8 bytes.
- Active vs inactive pane is visually distinguished by stroke color.
- Pane rendering depends on a `PaneView` trait, not the concrete
  `AgentEntity`.

**Non-Goals:**

- Rendering IME preedit (candidate preview) inside the terminal grid.
- One-step focus (clicking a sidebar row claims pane focus directly).
- Handling `DeleteSurrounding` from the IME.
- Mouse / scroll / clipboard takeover by `InteractiveFrame`.
- Changing `Event::Key` handling in `app.rs::forward_keyboard`.

## Decisions

### Decision 1 — `InteractiveFrame` is a builder-style wrapper around `egui::Frame`

```rust
// src/ui/common/interactive_frame.rs
pub struct InteractiveFrame {
    fill: Color32,
    inner_margin: Margin,
    active_stroke: Stroke,
    inactive_stroke: Stroke,
    id: Id,
}

impl InteractiveFrame {
    pub fn new() -> Self;
    pub fn fill(mut self, Color32) -> Self;
    pub fn inner_margin(mut self, Margin) -> Self;

    pub fn show<R>(
        self,
        ui: &mut egui::Ui,
        active: bool,
        on_commit: &mut dyn FnMut(&str),
        contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R;
}
```

**Rationale:** Matches `egui::Frame` ergonomics. Callers port their existing
`egui::Frame::new()..show(..)` call site mechanically. `active_stroke` and
`inactive_stroke` are hard-coded to `(75, 161, 215)` and `(60, 60, 60)`
respectively — no caller configuration needed for this change.

**Alternatives considered:**

- Thin wrapper that accepts an external `egui::Frame`. Rejected — pushes
  stroke choice back to the caller, defeating the purpose of encapsulating
  active/inactive visuals.

### Decision 2 — Global fixed ID

```rust
const FRAME_ID: Id = Id::new("interactive_frame");
```

**Rationale:** Only one pane is rendered at a time (the active selection).
No need to scope the ID per-agent. egui's focus system is global anyway.

### Decision 3 — Two-step focus model

- Clicking a sidebar agent row switches `active` selection (blue stroke),
  but does **not** request focus.
- Clicking anywhere inside the `InteractiveFrame` rect requests focus via
  `response.request_focus()`.
- Once focused, egui routes subsequent clicks on other widgets (sidebar
  buttons, etc.) away from us — we do not need to track this ourselves.

Each frame, `InteractiveFrame::show` reads `ui.memory(|m| m.focused() ==
Some(FRAME_ID))`. Only when this is `true` does it write
`PlatformOutput.ime = Some(IMEOutput { .. })`.

**Rationale:** Trusting egui's focus system avoids a parallel `focused:
bool` field in `TerminalApp`. We never have to handle "window lost focus"
or "sidebar grabbed focus" — egui already did.

**Cost:** Two clicks to start typing. Recorded as a pending-decision for
the future one-step UX.

### Decision 4 — Event handling split

```
InteractiveFrame::show internally:
    for ev in ui.input(|i| i.raw.events.clone()):
        match ev {
            Event::Ime(ImeEvent::Commit(s)) if has_focus => on_commit(s),
            _ => {}
        }

app.rs::forward_keyboard unchanged in shape:
    for ev in ...:
        match ev {
            Event::Text(s) => send_input(s.as_bytes()),
            Event::Key { .. } => /* existing */,
            _ => {}   // Event::Ime deliberately not matched here
        }
```

**Rationale:** egui's `raw.events` is a per-frame snapshot — multiple
consumers can read it without "consuming" it. Splitting by event type is
clean: `InteractiveFrame` owns everything IME, `app.rs` owns everything
keyboard.

### Decision 5 — `PaneView` trait

```rust
// src/ui/pane.rs
pub trait PaneView {
    fn ws_path(&self) -> &Path;
    fn agent_name(&self) -> &str;
    fn status(&self) -> SessionStatus;
    fn screen(&self) -> &vt100::Screen;
}

// src/agent_entities/mod.rs
pub trait AgentEntity: PaneView {
    /* existing methods unchanged */
}
```

`PiAgentPty` already has `name()`, `status()`, `screen()` — the only new
method it must provide is `ws_path()`, which it gains by storing the cwd
path it was spawned with.

**Rationale:** Decouples pane rendering from the concrete agent type. If
we later add `MockAgent` or `RemoteAgent`, pane.rs does not change.

### Decision 6 — `on_commit` closure is pre-bound in app.rs

```rust
// app.rs
let on_commit: &mut dyn FnMut(&str) = &mut |s: &str| {
    if let Some((ws_idx, agent_idx)) = self.active {
        if let Some(agent) = self.workspaces[ws_idx].agents.get_mut(agent_idx) {
            agent.send_input(s.as_bytes());
        }
    }
};
pane::render(ui, view, on_commit);
```

**Rationale:** pane.rs does not know about agents at all. It just forwards
the committed string to whoever bound the closure.

## Risks / Trade-offs

- **Two-click UX** — Users must click the pane after switching agents.
  Mitigation: pending-decision entry.
- **Global ID means only one pane can ever be focused** — Acceptable,
  since we render only one pane at a time.
- **Preedit invisible** — Still. Pending-decision entry updated.
- **`PlatformOutput.ime.rect` uses the whole frame rect** — winit will place
  the candidate popup somewhere near the frame, not at the exact cursor.
  Acceptable for MVP; refining to cursor-cell rect is future work.
- **`PiAgentPty` must store `ws_path`** — New field, small change.
