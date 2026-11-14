---
section: Added
---
- The new `bacnet-tool` crate holds the parts of `bacnet-ops` only the
  `bacnet` CLI, TUI and MCP server use (write safety, command classes, frame
  decoding, the BBMD survey), so an application on `bacnet-ops` no longer
  builds them (#2560).
