# Proposal

## Why

We currently have a single-file egui + portable-pty + vt100 demo that proves the
basic plumbing works: spawn a bash, render its ANSI output in egui, send
keystrokes back. That demo is throwaway. The real goal is a GUI shell around
the `pi` agent: a sidebar of sessions, a main pane rendering whichever session
is selected, and a clean abstraction so the pty backend can be swapped later.

## What Changes

- Remove the existing single-file demo in `src/main.rs`.
- Restructure the crate into modules:
  - `src/main.rs` — eframe entry point.
  - `src/app.rs` — `TerminalApp` holding the agent list and implementing
    `eframe::App`.
  - `src/agent_entities/` — `AgentEntity` trait + `AgentEvent` enum + the
    `PiAgentPty` implementation (portable-pty + vt100 + `pi` subprocess).
  - `src/ui/` — `sidebar.rs` and `pane.rs` rendering helpers.
- Introduce a sidebar UI with a `+` button. Clicking `+` creates a new
  `PiAgentPty` (default command: bare `pi`), names it `pi-N`, and selects it
  as the active session in the main pane.
- Main pane renders the active agent's vt100 screen, including a static
  block cursor and a border / status bar indicating which agent is active.
- Sidebar item shows name, status badge (starting / running / exited), and a
  hover-close button.
- Keyboard input is forwarded to the active agent only (printable chars,
  Enter, Backspace, Tab, arrow keys). Ctrl-combos and mouse are out of scope.
- Closing an agent (user clicks x) hard-kills the subprocess and removes it
  from the sidebar; if it was active, the main pane switches to the next
  agent or to an empty placeholder.
- If an agent exits on its own (EOF from pty), its badge flips to "exited"
  but the last screen is preserved; user must click x to remove it.
- Terminal size is hard-coded to 100 cols x 30 rows for this change; resize
  synchronization is tracked in `Docs/pending-decisions.md` for later.
- No persistence: closing the GUI kills all agents.

## Capabilities

### New Capabilities

- `agent-shell`: The desktop GUI shell around `pi` agent sessions — sidebar
  management of multiple agents, main-pane rendering of the active session,
  input forwarding, and session lifecycle (create / select / close / observe
  exit).

### Modified Capabilities

(None — the project has no existing specs.)

## Impact

- **Code**: `src/main.rs` is rewritten; new modules under `src/agent_entities/`
  and `src/ui/`. The demo logic moves into `PiAgentPty`.
- **Dependencies**: No new crates required; `egui`, `eframe`, `portable-pty`,
  `vt100` are already declared.
- **Runtime behavior**: The GUI now spawns `pi` (not bash) per session. The
  `pi` CLI must be on `PATH` for sessions to start; otherwise sessions will
  show an "exited" badge immediately.
- **Docs**: `Docs/pending-decisions.md` is the running TODO list for deferred
  concerns (multi-IO, resize, graceful shutdown, Ctrl-combos).
