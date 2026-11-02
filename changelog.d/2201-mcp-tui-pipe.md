---
section: Added
---
- On Windows too, agents attach read-only to a TUI through `bacnet mcp connect`, over a local named pipe only the TUI's user may open, named anew on each TUI run from secrets in the profile's workspace; each end checks the other's user first, and refusals are audited (#2201, #2219, #2237).
