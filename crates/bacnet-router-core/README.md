# bacnet-router-core

Portable BACnet router behaviour from Clause 6 of ASHRAE 135, for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet). It sits beside
the portable MS/TP and B/IP cores, `bacnet-mstp-core` and `bacnet-bip-core`,
and is meant to join their ports in firmware, and to run under the hosted
`BACnetRouter` of `bacnet-network`.

- `#![no_std]`, no allocator, and by default no dependencies besides
  `bacnet-time`.
- NPDUs are read in place from your frame and written into your buffer.
- The routing table, the discovery state and the router's output scratch
  live in arrays whose size you pick with const generics, all zero bits
  until used, so firmware keeps them in `.bss`.
- Time is an `Instant` you pass to each call; the crate reads no clock. It
  is `bacnet-time`'s, in nanoseconds, re-exported here, the same type the
  MS/TP and B/IP cores take, so firmware feeds all three one clock.

`bacnet-network`'s hosted `BACnetRouter` runs on it. The repository's
firmware link test builds a router with it, between a BBMD's B/IP port
and an MS/TP port with extended frames, into a bare-metal image with no
allocator: on a Cortex-M3, about 45 KB of flash, 15.5 KB of statics and
5.6 KB of stack
([#1679](https://github.com/jscott3201/rusty-bacnet/issues/1679)).

## What it has

- `npdu`: a borrowed view of an NPDU's control information and a writer.
  The view accepts and refuses exactly what `bacnet-encoding`'s decoder
  does, field for field, which a property test checks; an address longer
  than any data link uses is reported apart, with what a router needs to
  address its reject.
- `message`: the network-layer message types and reject reasons, and views
  and writers for the messages' data: network lists, rejects, routing-table
  entries and the rest.
- `table`: the routing table, sorted by network number, with direct and
  learned routes, standard and hardened convergence, busy and unreachable
  marks, a hold-down on reject-driven changes, and aging that keys off
  confirmation, never traffic.
- `discovery`: when to look for a route to an unknown network, at most once
  per cooldown each, in bounded storage.
- `router`: the sans-IO engine. A `Router` takes each frame a port
  receives, with the time, and returns what it means for the router's own
  application; the sends it calls for are polled after, one at a time,
  from its scratch. A port is data to it (a network number, the router's
  MAC there, and the longest NPDU the port carries), so B/IP, MS/TP and the
  hosted transports all plug in the same way.
- `bip` (feature `bip`): B/IP ports over `bacnet-bip-core`'s node engine,
  in normal or foreign-device mode, or as a BBMD over its tables. A
  Forwarded-NPDU reaches the router from its origin, a unicast to a group
  address is refused, and a foreign device's startup announcements wait
  for its registration. Datagrams leave through a sink you implement,
  which encodes each straight into your socket's buffer.
- `mstp` (feature `mstp`): MS/TP ports over `bacnet-mstp-core`'s master
  core, which must run with extended frames. Sends queue at their network
  priority, and a request passed on is answered Reply Postponed.

## Features

None by default. `bip` adds `bacnet-bip-core` and `bacnet-bip-codec`, and
`mstp` adds `bacnet-mstp-core`. Neither needs `std` or an allocator, and
every combination builds on bare-metal targets.

## Example

```rust
use bacnet_router_core::npdu::{Address, Npdu};
use bacnet_router_core::table::{Reachability, Table};
use bacnet_router_core::{Instant, Mac, PortId};

// Network 1 is on port 0; network 9 lies behind router 0x07 on port 1.
let now = Instant::from_secs(1);
let mut table: Table<8, 8> = Table::new();
table.add_direct(1, PortId(0));
assert!(table.learn(9, PortId(1), Mac::from_array([0x07]), now).applied());

// Node 0x0A on network 1 sends an APDU to node 0x05 on network 9.
let frame = [0x01, 0x20, 0x00, 0x09, 0x01, 0x05, 0xFF, 0x10, 0x08];
let npdu = Npdu::parse(&frame).unwrap();
let dnet = npdu.destination.unwrap().network;
let route = table.lookup(dnet).unwrap();
assert_eq!(route.port(), PortId(1));
assert_eq!(table.effective_reachability(dnet, now), Some(Reachability::Reachable));

// Toward the next router the destination stays, the arrival network and
// the sender's MAC become the source, and the hop count drops by one.
let relayed = Npdu {
    source: Some(Address { network: 1, mac: &[0x0A] }),
    hop_count: npdu.hop_count - 1,
    ..npdu
};
let mut out = [0u8; 32];
let len = relayed.write(&mut out).unwrap();
assert_eq!(len, relayed.encoded_len());
assert_eq!(
    &out[..len],
    &[0x01, 0x28, 0x00, 0x09, 0x01, 0x05, 0x00, 0x01, 0x01, 0x0A, 0xFE, 0x10, 0x08],
);
```

## License

MIT
