# Spec Delta

## MODIFIED Requirements

### Requirement: Asset registration flag gates project handoff

Each solution SHALL carry a flag controlling whether its projects register
into the asset service. Projects of a solution with the flag disabled SHALL
be absent from the handoff tree while remaining present in the solution's
own project set; toggling the flag SHALL take effect on the next handoff
without restarting, and the flag SHALL persist to the solutions file.

#### Scenario: Disabled solution stays out of the asset tree

- **WHEN** a solution with asset registration disabled resolves to one
  project and the workspace opens
- **THEN** the project is absent from the asset registration handoff

#### Scenario: Enabling mid-session registers on next handoff

- **WHEN** the user enables asset registration for that solution
- **THEN** the project appears in the handoff and its assets enumerate
  without restarting the application
