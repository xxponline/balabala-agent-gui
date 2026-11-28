# Design

## Context

Today the theme pipeline is: `gpui_kit::init` → `theme::init` forces
`ThemeMode::Light`; the main window's `TerminalApp::new` then calls
`Theme::sync_system_appearance` and subscribes to
`window.observe_window_appearance`, re-syncing (and re-injecting the palette
into live sessions via `push_tui_theme`) on every system flip. `theming`
requirement 1 codifies this as unconditional, with an explicit ban on override
and persistence; the archived `add-light-dark-theme` D1 deferred a manual
toggle to "a settings surface and a three-state (system/light/dark) model"
and noted the subscription pipeline would not need reworking.

Known constraints this design works within:

- `ThemeMode` (gpui-component) is upstream's `{Light, Dark}` — no System state;
  the three-state model must live on our side of the boundary.
- No settings panel exists anywhere in the app; the menu bar (native on macOS,
  in-window on Windows, both fed from one `menu::build`) is the only existing
  selection surface. `MenuItem.checked` is honored end-to-end:
  `gpui-fast-macos` maps it to `NSMenuItem.setState`, and
  `PopupMenu::with_menu_items` maps it to the in-window check icon.
- The app starts at the Launcher before any workspace exists, so a
  workspace-scoped file cannot serve startup (this is also why a dark-system
  user currently sees a light Launcher — nothing syncs until
  `TerminalApp::new`).
- `workspace_history.json` (`<ProjectDirs>`) is the precedent for global
  convenience data: versioned JSON, missing/corrupt → default, write failure
  logged not surfaced.

## Goals / Non-Goals

**Goals:**

- Three-state preference (system / light / dark), default system, persisted
  globally, applied before the Launcher.
- Pinned modes hold against system flips; follow-system keeps today's live
  switch behavior and session re-injection.
- One selection surface: `View > Theme`, three check-marked items, from the
  single `menu::build` source on both menu presentations.

**Non-Goals:**

- `View` menu during the Launcher phase (dispatch target does not exist yet;
  the preference already applies silently at startup — proposal records this).
- Keyboard shortcuts for theme switching; any other settings surface or
  settings for other concerns.
- Surfacing preference-write failures to the user (convenience data, history
  semantics: log and keep in-memory state).
- Changing how `TuiTheme` palettes are built (`src/ui/mod.rs` already maps
  `ThemeMode` → palette; preference only decides which `ThemeMode` is active).

## Decisions

### D1. Three-state modeled on our side as `ThemePreference`

`enum ThemePreference { System, Light, Dark }` in a new `src/theme_preference.rs`,
with `resolve(appearance) -> ThemeMode` for the System case. gpui-component's
`ThemeMode` stays untouched (upstream type, and `cx.theme().mode` remains the
resolved two-state truth everything else already reads).

- *Alternative*: fork/extend upstream `ThemeMode` with a System variant —
  rejected: invasive upstream change, every consumer would need to handle a
  state they resolve differently, for zero benefit since resolution is a
  one-liner at the edge.

### D2. Persistence: `<ProjectDirs>/theme_preference.json`, history semantics

Versioned JSON `{"version":1,"mode":"system"|"light"|"dark"}` next to
`workspace_history.json`. Missing, unreadable, wrong version, or unrecognized
value → `System` (the default). Load/save through `load_at`/`save_at`-style
functions on an explicit path, mirroring `workspace/history.rs`.

- *Alternative*: workspace-scoped file — rejected: unreadable during the
  Launcher phase (would leave the startup gap unfixed and require MRU-path
  reverse lookup), and theme is a property of the user's eyes, not of a
  workspace's contents; switching workspaces must not jump the theme.

The in-memory value lives as a field on `TerminalApp` (like `history_path`),
and the path is injectable for tests — the `cfg(test)` default path is
process-shared, so tests that write must pass their own temp path (the
discipline `history.rs` documents).

### D3. Apply point: right after `gpui_kit::init`, before `launcher::open`

One helper `apply_theme_preference(pref, window: Option<&mut Window>, cx)`
(= `Theme::change` for pinned, `sync_system_appearance` for System) called from
`main.rs` at startup and again from `TerminalApp::new` (where a real window
exists, so System resolution uses `window.appearance()`, the more accurate
source the upstream comment recommends). Calling it twice is idempotent.

- *Alternative*: only in `TerminalApp::new` (today's shape) — rejected: the
  Launcher phase would keep rendering the forced Light startup theme, leaving
  the known gap and making the stored preference invisible until a workspace
  is chosen.

### D4. Appearance subscription kept, gated on `System`

`observe_window_appearance` stays registered for the window's lifetime; the
handler becomes: if preference == `System` → `sync_system_appearance` +
`push_tui_theme` (today's body), else no-op. Pinned modes therefore ignore
system flips without unsubscribing, and switching System→pinned→System
naturally resumes following — the "return to system snaps to current
appearance" scenario is just the handler running on the next sync, plus an
explicit sync at the moment the preference changes back.

- *Alternative*: unsubscribe while pinned — rejected: the subscription object
  already must live on `TerminalApp`; gate vs. resubscribe churn buys nothing
  and adds a state variable that can drift.

### D5. Toggle action and menu wiring

`SetThemeMode { mode: ThemePreference }` action (derive `Action`, same
registration pattern as `crate::debug::OpenDebugSection`: global
`app.on_action` + `cx.defer` because menu dispatch runs inside the window
update lease and the handler touches the `TerminalApp` entity). Handler steps,
in order:

1. update in-memory field;
2. `apply_theme_preference` (Theme::change or sync-for-System);
3. save the file (log on failure, keep in-memory);
4. `push_tui_theme` — session re-injection is requirement 5's "when the theme
   changes" clause, independent of who changed it;
5. `rebuild_menus` — existing path (`set_menus` + `feed_global` +
   `AppMenuBar::reload`), which recomputes checkmarks from the field on next
   build.

`menu::build` gains the current preference parameter (threaded through
`feed_global`, `install`, `rebuild_menus` — mechanical, single source
preserved). The `View` menu with the three check-marked items is appended in
`build` only; `build_launcher` is untouched (non-goal).

- *Alternative*: re-read the file on every menu build — rejected: file I/O in
  a render-path build, and the field is already the single source of truth
  the handler maintains.

### D6. Test strategy

- Existing `theme_follows_window_appearance_on_startup` survives unchanged:
  no file in its injected temp path → `System` → resolves to appearance, which
  is exactly what it asserts today.
- New tests: store round-trip + corrupt/unknown → default; startup with a
  pinned `dark` file renders dark on the test platform (whose appearance is
  Light — pinned-vs-appearance is observable for free); gated handler (pinned
  ignores an appearance change, System follows); menu build carries three
  items with exactly one check on the active preference; toggle writes file,
  flips `Theme::global(cx).mode`, and fires `push_tui_theme`; rebuild
  preserves the checkmark.
- Real-machine smoke (macOS + Windows) for native/in-window checkmark
  rendering — source reading says both work (`setState` / check icon), runtime
  confirmation stays a verification task.

## Risks / Trade-offs

- [Native checkmark rendering only source-verified, not yet seen at runtime]
  → early smoke task; both mappings are one-liners in dependencies we already
  ship, and the in-window path is additionally covered by an existing render
  test pattern.
- [Menu API surface churn: `build`/`feed_global`/`install` signatures] →
  one parameter threaded through three call sites, all internal (no public API
  in this crate).
- [Process-shared `cfg(test)` default preference path could leak between
  tests] → same discipline as history: only no-file/default tests use the
  default path; any test that writes injects a temp path.
- [Menu action fires inside the window update lease; handler touches the
  entity] → `cx.defer` flush pattern, already proven by the Debug-menu
  dispatch.
- [Preference file written on every toggle — torn write on crash] →
  convenience data: corrupt file is spec'd to fall back to default; single
  small `serde_json` write, no reader/writer concurrency in-process.

## Migration Plan

Purely additive; no existing data, config, or behavior changes for users who
never open `View > Theme` (default `System` reproduces today's pipeline,
scenario-for-scenario). Rollback = revert the commits; a leftover
`theme_preference.json` is inert. Archive-time spec merge rewrites `theming`
requirement 1 in place and appends the `app-menu-bar` requirement.

## Open Questions

None. (Windows checkmark rendering is source-verified via
`with_menu_items`; runtime confirmation is a verification task, not a design
unknown.)
