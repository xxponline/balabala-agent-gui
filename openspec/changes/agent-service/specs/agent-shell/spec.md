# Spec Delta

## ADDED Requirements

### Requirement: Session control failure feedback

Session control requests initiated from the sidebar — selecting, creating, or
closing a session — SHALL give visible feedback when a request fails or times
out before taking effect: session state, active selection, and displayed
content SHALL remain as they were, and a notice SHALL name the failed action.
Input injection (keystrokes, wheel, clicks) is fire-and-forget and SHALL NOT
emit failure notices.

#### Scenario: Failed close keeps the session

- **WHEN** the user closes a session but the control request fails or times
  out before the session is closed
- **THEN** the session remains in the sidebar list, the active session and
  console tab content are unchanged, and a notice names the failed close

#### Scenario: Failed selection keeps the active session

- **WHEN** the user clicks a different sidebar session item but the selection
  request fails or times out before it takes effect
- **THEN** the previously active session remains active and keeps receiving
  keystrokes, and a notice names the failed selection

#### Scenario: Failed create shows a notice without a new row

- **WHEN** the user clicks `[+ agent]` but creating the session fails (for
  example the `pi` binary cannot be started)
- **THEN** no new session row appears and a notice names the failed creation
