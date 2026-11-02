---
section: Added
---
- With `mcp_writes = "elicit"`, agents attached to `bacnet tui` can write: each write opens the TUI's preview dialog under an agent banner, requests queue one at a time, and only a confirmation there sends it; a decline or the 120 s timeout returns `declined` (#2185).
