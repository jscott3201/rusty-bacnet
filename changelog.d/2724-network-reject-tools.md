---
section: Added
---
- `bacnet_ops` sorts a router's network reject as `Unreachable`, and a
  following `Device` then looks for its device again; the CLI and TUI show
  the reject decoded, and MCP reads return a `network-reject` error naming
  the router, network and reason (#2724).
