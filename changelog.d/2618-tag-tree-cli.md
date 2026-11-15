---
section: Changed
---
- **Breaking (CLI JSON):** the CLI, TUI and MCP show a value with a context
  tag and no decoded form as its tags, not `[raw ..]`; its JSON `decoded` is
  one `tagged` element, the octets under `fields.data`, where it was a list
  with a `{"raw": ...}` item per context element (#2618).
