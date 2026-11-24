# Spec Delta

## REMOVED Requirements

### Requirement: Left-click open hook
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-overview` → `Left-click open hook`.

### Requirement: Right-click context menu
**Reason**: Capability retired and renamed, and its execution model
changed: activating an item no longer dispatches back to the data source
for execution. Copy path and reveal in Finder are marked as executed by the
view and produce no invoke message.
**Migration**: See `asset-overview` → `Right-click context menu`, which
builds the menu from the service's descriptors and executes both entries in
the view.

### Requirement: Refresh triggers and behavior
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-overview` → `Refresh triggers and behavior`.

### Requirement: Frame-driven notifications do not rebuild the tree
**Reason**: Capability retired and renamed; the invalidation input is now
the view published by the service rather than a locally computed one.
**Migration**: See `asset-overview` → `Frame-driven notifications do not
rebuild the tree`.

### Requirement: Tree panel skips work while not rendered
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-overview` → `Tree panel skips work while not
rendered`.

### Requirement: Tree scope follows registered projects
**Reason**: Capability retired and renamed; the registered set is now read
from the view the service publishes.
**Migration**: See `asset-overview` → `Tree scope follows registered
projects`.

### Requirement: Empty and degraded display
**Reason**: Capability retired and renamed; the states are now reported by
the service.
**Migration**: See `asset-overview` → `Empty and degraded display`.
