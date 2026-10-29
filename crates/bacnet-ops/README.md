# bacnet-ops

The UI-free operations core of the `bacnet` tool in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. The
one-shot commands, the terminal UI and the MCP server (`bacnet-mcp`; see
[`docs/design/mcp.md`](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/mcp.md)) share it, so they read
targets, values and devices the same way and obey the same request limits.

- `class`: what each command does to a device (read, write or device
  management), the table the front ends' safety levels share.
- `target` and `destination`: the target grammar (`192.168.1.10`, `1234`,
  `2:1234`, `bip6:0x0004D2`, `mstp:5`, ...) and how a target becomes a
  service destination, routed through the client's discovery table.
- `parse`: the object, property and value grammar (`ai:1`,
  `present-value[3]`, `72.5@8`).
- `format` and `label`: property values as display text, and how objects,
  properties and command priorities are named.
- `limits`: a global request-rate cap, one outstanding confirmed request
  per device, optional rates per network, and per-caller accounts with
  their own token buckets.
- `decode`: frames of the tapped links (B/IP, B/IPv6, BACnet/SC, MS/TP),
  NPDUs and APDUs as one-line summaries and trees of fields with their
  octet ranges, with no traffic.
- `engine`: the read operations over a `BACnetClient`: properties (by
  ReadPropertyMultiple, falling back to ReadProperty), the object list, a
  device summary, trend log pages, alarms and the alarm and enrollment
  summaries, Who-Is and Who-Is-Router-To-Network with the discovery
  table, and the BBMD table reads.
- `bbmd`: B/IP broadcast management tables as plain data, the survey that
  follows each BDT to the BBMDs it lists, and the consistency matrix over
  them (one-way entries, missing own entries, mismatched masks).
- `watch`: the watch hub. One managed COV subscription per point (finite
  lifetime, renewed, cancelled when the last listener leaves) or a poll
  every 10 s or slower when the device refuses COV, however many listeners
  share it; it keeps the latest values, so reading a watched point sends
  nothing, and folds bursts of notifications, counting what it folds.
- `safety`: the write-safety core the terminal UI and the MCP server
  share: session levels, confirmation tiers, the write preview, the
  crash-safe ledger of pre-images, the redacted append-only audit log,
  the workspace lock, and the engine half of a write (read again, refuse a
  stale preview, record, send, settle, revert).

It is a hosted crate: it needs `std` and Tokio, and has no terminal or MCP
dependency. Its `sc-tls` feature lets the address formatter recognise a
BACnet/SC link; `test-support` adds frame builders for the CLI's decode
tests and is not a stable API.

The API follows the needs of the `bacnet` tool and changes with it before
1.0.
