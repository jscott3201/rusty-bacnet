# bacnet-ops

The UI-free operations core of the `bacnet` tool in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. The
one-shot commands, the terminal UI and the MCP server (`bacnet-mcp`; see
[`docs/design/mcp.md`](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/mcp.md)) share it, so they read
targets, values and devices the same way and obey the same request limits.

- `decode`: offline B/IP, B/IPv6, BACnet/SC and MS/TP header summaries and detailed field trees,
  including parse diagnostics, source ranges and forwarded/routed addresses.
- `target`: the target grammar (`192.168.1.10`, `1234`, `2:1234`,
  `bip6:0x0004D2`, `mstp:5`, ...).
- `parse`: the object, property and value grammar (`ai:1`,
  `present-value[3]`, `72.5@8`).
- `format` and `label`: property values as display text, and how objects,
  properties and command priorities are named.
- `limits`: a global request-rate cap, one outstanding confirmed request
  per device, optional rates per network, and per-caller accounts with
  their own token buckets.
- `client`: `Client`, the read operations over a `BACnetClient`:
  properties (by ReadPropertyMultiple, falling back to ReadProperty),
  the object list, a device summary, trend log pages, alarms and the
  alarm and enrollment summaries, Who-Is and Who-Is-Router-To-Network
  with the discovery table, and Who-Has. `Client::bip()`, `bip6()`,
  `sc(..)` and `mstp(..)` connect without naming a transport type. A
  discovery (`Discover`) or an object search (`Find`) sends a local
  broadcast, which no router passes on, unless it names one network or
  every network.
- `reading` and `error`: what a read returns, a `Value` (typed data,
  display text and the octets sent) a property, gathered into `Readings`
  by a bulk read, which keeps a refused property's error in its row and
  what was read when the device stops answering; and `OpsError`, why a
  call failed.
- `inventory`: a device's objects as point records
  (`Device::inventory`): each object's name, units, state texts and
  whether it takes commands, with its value when asked, read in batches of
  a few objects a request and written as JSON or CSV.
- `export`: an inventory in another model's terms. So far `export::s223`,
  an experimental point list in the terms of proposed ASHRAE Standard 223,
  as Turtle: the inventory's records as the point-list types of
  `bacnet_types::points`, then a writer with no I/O.
- `watch`: the watch hub. One managed COV subscription per point (finite
  lifetime, renewed, cancelled when the last listener leaves) or a poll
  every 10 s or slower when the device refuses COV, however many listeners
  share it; it keeps the latest values, so reading a watched point sends
  nothing, and folds bursts of notifications, counting what it folds.

The parts only the tool uses (the write-safety core, command classes
and the BBMD survey) are the
[`bacnet-tool`](https://crates.io/crates/bacnet-tool) crate.

It is a hosted crate: it needs `std` and Tokio, and has no terminal or MCP
dependency. Its `sc-tls` feature turns on the BACnet/SC transport underneath
and `Client::sc`, `serial` MS/TP over a serial port and `Client::mstp`;
`test-support` adds the 223 export's structural check and decode frame
builders for front-end tests, and is not a stable API.

The API follows the needs of the `bacnet` tool and changes with it before
1.0.
