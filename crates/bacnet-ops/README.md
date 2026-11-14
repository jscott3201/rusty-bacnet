# bacnet-ops

The UI-free operations core of the `bacnet` tool in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. The
one-shot commands, the terminal UI and the MCP server (`bacnet-mcp`; see
[`docs/design/mcp.md`](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/mcp.md)) share it, so they read
targets, values and devices the same way and obey the same request limits.

- `target`: the target grammar (`192.168.1.10`, `1234`, `2:1234`,
  `bip6:0x0004D2`, `mstp:5`, ...).
- `parse`: the object, property and value grammar (`ai:1`,
  `present-value[3]`, `72.5@8`).
- `format` and `label`: property values as display text, and how objects,
  properties and command priorities are named.
- `limits`: a global request-rate cap, one outstanding confirmed request
  per device, optional rates per network, and per-caller accounts with
  their own token buckets.
- `engine`: the read operations over a `BACnetClient`: properties (by
  ReadPropertyMultiple, falling back to ReadProperty), the object list, a
  device summary, trend log pages, alarms and the alarm and enrollment
  summaries, and Who-Is and Who-Is-Router-To-Network with the discovery
  table.
- `export`: a device's objects in another model's terms. So far
  `export::s223`, an experimental point list in the terms of proposed
  ASHRAE Standard 223, as Turtle: an inventory read through the engine
  into the point-list types of `bacnet_types::points`, then a writer with
  no I/O.
- `watch`: the watch hub. One managed COV subscription per point (finite
  lifetime, renewed, cancelled when the last listener leaves) or a poll
  every 10 s or slower when the device refuses COV, however many listeners
  share it; it keeps the latest values, so reading a watched point sends
  nothing, and folds bursts of notifications, counting what it folds.

The parts only the tool uses (the write-safety core, command classes,
frame decoding and the BBMD survey) are the
[`bacnet-tool`](https://crates.io/crates/bacnet-tool) crate.

It is a hosted crate: it needs `std` and Tokio, and has no terminal or MCP
dependency. Its `sc-tls` feature turns on the BACnet/SC transport underneath;
`test-support` adds the 223 export's structural check for the MCP server's
tests and is not a stable API.

The API follows the needs of the `bacnet` tool and changes with it before
1.0.
