# Design

## Context

A ~600-line egui immediate-mode UI (window bootstrap, sidebar, terminal
pane, keyboard forwarding, IME focus glue) sits over a framework-agnostic
backend: `src/agent_entities/` (portable-pty + vt100 + `pi` subprocess
behind the `AgentEntity` trait) and `src/workspace/` (workspace model +
`.workspaces.json` store). All current behavior is covered by the
`agent-shell` and `workspace-management` specs. See proposal.md — Why for
motivation (reactive model for future long-text workbench views, and
maintainability: the current UI carries an `unsafe` screen-pointer
workaround in `app.rs` and hand-rolled egui IME plumbing in
`InteractiveFrame`).

Constraints that shape the approach: macOS-first (specs are macOS-centric);
the terminal stays a fixed 100x30 vt100 grid (scrollback, resize sync, and
Ctrl-combo forwarding remain out of scope per `Docs/pending-decisions.md`);
behavior parity with existing specs is the acceptance bar except for the
one approved behavior change (one-step focus).

## Goals / Non-Goals

**Goals:**

- Behavior-parity port of `agent-shell` + `workspace-management` to
  gpui-kit 0.9.0, validated against the existing spec scenarios.
- Event-driven update model: PTY output wakes the UI (`notify`) instead of
  unconditional 50 ms full-frame repaints.
- A `Pane` seam: the shell's main area hosts pane kinds (today: agent
  terminal) so future tools (log analysis, code editing, UI design) plug
  in as new kinds without reworking shell chrome.
- One-step focus: sidebar selection grants keyboard + IME focus to the
  pane (the single deliberate behavior change; see specs delta).
- Complete removal of `egui` / `eframe` and their glue code.

**Non-Goals:**

- Workbench shell features: dock/tabs/drag-drop, multiple simultaneous
  panes, pane persistence (future change on the `Pane` seam).
- New pane kinds (log views, editors, design canvases).
- Terminal core changes: scrollback, vt100 replacement, window-resize
  sync, Ctrl-combo keys (tracked in `Docs/pending-decisions.md`).
- Visual redesign: near-identical look is the goal; gpui-component
  restyling is a later change.
- Replacing `rfd`; the blocking folder-picker wart is unchanged.

## Decisions

### D1. Single UI dependency: `gpui-kit = "0.9.0"` (default features)

`gpui_kit::*` re-exports GPUI (published as `gpui-pre =0.3.11`) plus
gpui-base (input/editor, virtual_list, dock, text stack) and gpui-component
(widgets, tree-sitter features). Using the umbrella crate keeps the whole
layer version-locked. Alternative: depend on raw `gpui` — rejected: we
would re-derive the text/scroll/component infrastructure that the
long-text roadmap needs anyway.

### D2. Sessions as gpui entities; the unsafe pointer hack dies

The shell is `Entity<TerminalApp>`; each session becomes
`Entity<AgentSession>` where `AgentSession` is a thin newtype over
`Box<dyn AgentEntity>` (kept as a trait object so the planned mock/remote
entities still fit). `Workspace.agents` changes from
`Vec<Box<dyn AgentEntity>>` to `Vec<Entity<AgentSession>>` — a type-level
touch to `src/workspace/mod.rs`; the `AgentEntity` trait, `PiAgentPty`,
and the store are behaviorally untouched. Render reads sessions via
`read(cx)`; mutations (create/close/send_input) go through
`update(cx, ...)` from event callbacks. This replaces the
`unsafe { &*screen_ptr }` borrow workaround with safe handles.
Alternative: one monolithic `TerminalApp` entity with a timer — simpler,
but re-renders the whole shell per wake and diverges from the per-pane
entity model the workbench roadmap needs.

### D3. Update model: throttled drain + notify-on-output

One background task (spawned by the shell) ticks every 50 ms — the current
repaint cadence, so perceived latency is unchanged — and calls `poll()` on
each session entity; the session calls `cx.notify()` only when the drain
consumed events. To make "consumed events" observable, `AgentEntity::poll`
changes signature from `fn poll(&mut self)` to `fn poll(&mut self) -> bool`
(a contract-compatible tweak to the trait; implementations already know
whether the channel yielded). The drain is **budget-capped per tick**
(`POLL_MAX_EVENTS` / `POLL_MAX_BYTES` in `pi_agent_pty.rs`): during
implementation a greedy `try_recv` loop was found to never empty when the
reader thread outpaced the vt100 parser on an unbounded channel, freezing
the 50 ms tick — excess is left for the next tick instead. Backpressure /
bounded channels remain future work per `Docs/pending-decisions.md`.

**General rule (found the hard way):** event dispatch holds the App's
mutable borrow for the whole listener, so a listener that blocks longer
than one tick (modal dialogs, sync IO) lets the drain loop's
`weak.update` re-enter the borrow and panic. **No blocking calls inside
event listeners** — see D8 for the concrete case.
Alternative: wake per reader-thread message —
rejected for now: output bursts (`cat bigfile`) would storm notifies; a
50 ms throttle matches today's behavior exactly and can be tightened later.

### D4. IME through GPUI's `InputHandler` bridge

The focused pane installs a `PlatformInputHandler`
(`selected_text_range / marked_text_range / text_for_range /
replace_text_in_range / unmark_text` — the NSTextInputClient bridge):
committed text arrives via `replace_text_in_range` and is forwarded to the
session as raw UTF-8 bytes. `marked_text_range` returns `None` — no preedit
display (spec-excluded). Selection installs the handler and focuses the
pane's `FocusHandle` in the same step (one-step focus). This replaces both
`InteractiveFrame`'s `PlatformOutput::ime` dance and the `Event::Ime`
match arm. A phase-0 spike validates Chinese commit reaches the pty before
any port work starts. Alternative: none viable — egui's IME approach has
no counterpart outside egui-winit.

### D5. Keyboard mapping on the focused pane

Verified end-to-end in the spike (evidence in `spike-notes.md`); three
mechanisms, all single-path:

- **Key path**: `on_key_down` on the pane's focus handle maps keystrokes
  to the same byte table as today: Enter `\r`, Backspace `\x7f`, Tab `\t`,
  arrows `\x1b[A`–`\x1b[D`; printable characters forward
  `keystroke.key_char` bytes (this GPUI version has no `ime_key`; composed
  characters arrive in `key_char`). **Every forwarded event calls
  `cx.stop_propagation()`** — without it the macOS backend's key-event-first
  branch also hands the key to the input context (`insertText`) and
  printable characters double-send.
- **IME path**: commits arrive via `replace_text_in_range` only (under an
  IME input source the backend routes printable keys to the input context
  first — single path); `replace_and_mark_text_in_range` (preedit) is
  never forwarded.
- **Tab (and any globally-bound key)**: gpui-base's Root binds `tab` → a
  focus-traversal action at context `Root`, and a matched binding
  suppresses the raw key event — Tab never reaches `on_key_down`. The pane
  must declare `.key_context("Terminal")` and register its own `tab`
  binding (deeper context wins in `resolve_bindings_for_input`) whose
  action handler forwards `\t`.
- Modifier filter: ctrl/cmd/fn combos are NOT forwarded and do NOT stop
  propagation, so platform shortcuts and the IME menu keep working.
  Ctrl-combos stay unforwarded (spec).

### D6. Terminal grid: per-row styled segments, not custom text layout

Keep the current segment-merging algorithm: each vt100 row merges runs of
equal (fg, bg) into segments; segments render as styled text on `div()`
backgrounds; cursor cell renders with swapped fg/bg. Monospace font (13 px
scale) is chosen explicitly and its cell metrics measured once via the
gpui text system. Alternative: Zed-style `TextLayout` custom painting —
more power (per-cell bg, scrollback painting) but unnecessary for a fixed
100x30 grid; revisit when scrollback/resize lands.

### D7. Sidebar via plain gpui elements with callbacks

Sidebar rows/buttons are plain elements; each control carries a callback
closure that dispatches into `TerminalApp` methods via the shell entity.
This retires the `SidebarAction` struct pattern. gpui-component buttons
may be used where they match the current look, but visual parity is
"near-identical", not pixel-identical (font metrics differ).
Alternative: keep an action-struct + dispatch loop — it maps awkwardly
onto retained-mode callbacks; a command/event-bus refactor is already
deferred in `Docs/pending-decisions.md` and would layer on top later.

### D8. Folder picking: rfd **async** (sync crashes under gpui)

`rfd::AsyncFileDialog::pick_folder()` driven by `cx.spawn`, with the
task stored on the shell (`_dialog_task`; starting a new dialog replaces
and cancels the previous one). The workspace-management spec's behavior
is unchanged: cancel keeps state, canonicalize + dedup, immediate
persist.

The synchronous `rfd::FileDialog` is unusable under gpui: its modal loop
blocks the main thread while the click listener still holds the App's
mutable borrow, so the 50 ms drain loop's `weak.update` re-enters the
borrow and panics (`RefCell already borrowed`, observed live — this is
the concrete case of D3's no-blocking rule). The async form lets the main
thread keep running the drain loop while the dialog is open. gpui's
native `prompt_for_paths` remains a possible future replacement; not
needed now.

### D9. Forced light theme

The shell renders in gpui-component's light theme unconditionally:
`gpui_kit::init` loads `ThemeMode::Light` and nothing in the stack calls
`Theme::sync_system_appearance`, so the theme never follows the system;
`main.rs` additionally calls `Theme::change(ThemeMode::Light)` to state
the intent durably. All hardcoded colors (hover, borders, active
highlight, dividers — and the terminal defaults planned under D6) are
chosen for light backgrounds; the dark values from the egui era are
invalid here (dark hover + default dark text was invisible, reported in
user testing).

## Risks / Trade-offs

- [IME parity gap on macOS via `InputHandler`] → Phase-0 spike is a hard
  gate: a minimal gpui-kit window whose pane forwards a Chinese IME commit
  into a pty. The port proceeds only on success; on failure we stop and
  reconsider (this is the single risk that could invalidate the plan).
- [Double-send between key events and IME commits] → The spike exercises
  both plain typing and IME commit and asserts single-path delivery; the
  spec scenario "Sidebar selection grants keyboard and IME focus in one
  step" is the standing regression check.
- [Pinned pre-release `gpui-pre =0.3.11`] → Version moves only with
  gpui-kit releases; acceptable for an internal tool. Mitigation: keep
  `Cargo.lock` committed (already is).
- [First build cost: GPUI dependency tree, long compile, larger binary] →
  One-time cost; local `target/` caches absorb it. No CI exists today.
- [Font metrics differ from egui] → Visual output is near-identical, not
  pixel-identical; accepted in proposal. Cursor block and cell alignment
  verified manually against the 100x30 grid.
- [50 ms drain throttle keeps the old latency floor] → Matches today's
  perceived behavior; tightening is a later tuning knob (D3), not a design
  change.

## Migration Plan

No coexistence is possible: egui (eframe) and gpui both own the window and
event loop, so the entry point swaps wholesale. Sequencing minimizes the
blind window:

1. **Phase 0 — IME spike** (throwaway): gpui-kit window + `InputHandler`
   → Chinese commit bytes into a `portable-pty` child. Gate for everything
   below.
2. **Phase 1 — Shell skeleton**: `main.rs` bootstrap (window 1000x600,
   title parity), `TerminalApp` entity, sidebar with workspace/session
   CRUD and persistence, one-step selection+focus wiring. Old egui files
   remain in tree until phase 3 so behavior can be diffed by running the
   old binary from git history.
3. **Phase 2 — Terminal pane**: grid rendering (D6), keyboard mapping
   (D5), IME (D4), starting-placeholder and status bar parity, session
   lifecycle (close/exit badges) parity.
4. **Phase 3 — Removal**: delete `src/ui/common/interactive_frame.rs` and
   all egui glue; drop `egui`/`eframe` from `Cargo.toml`; rewrite the
   spec's egui vocabulary is already captured in the specs delta.

Acceptance = walk every `agent-shell` / `workspace-management` scenario as
a manual checklist on the new binary (plus the two new/changed focus
scenarios). Rollback at any phase is a git revert; the backend modules are
untouched, so risk is bounded to the UI layer.

## Open Questions

None blocking. Deferred by design: `prompt_for_paths` adoption (D8),
gpui-component restyle, tree-sitter feature enablement — all future work,
tracked in `Docs/pending-decisions.md` where they overlap.
