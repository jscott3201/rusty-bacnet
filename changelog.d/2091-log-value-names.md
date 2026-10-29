---
section: Added
---
- `bacnet read-range` and the MCP `bacnet_read_trend` tool name a Trend Log's
  logged enumerations, bit strings and binary or multi-state states, using the
  monitored object's own state texts, at the cost of an extra read; values
  print as before when it fails (#2091, #2122).
