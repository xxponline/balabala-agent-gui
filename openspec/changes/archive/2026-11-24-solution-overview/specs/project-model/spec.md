# Spec Delta

## MODIFIED Requirements

### Requirement: Solution creation via the SolutionConfig tab

A workspace with zero solutions SHALL offer an add-solution action in the
left slot. Activating it SHALL open a SolutionConfig tab in the center slot
with a path input instead of a system file picker. The tab SHALL show
prechecks for the entered path — empty, duplicate, and valid (an existing
directory) — and SHALL enable submission only while they all pass.
Submitting SHALL append a `simple` solution item for that path to
`balabala.solutions.json`; the resulting solution row and its project SHALL
appear in the left slot without restarting the application. Closing the tab
without submitting SHALL leave the file and the view unchanged.

#### Scenario: Add solution from empty workspace

- **WHEN** the user activates the add-solution action in an empty workspace,
  enters `/Users/me/code/foo`, and submits with all prechecks passing
- **THEN** `balabala.solutions.json` gains one `simple` item for that path
  and a solution row for `foo` with the project row `foo` beneath it appears
  in the left slot

#### Scenario: Failed precheck blocks submission

- **WHEN** the entered path does not exist as a directory, or duplicates the
  root of an existing solution or project
- **THEN** submission is disabled and the failing precheck is shown in the tab

#### Scenario: Cancelled picker writes nothing

- **WHEN** the user activates the add-solution action, uses the tab's browse
  button, and cancels the system picker
- **THEN** `balabala.solutions.json` is unchanged and no project row appears

#### Scenario: Closing the tab writes nothing

- **WHEN** the user activates the add-solution action and closes the tab
  without submitting
- **THEN** `balabala.solutions.json` is unchanged and no project row appears

#### Scenario: Successful creation closes the tab

- **WHEN** a submission succeeds and its project row appears
- **THEN** the SolutionConfig tab closes itself

#### Scenario: Root outside the workspace is accepted

- **WHEN** the entered path is an existing directory outside the workspace
  directory and all other prechecks pass
- **THEN** submission appends the item like any other valid path
