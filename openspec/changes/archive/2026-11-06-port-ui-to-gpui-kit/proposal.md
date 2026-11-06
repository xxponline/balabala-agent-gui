# Proposal

## Why

The app is evolving from a single-purpose agent shell into a multi-tool
desktop workbench (log analysis, code editing, UI design views alongside
agent terminals). egui's immediate-mode model rebuilds the whole widget
tree every frame, which degrades exactly where the roadmap is headed —
large scrolling text views — and it has already forced a borrow-checker
workaround (`unsafe` screen pointer in `app.rs`) plus custom IME plumbing
(`InteractiveFrame`) that is hard to maintain. gpui-kit (GPUI's reactive,
retained model with virtualized lists, a text-view stack, dock/tabs, and an
editor foundation) matches both the performance and the maintainability
goals, so we re-found the UI layer on it now, while the UI surface is still
small (~600 lines) and fully covered by existing specs.

## What Changes

- **BREAKING (behavior)**: One-step focus model. Clicking an agent item in
  the sidebar switches the active session AND gives its pane keyboard/IME
  focus in a single step. The current two-step model (click sidebar to
  select, click pane to focus) is removed; the contradictory focus clauses
  in `agent-shell` are reconciled accordingly.
- Rewrite the UI layer (`src/main.rs`, `src/app.rs`, `src/ui/**`) on
  gpui-kit 0.9.0: shell layout, sidebar, terminal pane (vt100 grid with
  fg/bg colors and inverted-block cursor), keyboard forwarding, and IME
  commit handling via GPUI's `InputHandler` bridge instead of egui
  `Event::Ime` + `PlatformOutput::ime`.
- Introduce a framework-neutral `Pane` seam in the shell: the main area
  hosts pane kinds (today: agent terminal) so future tools (log views,
  editors, design canvases) plug in without reworking shell chrome. This
  change defines the seam only; no new pane kinds are delivered here.
- Replace frame-driven polling (`logic()` + `request_repaint_after(50ms)`)
  with event-driven updates: PTY reader events wake the UI via
  `cx.notify()`, with throttled re-render during bursts of output.
- Remove the `unsafe` screen-pointer workaround: agent sessions become
  gpui entities with safe `read(cx)`/`update(cx)` access.
- Remove the `egui` and `eframe` dependencies and all egui-coupled code
  (`src/ui/common/interactive_frame.rs`, `Color32` mapping, egui event
  forwarding).
- Reword the `agent-shell` keyboard requirement's egui-specific vocabulary
  (`Event::Text`, `Event::Ime(ImeEvent::Commit(s))`) into
  framework-neutral behavior descriptions. Scenarios keep their intent.
- No change to behavior of workspace management, session lifecycle,
  terminal output rendering semantics, key mapping, or persistence.

## Capabilities

### New Capabilities

None. The `Pane` seam is an architectural affordance, not user-visible
behavior; future pane kinds (log analysis, code editing, UI design) will
introduce their own capabilities when they land.

### Modified Capabilities

- `agent-shell`: (1) "Keyboard input forwarding" requirement — replace
  egui event-type vocabulary with framework-neutral wording (printable
  characters and IME-committed strings, Enter/Backspace/Tab/arrows, same
  byte mappings; preedit and Ctrl-combos still excluded). (2) Reconcile
  its IME focus clause with "Session selection focus model" into a
  one-step model (that requirement is already one-step and is unchanged):
  selecting a sidebar agent item also grants keyboard and IME focus to its
  pane; losing OS window focus remains the only condition under which
  input is not forwarded. (3) "Pane keyboard focus claim" requirement —
  its two-step clause (sidebar selection does NOT claim pane focus) and
  the corresponding scenario are rewritten to the same one-step model.

## Impact

- **Code**: `src/main.rs`, `src/app.rs`, `src/ui/` (sidebar, pane,
  interactive_frame, color mapping) rewritten against gpui-kit;
  `src/ui/common/interactive_frame.rs` deleted. A `Pane` seam is added to
  the shell.
- **Dependencies**: add `gpui-kit = "0.9.0"` (already staged in
  `Cargo.toml`); remove `egui`, `eframe`. Untouched: `portable-pty`,
  `vt100`, `rfd`, `serde`/`serde_json`.
- **Untouched**: `src/agent_entities/` (pty + vt100 + pi subprocess),
  `src/workspace/` (model + `.workspaces.json` store) — the UI port sits
  on the same backend contract.
- **Specs**: delta to `agent-shell` only; `workspace-management` unchanged.
- **Risks**: (1) IME parity on macOS depends on GPUI's `InputHandler`
  bridge — validated by an early spike before the port proceeds;
  (2) first build pulls the GPUI dependency tree (long compile; pinned
  `gpui-pre =0.3.11` release); (3) visual output will be near-identical,
  not pixel-identical (font metrics differ between egui and GPUI).
