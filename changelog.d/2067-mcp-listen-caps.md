---
section: Added
---
- A session runs at most 4 MCP listen streams, and their per-device alarm
  URIs count against the session's URI bound shared with legacy
  subscriptions, so the alarm poll stays bounded (#2150).
