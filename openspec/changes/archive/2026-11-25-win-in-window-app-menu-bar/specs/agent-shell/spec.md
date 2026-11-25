# Spec Delta

## MODIFIED Requirements

### Requirement: Copy selection with Cmd+C

While a selection is active and the console pane has focus, pressing the
platform's primary-modifier C shortcut SHALL copy the selected text to the
system clipboard and SHALL NOT modify the selection. The primary-modifier
shortcut is Cmd+C on macOS and Ctrl+C on Windows and Linux; the binding
SHALL resolve per platform rather than being fixed to a single modifier, so
the shortcut that works is the one shown beside the menu item. The copied
text SHALL be assembled per row from the live frame — skipping wide-character
continuation cells — with trailing whitespace per row removed and rows joined
by newlines. When no selection is active, the shortcut SHALL NOT write to the
clipboard.

#### Scenario: Copy selected text

- **WHEN** the user selects two lines containing ASCII and CJK text and
  presses the platform primary-modifier C shortcut
- **THEN** the clipboard contains exactly those cells' text, CJK characters
  appear once (not duplicated by their continuation cell), trailing spaces
  are trimmed, and the lines are joined by a newline

#### Scenario: Cmd+C without selection leaves clipboard untouched

- **WHEN** the user presses the platform primary-modifier C shortcut while no
  selection is active
- **THEN** the system clipboard content is unchanged

#### Scenario: Shortcut resolves to Ctrl+C on Windows

- **WHEN** the user runs the application on Windows and presses Ctrl+C with
  an active selection in the console pane
- **THEN** the selection is copied to the clipboard, and pressing the Windows
  system-reserved Win+C does not perform the copy

#### Scenario: Shortcut resolves to Cmd+C on macOS

- **WHEN** the user runs the application on macOS and presses Cmd+C with an
  active selection in the console pane
- **THEN** the selection is copied to the clipboard
