---
section: Fixed
---
- The MCP server caps what a session's listen streams can make it poll:
  per-device alarm URIs count against the session's 64-URI bound shared with
  legacy subscriptions, and a session runs at most 4 listen streams (#2150).
