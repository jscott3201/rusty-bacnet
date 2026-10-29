---
section: Security
---
- The CLI and TUI drop escape sequences, bidirectional overrides and other
  invisible format characters from device names, values and messages in table
  output, so a device string can't reorder, hide or break rows; JSON keeps the
  exact text (#2122, #2144).
