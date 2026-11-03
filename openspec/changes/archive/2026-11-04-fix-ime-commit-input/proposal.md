# Proposal

## Why

Typing Chinese (or any IME-composed language) into an active `pi` session
currently produces the raw latin keystrokes used to compose the candidate
(e.g. `ni hao`) instead of the committed text (`你好`). Root cause: egui-winit
only opens the macOS IME pipeline when a widget claims focus and publishes
`PlatformOutput::ime` — which only happens for a focused `TextEdit` today.
Our pane is just a pile of `Label`s, so IME events never reach the app at
all, and the raw latin keystrokes leak through `Event::Key` into the pty.

## What Changes

- Introduce a new reusable widget `InteractiveFrame` in `src/ui/common/`.
  It wraps `egui::Frame`, adds IME focus handling, and exposes an
  `on_commit: &mut dyn FnMut(&str)` callback for IME-committed strings.
- Replace the hard-coded `egui::Frame::new()` in `src/ui/pane.rs` (line 36)
  with `InteractiveFrame`.
- Stroke color now depends on selection state: active panel uses the
  existing blue `(75, 161, 215)`, inactive uses grey `(60, 60, 60)`.
- Introduce a `PaneView` trait in `src/ui/pane.rs` so pane rendering depends
  on a read-only view rather than the concrete `AgentEntity` type.
  `AgentEntity` itself is expanded to become a `PaneView`.
- Two-step focus model: clicking a sidebar agent row switches selection;
  clicking the pane area itself is what claims keyboard focus (and opens
  IME). Focus state is owned by egui's `memory.focused_widget`, not by us.
- Remove the `Event::Ime(ImeEvent::Commit(s))` match arm previously added
  in `app.rs::forward_keyboard` — IME handling is now owned entirely by
  `InteractiveFrame`.
- Append two entries to `Docs/pending-decisions.md`:
  - IME preedit preview in terminal (still deferred).
  - One-step focus: clicking a sidebar agent row should request pane focus
    directly, eliminating the second click.

## Capabilities

### New Capabilities
<!-- None -->

### Modified Capabilities

- `agent-shell`: The keyboard input forwarding requirement gains a clause
  requiring IME-committed strings (`Event::Ime(ImeEvent::Commit(s))`) to be
  forwarded as UTF-8 bytes to the active session, and a new requirement
  stating that the active pane claims keyboard focus (and therefore IME
  ownership) only when the user clicks the pane area.
