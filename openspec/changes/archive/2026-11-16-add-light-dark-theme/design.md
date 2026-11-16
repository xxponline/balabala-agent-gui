# Design

## Context

See proposal.md for motivation. Constraints discovered during exploration that
shape this design:

- gpui-component 0.9.0 ships the full dual-theme stack: `ThemeColor::light()/
  dark()` palettes, semantic tokens, `Theme::change(mode)` (which refreshes all
  windows), `Theme::sync_system_appearance`, and `window.observe_window_appearance`.
  Component layers (dock, tabs, list, `Editor`/`TextView` syntax highlighting)
  already read `cx.theme()` and switch correctly when the mode changes — verified
  in dependency source. Only this app's own hardcoded colors bypass it.
- The terminal color path is pure: `fg_to_gpui`/`bg_to_gpui` → `build_grid` →
  `terminal_grid`, rebuilt every render, so a theme switch automatically
  re-renders existing screens once mapping is theme-aware.
- Session content colors (tool-card panels, scrollbar, syntax fg) are produced
  by pi-shell-rs as explicit RGB, bypassing any palette. The fix is the
  parallel `TuiTheme` injection work in that repo (requirements delivered at
  `../pi-shell-rs/Docs/theme-injection-requirements.md`).
- Design D9 (archived `port-ui-to-gpui-kit`) mandated the forced light theme;
  this change supersedes it. The archive is historical and not edited.
- Existing specs hold in both themes: agent-shell "preserve colors" (explicit
  RGB passes through — mapping only affects defaults and indices) and "active
  color (blue)" (accent token is blue-family in both palettes), so no
  MODIFIED deltas are needed.

## Goals / Non-Goals

**Goals:**

- App follows OS appearance at startup and live, with zero manual theme UI.
- Every hardcoded chrome color replaced by a semantic token; adding a theme
  later means editing theme data, not call sites.
- VT100 default fg/bg and ANSI 0-15 palette theme-aware; 256-color conversion
  unchanged; pure functions stay pure (unit-testable without a window).
- Session-emitted dark-designed colors become theme-correct via pi-shell-rs
  injection (tasks included, blocked on that API).

**Non-Goals:**

- No manual theme toggle, no persisted theme preference (decision D1 —
  revisit only if users ask to override the system).
- No changes to pi agent (TS) or to pi-shell-rs's standalone default appearance.
- No edits to archived changes; D9 is superseded by this change, not rewritten.
- No new syntax-highlight theme authoring — light theme reuses syntect's
  bundled light theme selected by name through the injection API.
- No per-workspace or per-session theme; one app-wide theme.

## Decisions

### D1. Theme state source: follow system appearance only

**Decision**: Remove the forced `ThemeMode::Light`. On startup call
`Theme::sync_system_appearance`; subscribe via
`window.observe_window_appearance` → sync again on every change.

**Why**: gpui-component provides both halves already; system-follow is the only
variant requiring zero UI and zero persistence. Manual toggle and
"follow + override" were considered and deferred — they need a settings surface
and a three-state (system/light/dark) model; the subscription design leaves
room to add one later without reworking the pipeline.

**Detail**: the `Subscription` returned by `observe_window_appearance` must be
retained for the window's lifetime — store it on `TerminalApp` (the window's
root entity), not dropped at the end of the open-window closure.

### D2. VT100 mapping: theme parameter through pure functions

**Decision**: `fg_to_gpui(color, theme)` / `bg_to_gpui(color, theme)` take a
small `TerminalTheme` (or `&Theme`-derived snapshot) carrying the two palettes
and default fg/bg. `build_grid` and `terminal_grid` thread it from the render
context.

**Why over reading a global inside the mapper**: keeps the functions pure —
existing unit tests (`src/pane/mod.rs:626-645`) stay deterministic by passing
an explicit theme, and no hidden App dependency enters the hot render path.

**Palette contents**:

- `PALETTE_LIGHT` = today's table (unchanged — already tuned for white bg).
- `PALETTE_DARK` = restored dark-era values (green `0dbc79`, yellow `0xe5e510`,
  default fg `0xe5e5e5` — recovered from pre-port git history) with the current
  full 16-entry structure, NOT the old fallback behavior (8-15 all gray, 256+
  white) which the current conversion already fixed.
  **Apply-phase touch-up**: default bg aligned to gpui-component's dark
  `background` token `0x0a0a0a` instead of the historical `0x1e1e1e` — the
  pane frame paints that token (D3) and the spec scenario "no visible seam
  against the pane frame" requires the two to be identical (per Risks:
  adjust constants, not structure). Light side already matches (`0xffffff`).
- 256-cube and grayscale conversion: standard xterm math, shared by both
  themes (spec: "Higher xterm indices keep the standard conversion").

**Render source of truth**: `terminal_grid` callers pass the active theme
snapshot obtained from `cx.theme()` in the two call sites (render path +
test fixture).

### D3. Chrome colors: full semantic-token migration, zero literals

**Decision**: replace all 35 `rgb(0x..)` literals with `cx.theme()` tokens.
Mapping table:

| Current literal | Site(s) | Token |
|---|---|---|
| `0xe9e9e9` hover bg | sidebar ×4, tree | `list_hover` |
| `0xd3e6f7` active row | sidebar | `list_active` |
| `0xcccccc` border | sidebar, widgets | `border` |
| `0xd9d9d9` separator | pane, tree | `border` |
| `0xffffff` backgrounds | pane frame, md/editor viewers, widgets | `background` |
| `0x969696` / `0x666666` / `0x888888` muted text | pane, sidebar, tree | `muted_foreground` |
| `0xcd3131` red (close ×, exited) | sidebar, tree | `danger` |
| `0xe5c110` / `0x4bd76c` badges | sidebar | `warning` / `success` |
| `0x4ba3d7` blue (pane border, status dot) | pane | `accent` |
| `0x005fb8` tree hover link | tree | `accent` (hover) |
| `0x8f6f00` starting placeholder | pane | `warning` |
| `0x0078D959` selection overlay | pane | keep — translucent system blue works on both themes; revisit only if contrast fails in dark |

**Why full tokens (user decision D3)**: switching themes stays zero-maintenance,
colors stay source-of-tradeable with the component layer, and future brand
changes touch theme data only. Alternative — keeping brand-blue literals —
rejected: it reintroduces per-theme literal maintenance for two call sites.
Trade-off accepted: exact pixel values change (e.g. hover becomes the token's
neutral-100); visual deltas are minor and spec only pins "blue", which accent
satisfies.

**Note**: `ThemeColor` tokens are `Hsla`; call sites use `Rgba`. gpui-pre
provides `From<Hsla> for Rgba` — convert at the call site or use the token
directly where the API accepts `Hsla`.

### D4. pi-shell-rs injection wiring (dependent work)

**Decision**: at session spawn, build a `TuiTheme` from the active theme mode
and pass it in `EmbedConfig.theme`; on appearance change, after re-theming the
UI, iterate live sessions and call `set_theme`. Default (uninjected) pi-shell-rs
behavior stays as-is (their R2), so this wiring can land before or after their
API — tasks are marked blocked until `TuiTheme` / `EmbedConfig.theme` /
`set_theme` exist in the locked dependency.

**Why build at spawn + push on change** (vs pi-shell-rs reading a shared
atomic): the host owns theme truth (D1 context); push keeps pi-shell-rs a pure
consumer, matches its `set_title`/`copy_text` callback precedent, and makes the
spawn-time value correct for the first frame.

**Known intermediate state**: until this lands, light theme shows dark-designed
tool-card/syntax colors inside the terminal — already the status quo, tracked
by the parallel repo's work.

## Risks / Trade-offs

- [Subscription dropped early → appearance changes ignored] → store the
  `Subscription` on `TerminalApp`; verify with a manual light/dark toggle test
  in the apply phase.
- [`fg_to_gpui` signature change ripples into tests] → small, known surface
  (`src/pane/mod.rs` internal tests + fixture); update call sites with the
  explicit theme argument in the same task.
- [Dark palette values restored from history may need contrast touch-ups] →
  values are a starting point; apply-phase visual check on real content (pi
  TUI output, badges) before marking done; adjust constants, not structure.
- [Token migration shifts exact colors (hover/active/borders)] → accepted in
  D3; specs pin behavior (legibility, "blue"), not hex values.
- [pi-shell-rs API not yet available → injection tasks blocked] → W1-W3 are
  independent and land first; injection tasks are explicitly marked blocked-on-
  external and do not gate the rest.
- [Selection overlay single value across themes] → translucent system blue over
  both light and dark cells; flagged for contrast verification in apply.

## Migration Plan

1. Land D1+D2+D3 together (they compose into one coherent visual switch; a
   partial landing leaves mismatched chrome/terminal halves).
2. Run existing test suite (color assertions updated in-step), then manual
   visual pass: sidebar/tree/pane/viewers + terminal defaults in both
   appearances.
3. When pi-shell-rs API is released, bump the unpinned dependency
   (`cargo update -p pi-shell-rs`), enable injection tasks, re-verify terminal
   content in both themes.
4. Rollback: revert the single change commit — no data or persisted format is
   touched (D1: no stored preference exists).

## Open Questions

- None blocking. Whether to add a manual theme override later is explicitly
  deferred (D1) and would not change these specs — it would be its own change.
