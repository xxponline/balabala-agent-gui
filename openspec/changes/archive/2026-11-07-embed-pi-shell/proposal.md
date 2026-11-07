# Proposal

## Why

`pi-shell-rs` was written specifically to be embedded into this tool: it drives
`pi --mode rpc` (headless core) and renders the conversation itself, and it will
receive many targeted modifications for our GUI. Today the GUI instead spawns
pi's native TUI behind a PTY and replays its bytes through a `vt100` parser
(`PiAgentPty`), which keeps the TUI as a black box — no structured state, an
extra process layer, and no seam for the targeted modifications that motivate
`pi-shell-rs` in the first place. Embedding the TUI in-process is the
prerequisite for all of that future work, so the architecture decision should
be made now, before modifications accumulate on top of the PTY path.

## What Changes

- **In-process embedding replaces the PTY.** The GUI hosts `pi-shell-rs`'s app
  loop on a thread inside its own process; the only subprocess left is
  `pi --mode rpc` over stdio. `portable-pty` and the byte-pipe input path are
  removed from the agent-session path.
- **Rendering path stays cell-based (route α).** The embedded app draws through
  a new `VirtualBackend` that emits ANSI into memory; those bytes feed the
  existing `vt100::Parser`, and `pane.rs`'s cell renderer is untouched. The VT100
  layer is understood as the *transport* for the ratatui projection, not as part
  of pi-shell-rs's rendering.
- **`pi-shell-rs` becomes an embeddable library** (changes land in that repo,
  which is consumed via local path dependency): `app` module moves into the lib,
  the crossterm keyboard-poll thread is replaced by external key injection, and
  the three direct real-terminal touchpoints (`TerminalGuard` raw/alt-screen,
  `SetTitle`, OSC-52 clipboard write) become injectable sinks — title and
  clipboard map to real GUI facilities (an upgrade over the PTY path, where
  OSC-52 was dead output).
- **`AgentEntity` trait evolves.** `screen() -> &vt100::Screen` stays as-is;
  `send_input(&[u8])` becomes structured (`send_key` / `send_text`) because GPUI
  already produces structured key events and IME commits — encoding them to
  bytes just to decode them again is a detour.
- **Structured state becomes readable by the shell.** A read-only handle to the
  embedded `SessionState` lets the sidebar/status bar eventually show token
  usage, in-flight tool activity, or model — information a PTY black box can
  never provide. (UI for this is not committed here; the capability is unlocked,
  not specified.)
- **Deferred — second, GPUI-native renderer.** A future rendering scheme will
  bypass VT100 and draw directly with GPUI; both renderers must eventually be
  supported. Nothing is built for it now. The only insurance required now is a
  discipline: `view::render` must remain a pure function of
  `(SessionState, ViewUi)` — new state goes into those two structures, never
  read hiddenly from `App` — so a second renderer is a sibling function, not a
  view-layer rewrite.

## Capabilities

### New Capabilities

<!-- none — embedding is an implementation mechanism; user-visible behavior
     stays within agent-shell -->

### Modified Capabilities

- `agent-shell`: requirement wording moves off the PTY/subprocess mechanism onto
  the embedded RPC session — session spawn (no longer "spawn the `pi`
  subprocess" directly), keyboard input forwarding (structured key/IME events
  into the embedded app instead of raw UTF-8 bytes to subprocess stdin),
  terminal output rendering (produced by the embedded TUI rather than "colors
  emitted by the subprocess"), manual close (graceful embed shutdown + RPC child
  termination instead of hard-killing the PTY child), and observed exit (RPC
  channel closed instead of stdout EOF). The user-visible behaviors — grid
  rendering with colors and block cursor, which keys reach the session, sidebar
  lifecycle states, one-step focus/IME claim — are intended to remain intact.

## Impact

- **`balabala-agent-gui`**: `src/agent_entities/` gains a `PiAgentShell`
  implementation of `AgentEntity` replacing `PiAgentPty`; `src/app.rs` spawn
  site changes; `src/ui/pane.rs` unchanged; `Cargo.toml` already depends on
  `pi-shell-rs` via local path (`ref/pi-shell-rs`).
- **`pi-shell-rs` (separate repo, path-dep)**: lib-surface refactor — `app`
  module published, event-source injection, `VirtualBackend`, generic
  `draw(...)`, terminal sinks. The standalone TUI binary keeps working as a
  development harness over the same embeddable core.
- **Specs**: delta for `agent-shell` (mechanism rewording listed above).
- **Not affected**: `workspace-management` (workspaces, persistence, sidebar
  structure are orthogonal to how pane content is produced).
