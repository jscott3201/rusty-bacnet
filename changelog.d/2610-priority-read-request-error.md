---
section: Fixed
---
- A read the client or its link fails before anything was read (no route, a link refusing the send) fails once with that error, where each property showed it as text; after the first rows a bulk read keeps them and ends with it. Seen in the TUI, watches, MCP and the Python reads (#2610).
