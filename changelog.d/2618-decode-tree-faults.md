---
section: Fixed
---
- The frame decode tree (TUI capture, MCP `bacnet_decode`) marks a closing
  tag that doesn't match, an unclosed one and contents past the end, which
  it took silently, after what it read; a legacy EVENT_PARAMETERS value its
  decoder delimits keeps its named fields (#2618).
