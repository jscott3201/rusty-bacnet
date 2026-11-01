---
section: Added
---
- On Windows too, agents attach read-only to a TUI through `bacnet mcp connect`, over a local named pipe that only the TUI's own user may open; each end checks the other's user before any MCP traffic, and refusals are audited (#2201).
