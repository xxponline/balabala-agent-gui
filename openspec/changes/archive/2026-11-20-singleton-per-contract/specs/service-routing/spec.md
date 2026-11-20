# Spec Delta

## REMOVED Requirements

### Requirement: Two-level addressing
**Reason**: 选择策略（`Direct`/`Any`）的存在理由是同服务多实例；主骨架改为每契约恰一实例后不存在可挑选的候选，"挑哪个"成为永远没有答案的伪问题。
**Migration**: `Route::Service(name, Any)` → `Route::Service(name)`（直呼该契约唯一实例）；`Route::Service(name, Direct(id))` → `Route::Instance(id)`（精确直呼）。替换需求见下方 "Instance and service-name addressing"。

## ADDED Requirements

### Requirement: Instance and service-name addressing

Messages SHALL be addressable by instance (`Route::Instance(id)`) or by
service name (`Route::Service(name)`). The service-name form SHALL carry no
selection policy: the name SHALL resolve to the unique loaded instance of
that contract. Resolving a name whose service is not loaded or whose instance
is not `available` SHALL fail exactly as specified by the fail-fast delivery
requirement — there are no candidate instances to choose among and no hidden
buffering.

#### Scenario: Instance direct delivery

- **WHEN** a participant sends to `Route::Instance(id)` of a loaded instance
- **THEN** the envelope is delivered to that instance's mailbox

#### Scenario: Service name resolves to the unique instance

- **WHEN** a participant sends to `Route::Service(name)` for a loaded,
  `available` service
- **THEN** the envelope is delivered to that contract's single instance

#### Scenario: Service name with no loaded instance fails fast

- **WHEN** a participant sends to `Route::Service(name)` whose service has no
  loaded instance
- **THEN** delivery fails immediately with a routing error instead of
  selecting a candidate or holding the envelope
