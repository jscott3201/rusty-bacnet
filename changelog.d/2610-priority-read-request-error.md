---
section: Fixed
---
- A read whose request was never sent (no route, or a link refusing it) fails once with that error, where each property showed it as text: priority reads, object and bulk reads, the TUI browser, the write desk's read before a write, a watch's first read and polls, and MCP's reads (#2610).
