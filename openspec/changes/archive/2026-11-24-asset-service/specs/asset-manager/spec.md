# Spec Delta

## REMOVED Requirements

### Requirement: Project registration lifecycle
**Reason**: Capability retired and renamed — the asset manager was a
GUI-process entity fed by direct method calls; registration now arrives at a
registered bus service over the layer.
**Migration**: See `asset-service` → `Project registration lifecycle`.

### Requirement: Asynchronous asset enumeration
**Reason**: Capability retired and renamed; enumeration now runs handed off
the service's handler so it cannot delay other services on the layer.
**Migration**: See `asset-service` → `Asynchronous asset enumeration`, plus
the new `Enumeration does not delay other services`.

### Requirement: Module file enumeration with default dot exclusions
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-service` → `Module file enumeration with default
dot exclusions`.

### Requirement: File-row filtering relative to project roots
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-service` → `File-row filtering relative to
project roots`.

### Requirement: Refresh re-enumerates in the background
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-service` → `Refresh re-enumerates in the
background`.

### Requirement: Enumeration prunes excluded directories
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-service` → `Enumeration prunes excluded
directories`.

### Requirement: Filter provisioning from the project root
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-service` → `Filter provisioning from the project
root`.

### Requirement: Empty and degraded states
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-service` → `Empty and degraded states`.

### Requirement: Data-change notification is O(1) when nothing changed
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-service` → `Data-change notification is O(1) when
nothing changed`.

### Requirement: In-flight enumeration is not restarted by notifications
**Reason**: Capability retired and renamed; behavior unchanged.
**Migration**: See `asset-service` → `In-flight enumeration is not
restarted by notifications`.
