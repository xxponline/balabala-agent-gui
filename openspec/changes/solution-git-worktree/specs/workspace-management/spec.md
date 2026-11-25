# Spec Delta

## MODIFIED Requirements

### Requirement: Workspace data lives in the workspace directory

All workspace-scoped state SHALL be stored inside the workspace directory:
marker and configuration files now, and git worktree working copies under
the workspace's `.balabala.worktrees` directory. The application root SHALL
NOT store workspace configuration.

#### Scenario: Config written inside workspace

- **WHEN** workspace configuration changes after the workspace is open
- **THEN** the write lands in files under the workspace directory, and no
  workspace file in the application root is modified

#### Scenario: Worktrees written inside workspace

- **WHEN** a worktree is created for a project of the open workspace
- **THEN** its working copy exists under the workspace directory's
  `.balabala.worktrees`, not under the application root and not beside the
  project's own files
