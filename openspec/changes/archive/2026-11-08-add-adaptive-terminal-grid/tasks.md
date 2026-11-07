# Tasks

## 1. Measurement spike

- [x] 1.1 Confirm in a running app that the IME canvas paint closure
      receives the grid area bounds (log them, resize the window, verify they
      track the pane and exclude sidebar/status bar/padding) — verify by
      observing logged bounds change on window resize
- [x] 1.2 Confirm GPUI text-system metrics for Menlo 13px (cell advance +
      line height) are obtainable during paint and stable within a frame —
      verify by logging `cols = floor(w/cell_w)`/`rows = floor(h/line_h)`
      values that are plausible and stable across repaints of an unchanged
      window

## 2. Resize plumbing

- [x] 2.1 Add a resize path on `PiAgentShell` (`src/agent_entities/
      pi_agent_shell.rs`) that calls `embed.resize(cols, rows)` and
      `parser.set_size(rows, cols)` together and records the current
      dimensions, skipping sessions in `Exited` state — verify with a unit
      test: a running session updates its stored dimensions, an exited
      session's screen contents are byte-identical before/after
- [x] 2.2 Replace the fixed `COLS`/`ROWS` constants as source of truth with
      current-dimension state (constants remain only as pre-first-layout
      fallback) — verify `cargo build` and that a fresh session still starts
      at the fallback size before first paint

## 3. Pane integration

- [x] 3.1 In `src/app.rs`, read grid bounds from the IME canvas paint
      closure, compute (cols, rows) with floor division and minimum clamp
      (≥ 20×5), and dispatch resize to all `Running`/`Starting` sessions only
      when dimensions differ from their stored ones — verify by dragging the
      window larger and observing (logged) resize calls fire once per
      distinct size, none for unchanged repaints
- [x] 3.2 Verify the dedupe/no-op scenario from the spec: force repeated
      repaints without layout change (e.g. type in the session) and confirm
      no resize is propagated — verify by log output showing zero resize
      calls during input-only repaints

## 4. Verification

- [x] 4.1 Run `cargo test` — existing suites plus the new resize unit tests
      pass
- [x] 4.2 Manual pass against spec scenarios: resize window with a running
      session (grid columns/rows change, pi TUI re-lays out); resize while an
      exited session is shown (last frame unchanged); shrink pane to near
      zero (grid clamps to minimum, no panic/invalid size); background
      running session reflows when its pane is later shown
