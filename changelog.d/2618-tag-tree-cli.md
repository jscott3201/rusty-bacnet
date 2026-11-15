---
section: Changed
---
- **Breaking (CLI JSON):** the CLI, TUI and MCP show a value with a context
  tag and no decoded form as its tags, not `[raw ..]`; its JSON `decoded` is
  a `tagged` element, the octets under `fields.data`, not `{"raw": ...}`
  (#2618).
