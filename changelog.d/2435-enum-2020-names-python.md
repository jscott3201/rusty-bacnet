---
section: Changed
---
- **Breaking (Python API):** the same nine constants are renamed on
  `Segmentation`, `EngineeringUnits`, `PropertyIdentifier` and
  `NetworkPortCommand`, and `.name` returns the new names. The CLI, TUI and
  MCP server print the new kebab-case names and no longer accept the old
  ones (#2435).
