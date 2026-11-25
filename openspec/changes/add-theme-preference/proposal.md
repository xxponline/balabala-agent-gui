# Proposal

## Why

The theme is hard-wired to the operating system appearance: `theming` requirement 1
currently mandates "no in-app theme override and no persisted theme preference",
so a user whose OS sits in dark appearance cannot keep the app light (or vice
versa) — every system flip drags the whole application, terminal palette, and live
session content with it. This was a deliberate deferral, not a rejection: the
archived `add-light-dark-theme` change (design D1) recorded that a manual toggle
needs "a settings surface and a three-state (system/light/dark) model" and left
the subscription pipeline ready for exactly that. This change delivers it.

## What Changes

- **Three-state theme preference**: `follow system` / `light` / `dark`, defaulting
  to `follow system` (today's behavior becomes an opt-out-able option, zero
  regression for existing users).
- **Persisted globally**: the preference is stored in the application data
  directory alongside `workspace_history.json` — it is a property of the user's
  eyes, not of any workspace, and it must be readable during the Launcher phase
  before any workspace exists. Corrupt/missing file behaves as the default
  (convenience-data semantics, same as history).
- **Apply at startup, before the Launcher**: read the preference right after
  `gpui_kit::init` instead of waiting for the main window — this also closes an
  existing gap where a dark-system user sees a light-only Launcher.
- **System-follow becomes conditional**: the window-appearance subscription stays,
  but only syncs while the preference is `follow system`; a pinned light/dark
  preference ignores system flips, and switching back to `follow system` snaps to
  the current appearance immediately.
- **Selection surface — new `View > Theme` menu**: three check-marked items
  (follow system / light / dark) in the application menu bar, native and
  in-window from the same source; toggling writes the preference, changes the
  theme live, refreshes the checkmarks through the existing menu-rebuild path,
  and re-injects the palette into live sessions (existing `theming` requirement 5
  semantics).

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `theming`: requirement 1 ("Theme follows operating-system appearance") changes
  from unconditional system-follow with an explicit ban on override/persistence to
  a persisted three-state preference: startup applies the stored choice, system
  appearance is followed only in `follow system` mode, and pinned modes hold
  against system changes.
- `app-menu-bar`: new requirement for the `View` menu carrying the three-state
  theme selection with a check mark on the active choice and rebuild-on-toggle
  behavior.

## Impact

- **Code**: `src/main.rs` (apply preference after init), `src/app.rs`
  (`TerminalApp::new` — gate the appearance subscription, handle the theme action,
  extend `rebuild_menus`), `src/menu.rs` (`build`/`build_launcher` unchanged for
  launcher, `View` menu + `SetThemeMode` action), new small preference store module
  (mirroring `src/workspace/history.rs` load/save semantics).
- **Tests**: existing `theme_follows_window_appearance_on_startup` survives
  unchanged (no file → default `follow system` → falls back to appearance); new
  tests for persistence round-trip, startup apply of a pinned mode, gated
  subscription, menu checkmarks, and live re-injection on manual toggle.
- **Specs**: `openspec/specs/theming/spec.md` requirement 1 rewritten;
  `openspec/specs/app-menu-bar/spec.md` gains the View-menu requirement.
- **No breaking changes**: default preference reproduces today's behavior exactly.
- **Out of scope**: a `View` menu during the Launcher phase (no main-window
  entity to dispatch into yet — the preference already applies silently at
  startup), keyboard shortcuts for theme switching, additional settings surfaces.
