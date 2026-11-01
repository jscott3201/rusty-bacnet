---
section: Security
---
- On Windows, the TUI's MCP pipe name now mixes in a per-user secret kept in the profile's workspace, so another local user can no longer work the name out and take it first; the TUI refuses a workspace other users may list (#2219).
