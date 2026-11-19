# service-lifecycle Specification

## Purpose

管理可加载服务的完整生命周期：目录扫描建 Catalog、装载/停用/重载与同服务多实例、每实例标准状态机（loading/available/offline/error）及其事件语义——服务进出生死的唯一权威。

## Requirements

### Requirement: Catalog construction and unified registration

At startup the service layer SHALL scan the service directory once and build a
Catalog of service descriptors. Internal services SHALL register explicitly
into the same Catalog as DLL discoveries; the two are distinguished only by
the origin of their symbols (`Internal(fn)` vs `Dll(path)`) and SHALL be
loadable through one and the same load flow. The Catalog SHALL NOT watch the
file system: artifacts produced after startup SHALL NOT become visible until
an explicit reload.

#### Scenario: Startup scan lists DLL services

- **WHEN** the application starts and the service directory contains plugin
  cdylib artifacts
- **THEN** each discovered artifact appears as a Catalog entry with `Dll`
  origin

#### Scenario: Internal services share the catalog

- **WHEN** an internal service registers at startup
- **THEN** its descriptor sits in the same Catalog as scanned DLL entries
  with `Internal` origin and is loadable through the same load flow

#### Scenario: New artifacts need an explicit reload

- **WHEN** a new artifact is produced into the service directory after
  startup
- **THEN** it does not appear in the Catalog until `reload_catalog()` is
  called

### Requirement: Explicit catalog reload

`reload_catalog()` SHALL re-scan the service directory and SHALL affect future
loads only: loaded instances SHALL keep running untouched with their library
handles held, and a disappeared entry SHALL only prevent new loads. When a
re-scan finds a name that already exists, the new entry SHALL replace the old
Catalog entry; already-loaded instances of the replaced entry SHALL NOT be
affected.

#### Scenario: Reload picks up new artifacts

- **WHEN** a new artifact appears in the service directory and
  `reload_catalog()` is called
- **THEN** the new entry is present in the Catalog and can be loaded

#### Scenario: Reload does not disturb running instances

- **WHEN** instances of a Catalog entry are loaded and `reload_catalog()`
  replaces or removes that entry
- **THEN** the running instances continue in their current state and remain
  addressable; only future `load()` calls see the new Catalog

#### Scenario: Removed entry cannot load new instances

- **WHEN** a service artifact has been deleted and `reload_catalog()` has run
- **THEN** `load()` for that service name fails while existing instances of
  it keep running

### Requirement: Build fingerprint verification

Each DLL service artifact SHALL embed a build fingerprint (rustc version plus
contract-crate metadata hash) exposed as a fixed exported symbol. Before
creating an instance from a DLL entry, the Loader SHALL compare the artifact
fingerprint against the host fingerprint and SHALL refuse to load on mismatch,
surfacing the refusal as an explicit error — never as a silent failure.

#### Scenario: Matching fingerprint loads

- **WHEN** `load()` targets a DLL entry whose embedded fingerprint matches the
  host fingerprint
- **THEN** the instance is created and enters `loading`

#### Scenario: Mismatched fingerprint is rejected explicitly

- **WHEN** a plugin artifact was built against a different contract build than
  the host and `load()` is attempted
- **THEN** the load is refused with an explicit error naming the fingerprint
  mismatch, and no instance is created

### Requirement: Loading and multi-instance creation

`load()` SHALL create one service instance. Multiple instances of the same
service SHALL share a single dlopen of the DLL: one `Arc`-shared library
handle for N instance creations. `unload()` SHALL deactivate the instance and
remove it from the instance table; it SHALL NOT dlclose the library —
reclamation happens only at process exit, and API documentation SHALL NOT
imply that dynamic libraries are reclaimed.

#### Scenario: Two instances from one DLL

- **WHEN** `load()` is called twice for the same DLL service entry
- **THEN** two independent instances exist and the library has been opened
  once

#### Scenario: Unload removes the instance but keeps the library resident

- **WHEN** an instance is unloaded
- **THEN** it disappears from the instance table while the DLL remains loaded
  for the rest of the process lifetime

### Requirement: Per-instance state machine

Each loaded instance SHALL carry exactly one of the states `loading`,
`available`, `offline` (actively deactivated), `error` (failed to start or
crashed). Transitions: `load()` → `loading`; init ok → `available`; init
failure → `error`; `disable()` → `offline`; `enable()` → `loading`; crash →
`error`; `retry()` → `loading`. Not-loaded services SHALL NOT exist in the
instance table; instance deletion SHALL be accepted only from `error`.

#### Scenario: Successful initialization

- **WHEN** `load()` completes service init successfully
- **THEN** the instance transitions `loading` → `available`

#### Scenario: Initialization failure lands in error

- **WHEN** service init fails during `load()`
- **THEN** the instance transitions `loading` → `error` and is retryable

#### Scenario: Disable and re-enable

- **WHEN** the host calls `disable()` on an `available` instance and later
  `enable()`
- **THEN** the instance transitions `available` → `offline` → `loading` and
  re-enters `available` if init succeeds again

#### Scenario: Deletion only from error

- **WHEN** the host requests instance deletion while the instance is
  `available` or `offline`
- **THEN** the request is refused; deletion is accepted only from `error`

### Requirement: Supervisor-only recovery

Recovery authority is the supervisor state machine alone: actor auto-restart
SHALL be disabled. A crashed actor SHALL only report to the supervisor, which
moves the instance to `error` without restarting it; rebuilding happens only
via explicit `retry()` or `enable()`.

#### Scenario: Crash lands in error without auto-restart

- **WHEN** a running service actor crashes
- **THEN** the instance transitions `available` → `error` and is not
  restarted automatically; messages to it fail fast until `retry()` or
  deletion

### Requirement: State transition event delivery

Every state transition SHALL be delivered to subscribed participants through
the bus. Subscribing to an instance's state changes SHALL add a routing rule
that forwards transition events to the subscriber's address. Delivery SHALL be
ordered per subscriber.

#### Scenario: Subscriber receives transition sequence

- **WHEN** a participant subscribes to an instance which then goes
  `loading` → `available` → `error`
- **THEN** the participant receives the three transition events in order at
  its mailbox
