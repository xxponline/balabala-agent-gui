# Tasks

## 1. Cell-width metric spike

- [x] 1.1 Verify how to obtain the Menlo 13px advance width from the GPUI
      text system (try `typographic_bounds` / `layout_line` for `'M'`; if an
      explicit font-load step is required, record it), and confirm the value
      is stable across two calls in one frame — verify by logging the
      measured width in a running app session and checking it is a sane
      monospace advance (~7–8px at 13px)

## 2. Fixed-width renderer

- [x] 2.1 Extend `terminal_grid()` in `src/ui/pane.rs` so color-run segments
      carry a column count and are sized `columns × cell_width` px, skipping
      `is_wide_continuation()` cells and counting `is_wide()` cells as 2 —
      verify with `cargo build` and a unit test that renders a fixture screen
      containing `> 帮 me` and asserts each row div's summed width equals
      `columns × cell_width`
- [x] 2.2 Handle width-aware cursor inversion (block spans 2 cells on a wide
      leading cell, nothing on the continuation cell) and keep empty cells as
      width-1 nbsp boxes so rows never collapse — verify with a unit test
      placing the cursor on a CJK character and asserting the inverted span
      width is `2 × cell_width`
- [x] 2.3 Document the fixed-width contract in the `terminal_grid()`
      doc comment (segment width = columns × cell_width, shared metric
      source) — verify the comment exists in `src/ui/pane.rs` after the edit

## 3. Verification

- [x] 3.1 Run the existing test suite (`cargo test`) and confirm no
      regressions in `tests/embed_layout.rs` or `src/ui` tests
- [x] 3.2 Manual check in the running app: a session output containing mixed
      ASCII/CJK lines and an emoji — row right edges line up, no glyph
      clipping beyond terminal-typical overflow, colors and cursor block
      unchanged (compare against spec scenarios: ANSI colors preserved,
      cursor visible, mixed row alignment, continuation not painted)
