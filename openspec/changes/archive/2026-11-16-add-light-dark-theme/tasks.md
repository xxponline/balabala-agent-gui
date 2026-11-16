# Tasks

## 1. 外观接线 (W1: follow system appearance)

- [x] 1.1 Remove the forced `ThemeMode::Light` in `src/main.rs`; at window
  startup call `Theme::sync_system_appearance(Some(window), cx)` and subscribe
  `window.observe_window_appearance` → sync again, storing the `Subscription`
  on `TerminalApp` (design D1). Verify: `cargo check` passes; launching the
  app under a dark system appearance renders dark chrome (manual visual check)
  and toggling the OS appearance live re-renders both open windows without
  restart (spec: "Live switch while running").
- [x] 1.2 Add a regression check for the wiring: an automated test asserting
  the app does not force `ThemeMode::Light` after init (e.g. `Theme::global(cx)`
  mode equals `cx.window_appearance()` derived mode in a headless update).
  Verify: `cargo test` passes with the new assertion.

## 2. 双调色板 VT100 映射 (W2: theme-aware terminal colors)

- [x] 2.1 In `src/ui/mod.rs` split `PALETTE` into `PALETTE_LIGHT` (today's
  values unchanged) and `PALETTE_DARK` (dark-era values recovered from pre-port
  git history: default bg `0x1e1e1e`, default fg `0xe5e5e5`, green `0dbc79`,
  yellow `0xe5e510`, remaining entries filled to the same 16-entry structure);
  keep the shared 256-cube/grayscale conversion (design D2). Verify: unit
  test locks both tables' size and the shared conversion output for indices
  16-255; `cargo test` passes.
- [x] 2.2 Change `fg_to_gpui`/`bg_to_gpui` to take a theme argument (palette +
  default fg/bg snapshot) and thread it through `build_grid` →
  `terminal_grid` from the two render call sites (`cx.theme()`); update the
  existing color assertions in `src/pane/mod.rs` (~lines 626-645) to pass an
  explicit theme. Verify: `cargo test` passes, including a new test asserting
  `Color::Default` maps to dark values under the dark theme and light values
  under light, and that explicit `Color::Rgb` passes through byte-identical in
  both (spec: "Explicit session colors pass through").
- [x] 2.3 Headless render verification: render the terminal grid fixture under
  both themes and assert default cells match the pane background (no seam) and
  index-colored cells differ between themes. Verify: `cargo test` passes
  (spec: "Empty terminal area matches the pane", "Theme switch re-maps
  existing screen" at the mapping level).

## 3. Chrome 硬编码 → 语义 token (W3: full-token migration)

- [x] 3.1 Migrate `src/sidebar/mod.rs` (14 literals: hover → `list_hover`,
  active row → `list_active`, borders → `border`, muted text →
  `muted_foreground`, red close/exit → `danger`, badges → `warning`/`success`)
  using the design D3 mapping table. Verify: `cargo check` passes; grep
  confirms no `rgb(0x` literals remain in the file; sidebar tests pass.
- [x] 3.2 Migrate `src/tree/mod.rs` (6 literals) and `src/ui/widgets.rs`
  (2 literals) with the same table (tree hover link → `accent`). Verify:
  `cargo check` passes; grep confirms zero `rgb(0x` literals in both files;
  tree tests pass.
- [x] 3.3 Migrate `src/pane/mod.rs` chrome literals (8: empty state/status bar
  muted text → `muted_foreground`, blue border/dot → `accent`, separator →
  `border`, starting placeholder → `warning`, pane frame background →
  `background`; keep the translucent selection overlay `0x0078D959` per design
  D3) and the viewer panel backgrounds in
  `src/property_detail_manager/{md_panel,editor_panel}.rs` (→ `background`).
  Verify: `cargo check` passes; grep across `src/` reports zero `rgb(0x`
  literals outside `src/ui/mod.rs` (palette tables are intentional);
  `cargo test` passes.
- [x] 3.4 Document the token mapping and the Hsla→Rgba conversion point in
  the module docs of `src/ui/mod.rs` (where the palette/theme helpers live).
  Verify: module doc renders the design D3 table reference and `cargo doc`
  builds without warnings for the crate's own docs.

## 4. pi-shell-rs 注入接线 (W4: blocked on external API)

- [x] 4.1 **Blocked→unblocked**: `TuiTheme` / `EmbedConfig.theme` / `set_theme` must
  exist in the locked pi-shell-rs dependency (requirements at
  `../pi-shell-rs/Docs/theme-injection-requirements.md`). When available,
  `cargo update -p pi-shell-rs` and verify `cargo check` shows the new API.
- [x] 4.2 Once 4.1 is unblocked: build a `TuiTheme` from the active theme mode
  at session spawn (`EmbedConfig.theme`) and push `set_theme` to all live
  sessions on appearance change (design D4). Verify: with the light theme
  active, a spawned session renders light tool-card panels / scrollbar /
  syntax highlighting; toggling OS appearance updates running sessions
  without restart (spec: "Light theme receives light color set", "Theme
  change updates live sessions").
- [x] 4.3 Once 4.2 is unblocked: verify the uninjected default is unchanged —
  spawning with injection disabled (or pi-shell-rs default) reproduces today's
  dark-designed content byte-for-byte. Verify: pi-shell-rs default-path tests
  pass from the GUI side smoke run (spec: "Without injection … unchanged").

## 5. 集成验收

- [x] 5.1 Full suite + visual pass both appearances — **verified 2026-11-16 by user** (cargo run ×2, Dark+Light 双向; chrome + 控制台 W4 注入后均正常), cargo test 82/82, 0 warnings. run `cargo test`
  (whole workspace) and manually inspect sidebar, file tree, pane frame,
  status bar, viewers, and terminal defaults/ANSI colors in light and dark —
  confirm legibility and no single-theme-locked surface remains (spec:
  "Chrome surfaces are theme-consistent", "Index-colored text readable in dark
  theme"). Note any palette constants needing contrast touch-ups and adjust
  in `PALETTE_DARK` only.
- [x] 5.2 Verify spec scenarios end-to-end — **verified 2026-11-16 by user** ("测试没有问题"): 启动随系统外观、热切换、会话内容随主题更新均通过; 自动化覆盖: theme_follows_window_appearance_on_startup / grid_renders_under_both_themes_without_seam / theme_change_pushes_tui_theme_to_live_sessions / tui_theme_dark_is_pi_default_and_light_is_valid. startup
  under dark appearance, live toggle both directions, session screen
  re-mapping with a session running. Verify: checklist against
  `specs/theming/spec.md` scenarios all pass; record results in the change's
  task notes.

## Workflow follow-up

- Archive the change after the project's review requirements are satisfied.
- Verify the archived result.
