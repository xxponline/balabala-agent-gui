# Proposal

## Why

The right dock still roots its tree at the workspace directory and resolves
that root from the active agent session, while the domain model — now
anchored on Solution → Project → Module — carries no weight in the view at
all: `Module` has no filesystem identity, and the data plumbing lives in
`workspace_property_source`, a plugin layer whose "source" concept predates
projects. This change re-roots the right dock on registered Projects,
gives modules filesystem semantics through asset enumeration, and promotes
the data half of the old source into a service-layer **asset manager** (the
same tier future services such as LSP will occupy).

## What Changes

- **BREAKING: `workspace_property_source` is removed.** Its trait, static
  registry, and root-resolution logic disappear; the `SourceNode` DTO family
  (`SourceNode`/`SourceNodeKind`/`MenuDesc`/`SourceEffect`) moves into the
  asset manager (the open/dispatch system's `OpenObject` follows the kind
  type), and the gitignore-style filter and recursive scanner migrate into
  the manager's enumeration path.
- **New service: `asset_manager` (service layer + UI ViewModel).**
  Solutions register Projects into it when a workspace opens (all projects)
  or a solution is added (incremental, no restart); it owns asynchronous
  enumeration, filtering, refresh, empty/degraded states, generation-based
  O(1) invalidation, and in-flight enumeration dedup — the data-side
  contract the old source trait used to carry.
- **Asset enumeration contracts.** Project and module expose asset
  enumeration: a project enumerates its modules asynchronously; a module
  creates the standard file enumerator from its own root path, which
  recursively enumerates all files (with the default exclusions). Workspace
  files that belong to no project are not managed and never appear.
- **Filtering moves to the manager, re-rooted on projects.** Rules still
  come from the workspace's `file_tree` config, but patterns are now
  relative to each Project root (workspace-root semantics retire with the
  workspace-root tree).
- **The right-dock panel becomes a pure view.** It keeps rendering,
  expansion state, render caching, and the visible affordances (refresh
  control, read-only rule summary, left-click open, context menu), with
  data and dispatch provided by the asset manager. Its scope no longer
  depends on an active agent session: scope is the registered project set.
- **Registration wiring in the app layer.** Successful workspace open
  clears and re-registers; adding a solution registers one project; a
  workspace switch replaces the set.

## Capabilities

### New Capabilities

- `asset-manager`: service-layer contract for the right dock's data:
  project registration lifecycle, asynchronous asset enumeration
  (projects → modules → files), project-root-relative gitignore filtering,
  refresh and empty/degraded states, and the O(1)/in-flight guards over
  enumeration data.

### Modified Capabilities

- `file-tree`: rewrites the panel contract — scope follows registered
  projects instead of the active agent's workspace (exclusions, filtering,
  O(1), and in-flight requirements migrate to `asset-manager`), open/menu
  dispatch runs through the asset manager, and display requirements keep
  render caching and hidden-panel cost rules.
- `project-model`: the module structure shape gains filesystem-root
  semantics (a module's root界定 the assets it enumerates).
- `dock-layout`: right-dock default-render wording no longer says
  "workspace file tree".

## Impact

- **Code**: remove `src/workspace_property_source/`; add `src/asset_manager/`
  (DTOs, registration, enumerators, filter, view/generation);
  `src/tree/mod.rs` reworked to consume the manager's view;
  `property_detail_manager` open objects switch their kind type import;
  `src/app.rs` registration hooks at workspace-open and solution-add.
- **Tests**: the existing `tree/mod.rs` source-injection tests rewire to
  registration-based setup; filetree scan/filter tests move with their
  implementations.
- **Specs**: new `asset-manager` capability; `file-tree` largely rewritten;
  small deltas to `project-model` and `dock-layout`.
- **Not affected**: agent sessions, terminal rendering/input, center tabs,
  workspace lifecycle (open/switch/gating), theming.
