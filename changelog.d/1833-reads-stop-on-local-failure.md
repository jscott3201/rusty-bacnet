---
section: Fixed
---
- MCP reads and searches, a trend's state texts, and the CLI's and TUI's
  name and state-text reads stop when the client's own link fails (a stopped
  transport, say) instead of taking each object for one with no name or
  texts; see `OpsError::failed_locally()` (#1833).
