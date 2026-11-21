---
section: Changed
---
- The frame decode tree in the TUI capture screen, MCP `bacnet_decode` and
  Python shows an MS/TP data CRC as `valid` or `mismatch` with its span,
  never its value, and a frame whose CRC fails keeps its Data field
  (#2633).
