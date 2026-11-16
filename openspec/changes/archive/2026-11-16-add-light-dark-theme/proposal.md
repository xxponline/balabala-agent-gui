# Proposal

## Why

The app is hard-locked to a light theme (design D9 of `port-ui-to-gpui-kit` forces
`ThemeMode::Light` and nothing follows system appearance), so users running a dark
system get a blinding white shell; worse, the lock is only skin-deep — 35 hardcoded
`rgb(0x..)` chrome literals and a light-only ANSI 16-color palette bypass
gpui-component's dual-theme infrastructure entirely, so no dark mode exists today.
gpui-component 0.9.0 already ships light/dark palettes, semantic tokens, and
system-appearance observation that components consume automatically — this change
wires the app into that infrastructure instead of maintaining colors by hand.

## What Changes

- **Follow system appearance**: remove the forced `ThemeMode::Light` in `main.rs`;
  sync the theme with the OS appearance at startup and subscribe to window
  appearance changes so light/dark switches apply live (decision D1: no manual
  toggle, no persistence).
- **Theme-aware VT100 color mapping** (`src/ui/mod.rs`): split the ANSI 0-15
  `PALETTE` into light/dark tables, make default fg/bg follow the active theme,
  and thread a theme parameter through the pure `fg_to_gpui`/`bg_to_gpui`
  functions. The xterm 256-color cube/grayscale conversion is standard and shared.
- **Chrome colors → semantic tokens**: replace all 35 hardcoded `rgb(0x..)`
  literals across sidebar, tree, pane, widgets, and viewer panels with
  gpui-component semantic tokens (`cx.theme()`: `background`, `border`,
  `list_hover`, `list_active`, `muted_foreground`, `accent`, `danger`,
  `warning`, `success`, …) — decision D3: full-token, zero literal exceptions
  (decision record in design.md).
- **pi-shell-rs theme injection wiring (dependent)**: once the parallel
  pi-shell-rs work lands its `TuiTheme` injection API (documented in
  `../pi-shell-rs/Docs/theme-injection-requirements.md`), spawn sessions with a
  theme derived from the active mode and call `set_theme` on live sessions when
  appearance changes. Tasks for this are included but marked blocked on that API.
- Supersedes design D9 (archived change `port-ui-to-gpui-kit`); the archive
  itself is not edited.

## Capabilities

### New Capabilities

- `theming`: application-level light/dark theme behavior — following OS
  appearance, theme-consistent chrome surfaces, and theme-aware terminal color
  mapping (default colors + ANSI palette) while preserving session-emitted
  colors.

### Modified Capabilities

<!-- none: existing requirements (agent-shell "preserve colors" / "active color
     (blue)", center-tabs highlighting) hold unchanged in both themes -->

## Impact

- **Code**: `src/main.rs` (theme bootstrap + appearance subscription),
  `src/ui/mod.rs` (dual palette, themed fg/bg mapping), `src/pane/mod.rs`
  (status bar, placeholder, separator, pane frame, empty state),
  `src/sidebar/mod.rs` (hover/active/borders/badges), `src/tree/mod.rs`,
  `src/ui/widgets.rs`, `src/property_detail_manager/{md_panel,editor_panel}.rs`.
- **Tests**: pane color assertions (`fg_to_gpui`/`bg_to_gpui` signature change,
  `src/pane/mod.rs:626-645`); headless renders that assume white backgrounds.
- **Dependencies**: none added (gpui-component theme infra already present).
  External dependency on pi-shell-rs `TuiTheme` API (parallel repo, requirements
  doc delivered); `Cargo.lock` currently at `6bd2debb`, unpinned by design (D4).
- **Known intermediate state**: until the pi-shell-rs API lands and its injection
  tasks run, session-emitted content (tool-card panels, syntax highlighting)
  keeps dark-designed colors under light theme — already tracked as the
  pi-shell-rs side of this effort.
