# Spec Delta

## REMOVED Requirements

### Requirement: Loading and multi-instance creation
**Reason**: 主骨架收敛为"每契约恰一实例"——多实例从未被负载均衡/分客户消费（那些场景由容器型服务内部自理），其遗留的"同名 N 实例共享 dlopen"语义与单实例强制直接冲突。
**Migration**: 装载语义由新需求 "Single-instance loading per contract" 承接（同契约任何名字的二次装载显式拒载）；DLL 库常驻不 dlclose 的承诺原样保留在新需求中。

## ADDED Requirements

### Requirement: Single-instance loading per contract

`load()` SHALL create the one instance of its service contract. While an
instance of that contract exists in the instance table — in any state,
including `error` — every further `load()` resolving to it SHALL be refused
with an explicit error naming the occupying and the requested service, and no
instance SHALL be created. `unload()` SHALL remove the instance and release
that contract; it SHALL NOT dlclose the library — reclamation happens only at
process exit.

#### Scenario: Second load of the same service is refused

- **WHEN** `load()` is called for a service name whose contract already has
  an instance in the table
- **THEN** the load is refused with an explicit error naming the occupying
  and the requested service, and no second instance is created

#### Scenario: Same contract under a different name is refused

- **WHEN** a different Catalog name resolving to the same contract attempts
  `load()` while an instance of that contract exists
- **THEN** the load is refused with an explicit error instead of creating a
  second instance of the same class

#### Scenario: Unload releases the contract slot but keeps the library resident

- **WHEN** an instance is unloaded
- **THEN** it disappears from the instance table, its contract becomes
  loadable again, and the DLL remains loaded for the rest of the process
  lifetime

### Requirement: Contract identity declaration

Every Catalog service SHALL carry a contract identity minted by the host.
Internal services SHALL declare it at registration; a DLL artifact SHALL
expose its contract name as a fixed exported symbol. The Loader SHALL resolve
that name through a host-registered contract table before creating an
instance, and SHALL refuse the load with an explicit error when the symbol is
missing or the name is unknown — never a silent failure.

#### Scenario: Internal service declares its contract

- **WHEN** an internal service registers with a contract marker and is loaded
- **THEN** its Catalog entry and its instance carry that contract identity in
  the instance table

#### Scenario: Missing or unknown contract symbol is refused explicitly

- **WHEN** `load()` targets a DLL artifact whose contract symbol is absent or
  names a contract the host contract table does not know
- **THEN** the load is refused with an explicit error and no instance is
  created

#### Scenario: Distinct contracts load side by side

- **WHEN** two loaded services declare different contract identities
- **THEN** both instances coexist in the instance table
