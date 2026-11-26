---
section: Security
---
- A device named by its instance (handles, watches, `bacnet` commands, the
  TUI's `writem`, MCP tools, old ledger reverts) is reached only where it
  answered the client's own question, after one Who-Is when nothing is
  confirmed yet; a station answering for an absent device still is (#2824).
