# bacnet-bip-core

Portable BACnet/IP behaviour from Annex J of ASHRAE 135, for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet). It sits above
`bacnet-bip-codec`, which has the wire format, and below a socket.

- `#![no_std]`, no allocator, no dependencies besides `bacnet-bip-codec`
  and `bacnet-time`. The codec is re-exported as `bacnet_bip_core::codec`,
  since the API speaks its types.
- Tables live in arrays whose size you pick with a const generic.
- Time is an `Instant` you pass to each call; the crate reads no clock. It
  is `bacnet-time`'s, in nanoseconds, re-exported here, the same type
  `bacnet-mstp-core` and `bacnet-router-core` take, so firmware feeds them
  one clock. A `now` earlier than the latest one passed in is taken as
  that latest one: time never runs backwards, so a clock that steps back
  holds the timers still instead of failing.
- The types that may grow are `#[non_exhaustive]`: build a `Datagram`, a
  `bbmd::Context`, a `LocalSubnet` or a `bbmd::Registration` with `new`;
  start a `Link` or a `Foreign` from `new`, or the `RegistrationLimits`
  from `DEFAULT`, and set the fields to change; and give a `match` on an
  error, a refusal, a `Discard` or another of these enums a wildcard arm.

## What it has

The `node` module is a sans-IO B/IP node in normal, foreign-device and BBMD
mode:

- `Link`, its fixed configuration, decides what to do with each received
  datagram (the receive rules, own echoes, the NAKs a node that is not a
  BBMD owes, a Forwarded-NPDU's origin) and what to send for an NPDU, within
  a datagram budget. Its decisions are pure.
- `Node` keeps the rest: its own management exchanges, matched by peer and
  operation; a foreign device's registration, with backoff after a loss and
  a lease that never counts on the BBMD's grace period; broadcasts held
  until the first registration resolves; and the timers. You poll its
  actions after every call.

The `bbmd` module holds a BBMD's broadcast distribution table and foreign
device table, and decides where each broadcast goes:

- an Original-Broadcast-NPDU from the local subnet, to the peer subnets
  (one-hop or two-hop, from each BDT row's mask) and the foreign devices;
- a peer BBMD's Forwarded-NPDU, to the foreign devices and, when it came
  to this BBMD alone (its own BDT row is two-hop and it arrived by
  unicast), the local subnet;
- a registered foreign device's Distribute-Broadcast-To-Network;
- the BBMD's own broadcasts.

Each decision writes its destinations into storage you pass and returns the
Forwarded-NPDU to send them. Foreign-device registration applies
time-to-live limits, a per-address quota and reserved capacity, and asks
your own admission check (an access list, a rate limit) in between.
Registrations expire on your clock, with the 30-second grace period.

The Tokio B/IP transport in `bacnet-transport` runs its node and its BBMD on
this crate.

## Example

```rust
use core::net::{Ipv4Addr, SocketAddrV4};
use bacnet_bip_core::bbmd::{Context, Tables, Target};
use bacnet_bip_core::codec::{BdtEntries, BdtEntry};
use bacnet_bip_core::Instant;

let own = SocketAddrV4::new(Ipv4Addr::new(10, 0, 1, 1), 47808);
let peer = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 1), 47808);
let mut tables: Tables<8, 16> = Tables::new(own);
tables
    .bdt
    .set(BdtEntries::from_entries(&[BdtEntry::new(peer, Ipv4Addr::BROADCAST)]))
    .unwrap();

let cx = Context::new(Instant::from_millis(1_000), 16);
let source = SocketAddrV4::new(Ipv4Addr::new(10, 0, 1, 20), 47808);
let mut out = [Target::UNUSED; Tables::<8, 16>::MAX_TARGETS];
let sent = tables.original_broadcast(&cx, source, &[0x01, 0x00], &mut out);
for (destination, message) in sent.transmissions() {
    // Encode `message` (a Forwarded-NPDU) and send it to `destination`.
    let _ = (destination, message);
}
```

## License

MIT
