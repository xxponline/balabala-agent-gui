# project-model Specification

## Purpose

Turns a workspace's solution configuration into the Project/Module structure
everything else anchors on: it owns the `balabala.solutions.json` contract,
the solution-type registry, load-time validation, and the structural shape
projects and modules must satisfy.

## Requirements


### Requirement: Solution configuration file

A workspace SHALL carry its solution configuration in `balabala.solutions.json`
in the workspace directory: a JSON array of solution items, each declaring a
`SolutionType`. Each item SHALL carry two configuration flags — whether its
projects register into the asset manager, and whether git management is
enabled for it — and a missing flag SHALL default to enabled, so files
written before the flags existed load with unchanged behavior. The file SHALL
be created (empty array) together with the workspace marker; a workspace
without the file SHALL load as a valid workspace with zero projects.

#### Scenario: Empty solutions file yields zero projects

- **WHEN** a workspace loads and its `balabala.solutions.json` contains `[]`
- **THEN** the workspace opens with no project rows and no error

#### Scenario: Missing solutions file is a valid empty workspace

- **WHEN** a workspace loads and its `balabala.solutions.json` does not exist
- **THEN** the workspace opens with no project rows and no error

#### Scenario: Legacy item without flags loads with unchanged behavior

- **WHEN** a solutions file predates the flags and its items declare
  neither of them
- **THEN** both flags behave as enabled: the item's projects register into
  the asset manager and git management applies as before
### Requirement: Solution type registry resolves items to projects

Each solution item's `SolutionType` SHALL resolve through a registry of
solution implementations; v1 SHALL ship exactly one type, `simple`, whose item
carries a filesystem `path` and SHALL produce exactly one project rooted at
that path with a single root module named after the project.

#### Scenario: SimpleSolution item produces one project

- **WHEN** the solutions file contains one `simple` item with
  `path: "/Users/me/code/foo"`
- **THEN** the model contains exactly one project rooted at
  `/Users/me/code/foo` whose root module is named `foo`

#### Scenario: Two simple items produce two projects

- **WHEN** the solutions file contains two `simple` items with distinct paths
- **THEN** the model contains exactly two projects, one per item
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
### Requirement: Solution load failure blocks the workspace

If `balabala.solutions.json` cannot be parsed, declares an unknown
`SolutionType`, or a `simple` item's `path` does not exist on disk, the system
SHALL show an error dialog describing the failure and SHALL NOT open the
workspace. The dialog SHALL appear at every load boundary (startup restore,
manual workspace switch, explicit reload) — the system SHALL NOT fall back to
an empty project list.

#### Scenario: Corrupt solutions file at startup

- **WHEN** the application restores the most-recent workspace at startup and
  its `balabala.solutions.json` contains invalid JSON
- **THEN** an error dialog describes the parse failure and the application
  exits without rendering the workspace

#### Scenario: Solution path missing on disk

- **WHEN** a workspace loads and a `simple` item names a directory that does
  not exist
- **THEN** an error dialog describes the missing path and the workspace is
  not opened

#### Scenario: Unknown solution type

- **WHEN** a solutions file contains an item whose `SolutionType` is not in
  the registry
- **THEN** an error dialog describes the unknown type and the workspace is
  not opened
### Requirement: Solution creation via path picker

A workspace with zero projects SHALL offer an add-solution action in the left
slot. Activating it SHALL open a directory picker; picking a directory SHALL
append a `simple` solution item for that path to `balabala.solutions.json` and
the resulting project SHALL appear in the left slot without restarting the
application. Cancelling the picker SHALL leave the file and the view unchanged.

#### Scenario: Add solution from empty workspace

- **WHEN** the user activates the add-solution action in an empty workspace
  and picks `/Users/me/code/foo`
- **THEN** `balabala.solutions.json` gains one `simple` item for that path and
  a project row `foo` appears in the left slot

#### Scenario: Cancelled picker writes nothing

- **WHEN** the user activates the add-solution action and cancels the picker
- **THEN** `balabala.solutions.json` is unchanged and no project row appears
