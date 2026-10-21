# bacnet-router-core

Portable BACnet router behaviour from Clause 6 of ASHRAE 135, for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet). It sits beside
the portable MS/TP and B/IP cores, `bacnet-mstp-core` and `bacnet-bip-core`,
and is meant to join their ports in firmware, and to run under the hosted
`BACnetRouter` of `bacnet-network`.

- `#![no_std]`, no allocator, and by default no dependencies besides
  `bacnet-time` and `bacnet-npdu`.
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
4.5 KB of stack
([#1679](https://github.com/jscott3201/rusty-bacnet/issues/1679),
[#1774](https://github.com/jscott3201/rusty-bacnet/issues/1774)).

## What it has

- `npdu`: `bacnet-npdu`, re-exported, the network-layer vocabulary: a
  borrowed view of an NPDU's control information and its encoder, the one
  NPDU codec of the stack (`bacnet-encoding`'s owned NPDU goes through it
  too); the message types, reject reasons and network priority; the port
  limits; and the bounded `Mac`. An address longer than any data link uses
  is reported apart, with what a router needs to address its reject.
- `message`: views and writers for the network-layer messages' data:
  network lists, rejects, routing-table entries and the rest.
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
  MAC there, its group-address rule, and the longest NPDU the port carries), so B/IP, MS/TP and the
  hosted transports all plug in the same way. `Router::originate` sends
  the router's own application's requests through the same table, from a
  source address it selects. Two opt-in policies, both off by default:
  `Options::congestion` announces Router-Busy and Router-Available for a
  port whose egress stays full, and `Options::duplicates` drops a global
  broadcast seen again within its time to live.
- `bip` (feature `bip`): B/IP ports over `bacnet-bip-core`'s port
  driver (`NodePort` in normal or foreign-device mode, `BbmdPort` for a
  BBMD, which answers management requests from its tables). `BipPort`
  hands the router what a port delivers and carries out the router's
  sends on it. A Forwarded-NPDU reaches the router from its origin, a
  unicast to a group address is refused, and a foreign device's startup
  announcements wait for its registration. Datagrams leave through a sink
  you implement, which encodes each straight into your socket's buffer.
- `mstp` (feature `mstp`): MS/TP ports over `bacnet-mstp-core`'s master
  core, which each port owns and which must run with extended frames.
  Sends queue at their network priority, and a request passed on is
  answered Reply Postponed.

## Port group rules

Use `BipPort::config`, or `mstp::port_config` (the `port_config` method on
a port already built over its core), to build the router's port
configuration. Each carries its link's group rules, so
`router::Permissive` handles group destinations
without an extra hook. An Unconfirmed-Request for a directly attached
port's own group leaves as a local broadcast. Other APDUs and network
messages for groups, and B/IP groups at another UDP port, are dropped and
counted. This conversion is an interoperability rule of this router.

For another link, construct `PortConfig::new(network, mac, limits, groups)`.
The default `Groups` type is copyable; `Groups::None` explicitly describes
a link with no group addresses. An owned custom `GroupClassifier` can
return `No`, `Broadcast`, or `Refuse` and retain shared state; use its type
as the last parameter of `Storage` and `Router`. It needs no allocator
unless the classifier itself uses one. `Host` handles authorization and
events only. These rules are consulted on the destination port, never for
a DADR behind another router or an empty DADR.

## Features

None by default. `bip` adds `bacnet-bip-core` (and through it
`bacnet-bip-codec`), and `mstp` adds `bacnet-mstp-core`. Each re-exports
its core, as `bip_core` and `mstp_core`, so a router image needs this crate
alone; `RouterIn` names a router over borrowed storage. The router's
counters drain like the link cores': `take_counters()` returns `u32` counts
since the last take, and `counters()` looks without taking.
`Options::STANDARD` fits any storage; `Options::HOSTED` is the hosted
router's, with its 256-route cap. Neither needs `std` or an allocator, and
every combination builds on bare-metal targets.

## Example

```rust
use bacnet_router_core::npdu::{Address, Mac, Npdu};
use bacnet_router_core::table::{Reachability, Table};
use bacnet_router_core::{Instant, PortId};

// Network 1 is on port 0; network 9 lies behind router 0x07 on port 1.
let now = Instant::from_secs(1);
let mut table: Table<8, 8> = Table::new();
table.add_direct(1, PortId(0));
assert!(table.learn(9, PortId(1), Mac::from_array([0x07]), now).applied());

// Node 0x0A on network 1 sends an APDU to node 0x05 on network 9.
let frame = [0x01, 0x20, 0x00, 0x09, 0x01, 0x05, 0xFF, 0x10, 0x08];
let npdu = Npdu::decode(&frame).unwrap();
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
let len = relayed.encode_into(&mut out).unwrap();
assert_eq!(len, relayed.encoded_len());
assert_eq!(
    &out[..len],
    &[0x01, 0x28, 0x00, 0x09, 0x01, 0x05, 0x00, 0x01, 0x01, 0x0A, 0xFE, 0x10, 0x08],
);
```

`examples/bip_mstp_router.rs` runs a router between B/IP and MS/TP, its
B/IP port also the network's BBMD, in the main loop firmware runs: every
core's deadline merged with `bacnet_time::earliest`, and the BBMD's
management answers coming from its tables:
`cargo run -p bacnet-router-core --features bip,mstp --example bip_mstp_router`.

## License

MIT
