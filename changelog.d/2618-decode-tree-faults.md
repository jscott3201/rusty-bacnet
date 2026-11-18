---
section: Fixed
---
- The frame decode tree (TUI capture, MCP `bacnet_decode`) reports a closing
  tag that doesn't match its opening tag, an unclosed one and contents past
  the end, which it took silently, as a `Tag walk stopped` leaf where the
  walk stopped, after what it read (#2618).
