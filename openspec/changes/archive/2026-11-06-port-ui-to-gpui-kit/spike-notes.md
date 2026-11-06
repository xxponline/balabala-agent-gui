# IME Spike Notes (task 1.1 → 4.3 reference)

Spike code: `src/bin/ime_spike.rs` (throwaway; delete after task 4.3
verification). Self-test: `cargo run --bin ime_spike -- --self-test`.

## Verified programmatically (2026-11-06, self-test: 13/13 PASS)

- **Mapping table** (`keystroke_to_bytes`): `enter`→`\r` (even when
  `key_char` is `Some("\n")` — named key wins), `backspace`→`\x7f`,
  `tab`→`\t`, `up/down/right/left`→`\x1b[A`–`\x1b[D`, printable via
  `keystroke.key_char` first then single-char `key` fallback, `space`→` `,
  unknown named keys (e.g. `cmd`) not forwarded.
- **IME commit bytes**: `replace_text_in_range(text)` → raw UTF-8
  (`你好` → 6 bytes).
- **PTY plumbing**: both byte paths round-trip through a real
  `portable-pty` child (`/bin/cat`) — bytes written via
  `master.take_writer()` are readable back via `master.try_clone_reader()`.

## API surface confirmed (gpui-pre 0.3.11, source in registry)

- IME mount point: `Window::handle_input(&FocusHandle, impl InputHandler,
  &App)` — **must be called during an element's paint phase**; it
  self-checks `focus_handle.is_focused(window)` and registers the handler
  for the upcoming frame. Spike uses `canvas(prepaint, paint)` to reach the
  paint phase from a plain `div()` tree, wrapping
  `ElementInputHandler::new(bounds, entity)`.
- View-side trait: `gpui::EntityInputHandler` (required:
  `text_for_range`, `selected_text_range`, `marked_text_range`,
  `unmark_text`, `replace_text_in_range`, `replace_and_mark_text_in_range`,
  `bounds_for_range`, `selection_bounds_for_range`,
  `character_index_for_point`, `set_selected_text_range`,
  `text_length_utf16`; defaults: `paste`, `accepts_text_input`,
  `text_input_configuration`, `text_input_editable_range`).
- `InputHandler::prefers_ime_for_printable_keys()` default `false`;
  framework doc: *"The terminal keeps the default false so that raw keys
  reach the terminal process."* — i.e. printable keys are expected on the
  **key-event path**, IME commits on the **`replace_text_in_range` path**.
- `Keystroke { modifiers, key: String, key_char: Option<String> }` — there
  is no `ime_key` field in this version; composed characters arrive in
  `key_char`.
- Focus: `cx.focus_handle()`, `div().track_focus(&handle)`,
  `FocusHandle::is_focused(window)`, `FocusHandle::focus(window, cx)`.
- Key events: `div().on_key_down(cx.listener(|this, event, window, cx| …))`.
- Background loop: `cx.spawn(async move |this: WeakEntity<T>, cx| { loop {
  cx.background_executor().timer(50ms).await; this.update(cx, …) } })` —
  the returned `Task` MUST be stored (dropping it cancels the loop).

## Platform-level status (the gate)

- Automated keyboard driving is impossible on this machine: `osascript`
  System Events is TCC-denied (`-1743 Not authorized to send Apple
  events`); gpui's test harness has no IME-commit simulation
  (only `dispatch_keystroke`).
- **Pending manual verification at the open spike window** (see
  task 1.1): plain typing "hello" must produce exactly one `key_down`
  forward per character and zero `ime_commit` lines; composing 你好 must
  produce exactly one `ime_commit` line and zero `key_down` lines for the
  committed text; `ime_preedit` lines must show but never forward.
- The spike prints every forward to stdout/stderr as
  `[ime-spike] <source> -> N byte(s): "<text>"` — that log is the evidence
  artifact for task 4.3's double-send check.

## Platform routing rules (gpui-pre-macos 0.3.11 `handle_key_event`, read 2026-11-06)

For a key-down the backend picks a branch:

- **Branch 1 (IME-first)** when: composing (marked text) OR
  `is_ime_printable_key` (printable key_char, no ctrl/fn/cmd, current input
  source is a non-ASCII IME mode, and
  `handler.prefers_ime_for_printable_keys()`) OR `key_char.is_none()` with
  no ctrl/fn/cmd. It calls `inputContext.handleEvent:` first (→
  setMarkedText/insertText/doCommandBySelector); only if that returns NO
  does it fall through to dispatching the key event.
- **Branch 2 (key-event-first)** otherwise: dispatches the key event
  first; if the app does not handle it, THEN calls `handleEvent:` (→
  insertText). **This is the double-send path**: printable chars under an
  ASCII input source fired BOTH `key_down` AND `ime_commit` per character.

Fix adopted: in the key path, forward printable/control-key bytes and call
`cx.stop_propagation()` (event handlers propagate by default; stopping
marks the event handled so the backend skips the trailing `handleEvent:`
and no insertText fires). Combos with ctrl/cmd/fn are NOT forwarded and do
NOT stop propagation, so platform shortcuts and the IME menu keep working.

Observed on this machine (first interactive run, before the fix):

- ASCII input source: `key_down "s"` + `ime_commit "s"` per character
  (double-send, branch 2).
- Chinese IME: single-path commits (`ime_commit "你"` exactly once,
  preedit lines shown and never forwarded) — branch 1.

Load-bearing quirk: `ElementInputHandler::prefers_ime_for_printable_keys`
forwards to `EntityInputHandler::accepts_text_input` (upstream quirk), and
our `accepts_text_input` defaults to `true`. That keeps
`query_prefers_ime_for_printable_keys()` true so Chinese-IME printable
keys take branch 1 (single path). Do NOT "fix" this by returning false
from `accepts_text_input` — it would push IME keys into branch 2 and
reintroduce the double-send under the Chinese IME.

## Final verification (2026-11-06, interactive run: GATE PASSED)

Evidence (event log, both runs combined):

- IME commit exactly once per commit: `ime_commit -> 6 byte(s): "你好"`
  (twice, one per user test), `ime_commit -> 5 byte(s): "hello"` (IME
  passthrough of ASCII under Pinyin).
- Preedit never forwarded: `ime_preedit (NOT forwarded): "ni" / "ni hao" /
  "hello"` lines with zero bytes sent.
- Plain typing single-path after the stop_propagation fix: only
  `key_down` lines, no paired `ime_commit` (the pre-fix log showed
  `key_down "s"` + `ime_commit "s"` per character under the ASCII input
  source — the branch-2 double-send).
- Tab via the Terminal-context binding: `tab_action -> 1 byte(s): "\t"`
  (3 presses), no key_down for tab, no focus-traversal side effect.
- Backspace `\x7f`, arrows `\x1b[A`–`\x1b[D`, Enter `\r`, space — all
  single-path `key_down` lines.
- Echo artifact resolved: child spawned as `sh -c 'stty -echo; exec cat'`
  so the display shows only the child's own output (text appears after
  Enter, once). Real `pi` runs raw-mode and never echoes.

Known edge (NOT spec'd, deferred — flag at task 4.3 if it matters):
**Enter-during-composition ordering.** Because the pane reports
`marked_text_range = None` (spec: no preedit display), the backend's
`is_composing` stays false, so Enter takes branch 2 and the `\r` reaches
the pty BEFORE the IME's (possibly async) commit of the composed text —
pty sees `\r` then `"hello"` instead of `"hello"` then `\r`. Potential
fix: track preedit internally (store the marked text/range from
`replace_and_mark_text_in_range`, return it from `marked_text_range`
without rendering it) so the backend routes composing keys through the
IME first. Not implemented: outcome depends on the IME consuming Enter,
and no spec scenario covers commit/Enter ordering.

## Implications for task 4.3

- Keep the same two-path architecture: key path
  (`keystroke_to_bytes` + `on_key_down`) and IME path
  (`replace_text_in_range` only). Never forward from
  `replace_and_mark_text_in_range` (preedit) — spec forbids it.
- **Three platform discoveries the pane must reproduce** (all verified
  in the spike):
  1. `cx.stop_propagation()` on every forwarded key event — without it
     the macOS backend's branch 2 delivers the key to the input context
     as well (`insertText`) and printable chars double-send.
  2. Tab (and any key bound by a global keymap) never reaches
     `on_key_down`: gpui-base's Root binds `tab` → focus-traversal `Tab`
     action at context `Root`. The pane must declare
     `.key_context("Terminal")`, register
     `cx.bind_keys([KeyBinding::new("tab", TerminalTab, Some("Terminal"))])`
     (deeper context wins in `resolve_bindings_for_input`), and forward
     `\t` from the action handler — single path, since a matched binding
     suppresses the raw key event.
  3. Modifier filter: ctrl/cmd/fn combos are not forwarded and must NOT
     stop propagation (platform shortcuts and the IME menu keep working).
- `bounds_for_range` returns the pane bounds so the IME candidate window
  sits near the terminal.
- `selected_text_range` returns a caret at 0 (no in-app selection);
  `marked_text_range` returns `None` (no preedit display).
