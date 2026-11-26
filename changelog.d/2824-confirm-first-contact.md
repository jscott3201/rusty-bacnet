---
section: Security
---
- A device named by its instance (handles, `bacnet` commands, MCP tools, Python's
  `client.device(n)`) is reached only where it answered the client's own
  question of which device it is, never on an announcement alone; a station
  answering as a device no one else claims is still taken (#2824).
