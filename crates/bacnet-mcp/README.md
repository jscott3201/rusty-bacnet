# bacnet-mcp

A [Model Context Protocol](https://modelcontextprotocol.io) server for
BACnet sites, read-only unless a profile offers writes, in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. An AI
agent host (Claude Code, Codex and others) launches it as `bacnet mcp serve
--stdio` (the `bacnet-cli` feature `mcp`) and calls its tools to discover
devices, describe them, find, list and read their objects, read and
summarise trend logs, list alarms, map networks and routers, decode
frames, and follow points as they change.

- Built on the official `rmcp` SDK; targets the 2026-07-28 revision of MCP
  and still answers the older `initialize` handshake.
- Every request goes through the `bacnet-ops` engine and its limits: one
  request at a time per device, a global rate, a rate per network and a
  token bucket per principal. Each result reports what it cost.
- Device-supplied text comes back in an untrusted envelope: cleaned of
  control and bidirectional characters, defanged and capped.
- Results are paged with cursors signed by a per-process key.
- Passive mode builds no client and sends nothing.
- Resources for devices, points, properties, alarms and the session, with
  `subscriptions/listen` (2026-07-28), or `resources/subscribe` from hosts
  on older revisions, on a shared watch hub: one COV
  subscription per point however many listeners, polling when a device
  refuses COV, 32 points per principal and 256 per process. `bacnet_watch`
  gives hosts without subscriptions the same through a tool.

- Writes are off by default. A policy that offers them lists
  `bacnet_write` and `bacnet_revert`, which go through the TUI's preview
  tiers, ledger and audit log (`bacnet-ops::safety`): the client's
  confirmation may approve T1 writes only, a plan grant the writes it
  names, and the confirmation state is sealed for its caller, single use
  and short-lived.

Experimental in 0.15; the API (`McpServer`, `Policy`) may change in any
release before 1.0. The design is
[`docs/design/mcp.md`](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/mcp.md).
