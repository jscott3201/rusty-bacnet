---
section: Added
---
- On Unix, `bacnet tui` lets agents attach read-only to a profile authorized for MCP (`bacnet mcp connect`), profiles authorized earlier included, unless `tui_listener = false`; the `[mcp]` limits apply only while one is attached, and its workspace is kept private (#2185).
