# Spec Delta

## MODIFIED Requirements

### Requirement: Project and module structure shape

A project SHALL have a stable name and a root directory path. Each project
SHALL have exactly one root module named after the project and rooted at the
project's root directory. Every module SHALL carry the filesystem root that
defines the assets it enumerates, and MAY gain child modules as solutions
emit them. Project dependencies and module references SHALL form a DAG;
cyclic output SHALL be rejected once solutions can emit multiple projects or
module references.

#### Scenario: Root module shares the project name

- **WHEN** a project is produced with root directory `/a/bar`
- **THEN** the project is named `bar` and its root module is named `bar`

#### Scenario: Cyclic project dependency rejected

- **WHEN** a solution type emits projects A and B where A depends on B and
  B depends on A
- **THEN** loading that workspace fails validation and the failure surfaces
  through the solution load-error contract

#### Scenario: Module root mirrors the project root

- **WHEN** a project rooted at `/a/bar` is loaded with its single root module
- **THEN** the root module's filesystem root is `/a/bar`, defining its
  enumeration scope
