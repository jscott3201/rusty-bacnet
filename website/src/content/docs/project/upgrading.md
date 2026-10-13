---
title: "Upgrade from v0.12.0 to v0.13.0"
description: "Review the breaking changes by area before replacing a working integration."
---

Do not treat this pre-1.0 update as a drop-in replacement merely because the package name stayed the same. v0.13.0 moves the MS/TP and BACnet/IP transports and the router onto new portable `no_std` cores, names BACnet/IPv6 peers by VMAC, and changes Rust and Python APIs and some on-the-wire behavior. This page groups what changes for code and deployments built on v0.12.0.

:::note[This guide and the changelog]
The [0.13.0 section of the changelog](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/CHANGELOG.md) lists every change by area and issue, and its migration notes give the exact replacements. This guide groups what changes for code written against v0.12.0.
:::

## Before updating

Record your current package/binary version, enabled features, platform, transport, and important application workflows. On BACnet/IPv6, list every address you store (recipient lists, bindings, COV subscriptions, configuration): their form changes in this release, and a router must be upgraded together with the BACnet/IPv6 nodes it serves. Keep a known-working deployment or environment available according to your operational policy.

## Update the selected interface

In a Python virtual environment:

```sh
python -m pip install --upgrade rusty-bacnet==0.13.0
```

For Rust, move every `bacnet-*` dependency to `0.13.0` together; all seventeen crates share the version, and six are [new on crates.io](#new-portable-crates). A crate that depends on `bacnet-types`, `bacnet-encoding` or `bacnet-services` with `default-features = false` must now turn on `std` or the new `alloc` feature, or it won't build (#1604). For the CLI, [download the v0.13.0 executable](/rusty-bacnet/start/installation/#install-a-cli-executable), check it against `SHA256SUMS`, and confirm its version and features.

The packaging is unchanged: the same platforms, glibc floor and CPython versions as v0.12.0.

## Rust API

- `bacnet_types::error::Error` is `#[non_exhaustive]`, so a `match` on it needs a wildcard arm. A ReadRange answer that breaks a rule now fails with `Error::ReadRangeViolation` naming the rule instead of `Error::Decoding`, and strict validation refuses some pages it took before; `ReadRangeValidation::Lenient` keeps them (#1530, #1531, #1604).
- Struct literals need new fields, or `..Default::default()`: `ClientConfig` takes `min_request_interval_ms` and `share_port_by_address`, `ServerConfig` takes `share_port_by_address` and `on_reinitialize`, `SessionConfig` takes `min_request_interval_ms`, and a full `DeviceConfig` takes `apdu_segment_timeout`, which must match the server's (#1138, #1535, #1538, #1542, #1593).
- `handle_reinitialize_device` returns the decoded `ReinitializedState` instead of `()` (#1138).
- `AccessPointObject::set_access_event` takes an `AccessEventReport`, and `set_number_of_authentication_policies` no longer refuses a count below Active_Authentication_Policy (#1132, #1325).
- `ColorObject::set_present_value` takes a `BACnetXyColor`, and the colour objects' `set_present_value` and `set_min_max` return `Result`. `MemberDatatype::of` takes the member's object type, and `constructed_channel_value` replaces `is_lighting_command_channel_value` (#1474).
- A custom `LogBufferRecords` implements `record_identity`, and a wrapper that forwards `BACnetObject` methods forwards the new `report_access_input_internal` and `set_present_value_from_internal` too (#1132, #1536, #1552).
- `EndpointReadRequest::Range` takes a `ReadRangeValidation`, and the endpoint's range answer is a `ReadRangeReply`. `PolicyReason` gains `GroupSource` (#1504, #1531).
- `set_life_safety_operation_expected_local` must be awaited inside a Tokio runtime, like the other local writes (#1520).
- `decode_app_primitive` takes its label as any `impl Display` (#1645).
- `BvlcFunction`, `BvlcResultCode`, `Bvlc6Function` and `Bvlc6ResultCode` move out of `bacnet-types`: import them from `bacnet_transport::bvll` and `bacnet_transport::bip6` (#1604, #1621).

## Python API

- BACnet/IPv6 peers are named by VMAC. Pass `bip6:<device instance>`, `bip6:0x<6 hex digits>` or the result of `await client.resolve_vmac("[ip]:port")`; an `"[ip]:port"` string is refused. `DiscoveredDevice.mac_address` and a COV notification's `source_mac` are 3 octets on BACnet/IPv6, and an ipv6 server's `local_address()` reads `bip6:0x…` (#1671).
- An ipv6 `BACnetServer`'s VMAC is its device instance, so `start()` fails when another node on its link and port uses that instance. A client picks a random VMAC unless it gets `ipv6_device_instance`, which it needs on a link without multicast, such as Linux loopback (#1671).
- A ReadRange page that breaks a rule raises `BacnetReadRangeViolationError` naming the rule; `validation="lenient"` keeps such pages, which some devices send (#1531).
- On MS/TP, a client or server waits longer by default for a successor, a poll answer and a reply ([MS/TP](#mstp)); `mstp_usage_timeout_ms` and `mstp_reply_timeout_ms` set those waits (#1655).

The new keywords need no change to existing code: `bbmd=` and `foreign_device=` on the B/IP owners, `min_request_interval_ms`, `share_port_by_address`, the `mstp_*` options, and `segmentation_supported` with `apdu_segment_timeout_ms` on a full server (#938, #1535, #1538, #1593, #1611, #1653, #1655).

## CLI

- With `--ipv6`, targets take `bip6:<device instance>` or `bip6:0x<6 hex digits>`. An `[ip]` or `[ip]:port` target is first resolved to the VMAC of the node there, and addresses print as `bip6:0x…`. Without `--device-instance` the CLI needs a link with multicast (#1671).
- `register` and `discover --bbmd` take the global `--ttl`, and `--foreign-bbmd` with `--renewal` runs the whole session as a registered foreign device (#938).
- `read-range` takes `--position`, `--sequence` or `--time`, pages a log with `--all`, and shows the rules a page broke; `--strict` refuses such a page (#1532).
- The packet decoder shows BVLC function 0x0C as `12`, and reports a capture whose BVLC Length disagrees with its UDP payload as a decode error (#1604).

## Wire behavior

- Color and Color Temperature serve their addendum rows: Status_Flags, Event_State, Reliability and Out_Of_Service answer UNKNOWN_PROPERTY, Color's Present_Value is writable, and Default_Fade_Time refuses a value outside 100 to 86,400,000 ms (#1474).
- An Access Point serves Access_Event_Authentication_Factor, and its Active_Authentication_Policy can read 0, with Reliability CONFIGURATION_ERROR, when no usable policy is in effect (#1132, #1325).
- The server, client and endpoint ignore a confirmed request whose link-layer source is a group address (#1504).
- A WriteProperty of Tags to an object without Tags answers that it has no such property, whatever the value (#1553).
- An accepted WARMSTART or COLDSTART ends DISABLE_INITIATION at once, and the full server answers a matching Who-Has while initiation is disabled (#1567, #1590).
- A full server's segment timeout is configurable and matches what its Device declares; a narrow endpoint refuses a segmentation declaration it can't serve (#1593).

## MS/TP

- The transport runs on `bacnet-mstp-core`. `MasterNode`, `MasterState` and `bacnet_transport::mstp_frame` are gone: build and read frames with `bacnet_transport::mstp::codec`, whose `FrameType` has constants (`TOKEN`) where the enum had variants (`Token`), and whose CRC functions are in its `crc` module (#1608, #1684).
- `MstpConfig` gains `extended_frames`, `usage_timeout`, `reply_timeout` and `transmit_latency_pad`; end a literal with `..MstpConfig::default()`. `start()` and the new `validate()` refuse `max_info_frames` 0 and baud rates below 2667 (#1608, #1611, #1654, #1655).
- A port waits 35 ms for its successor or a polled station and 300 ms for a reply, the longest Clause 9.5.3 allows, so an answer a USB adapter delivers late isn't missed. v0.12.0 waited 20 ms after passing the token, 10 ms after a poll and 255 ms for a reply; set `usage_timeout` and `reply_timeout` to shorten the waits (#1655).
- A port with no drain, such as `TokioSerialPort` on Windows, adds `transmit_latency_pad` to each frame's wire time, 4 ms by default on Windows (#1654).
- A custom `SerialPort::read` waits for data: three `Ok(0)` reads in a row now end the transport as a read error does. Declare an adapter that hears its own frames with `with_echo(Echo::full())`, or `mstp_echo=True` in Python (#1608, #1653, #1656).
- `MstpDiagnosticsSnapshot` loses its received-frame counts; read them from its new `mac` field (#1608).
- Extended frames (Types 32 and 33) carry NPDUs of up to 1497 octets when `extended_frames` is on. It is off by default; `BACnetRouter` turns it on for every MS/TP port it routes, so a router's MS/TP ports declare an APDU limit of 1476 (#1611, #1663).
- Token passing follows Clause 9 more closely: turnaround before every frame, the waiting states' handling of unexpected and invalid frames, and the successor search and maintenance polls with a successor above Max_Master. A frame dropped after a valid header, for a COBS type the port doesn't take, a bad source or a length too long, is skipped whole, where v0.12.0 looked for a new frame inside its body (#1608, #1609, #1640).

## B/IP and BBMD

- The client, server and endpoint builders take one `BipMode`. On the endpoint builder, `.mode(BipMode::Foreign(config))` replaces `register_as_foreign_device`, and `.mode(BipMode::Bbmd(BbmdOptions { .. }))` replaces `enable_bbmd` and the BBMD setters. A `ForeignDeviceConfig` literal adds `renewal_interval: None` (#938, #1013).
- A `BipTransport` set up both as a BBMD and as a foreign device fails `start()`. A foreign device manages its own registration, so a manual Register-Foreign-Device, Delete-Foreign-Device-Table-Entry or Write-BDT to its own BBMD fails; send those from a transport in normal mode (#1622).
- A foreign device takes a Forwarded-NPDU only from its BBMD by unicast, holds up to 16 broadcasts until its first registration attempt is answered, and retries a lost registration sooner (#1622). `bvlc_client_snapshot()` reports how its registration goes (#1013).
- `bbmd_state()` hands out a `std::sync::Mutex`: lock it without `.await`, never across an await, and drop `.await` from `fdt_counters()`. `BbmdState::bdt()` and `fdt()` return copies, and `encode_bdt`, `encode_fdt` and the forwarding helpers are gone (#939, #1623, #1669).
- A custom `TransportPort` overrides `bip_port` instead of `normal_bip_endpoint`, and `publish_bip_port_internal` takes the port's mode (#939).
- A Forwarded-NPDU's `BvllMessage::payload` now starts with the 6-octet origin: read the NPDU with `npdu()` and the origin with `origin()`. `encode_bvll` takes a `&Bvll`, and several `bvll` constants are renamed (#1604).
- The transport drops a datagram whose BVLC Length field is wrong, refuses to send an NPDU over 1497 octets, and fails a unicast to one of the link's group addresses; `NetworkLayer::send_apdu` sends an Unconfirmed-Request for such an address as a broadcast (#1604, #1622, #1677).
- A BBMD rebroadcasts a peer's Forwarded-NPDU on its subnet when its own BDT row is two-hop and the datagram came by unicast, not by the sender's row (#1623).
- A registered Network Port follows the transport's mode: a BBMD serves its BDT and FDT, and a foreign device its BBMD's address (#939).

## B/IPv6

- A B/IPv6 transport's MAC is the node's 3-octet VMAC instead of its 18-octet address: pass a peer's VMAC to sends, client requests, recipient lists and bindings, and rewrite the addresses you stored. `Bip6Transport::resolve_virtual_address` finds a peer's VMAC from its address, and `local_address()` gives this node's (#1637).
- `encode_bip6_mac` and `decode_bip6_mac` are gone; build and read an address with `Bip6Address`. `Bvlc6Frame`, `decode_bvlc6`, `encode_bvlc6` and the message helpers become `Bvll6` values with `decode_bvll6` and `encode_bvll6` (#1621, #1671).
- A unicast to a VMAC the transport hasn't heard sends Address-Resolution and waits up to 3 s before failing; a router port queues instead (#1637).
- A node answers a BBMD's Forwarded-Address-Resolution for its VMAC, and NAKs the requests only a BBMD serves (#1632, #1638).

## The router

- `BACnetRouter` runs on `bacnet-router-core`. `table()` returns a synchronous guard: drop the `.lock().await`, and don't hold it across an await or call `table()` again while it lives (#1679).
- The table's types are the core's: `RouteEntry`, `ReachabilityStatus`, `ConvergenceMode` and `RoutingClaimSnapshot` are `Route`, `Reachability`, `Convergence` and `ClaimCounters`, ports are `PortId`s and next hops `Mac`s. `lookup` returns an owned `Route`, read through getters such as `port()`, `is_direct()` and `next_hop()`, and `lookup_mut` is gone. `add_direct` and `add_learned` return `bool`, refusing a new network once the table holds 256 routes (#1679, #1684).
- Calls that read the clock take `router.now()`: `add_learned(.., now)` replaces `add_learned_at`, `touch(.., now)` replaces `touch_used_at`, and `learn` or `apply_claim` replaces `add_learned_with_flap_detection` and its `_at` form. `purge_stale` and `expire_pending` report what they drop through a callback instead of a `Vec`, the listings (`networks()` and the rest) are iterators in ascending order, so `sorted_networks()` is `networks()`, and `wire_port_id` is `PortId::wire_id`. The [Rust API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/rust-api.md#moving-a-router-table-from-012) lists every change (#1679, #1684).
- `start` refuses a router with no ports, more than 255, or a port whose MAC is longer than 18 octets (#1679).
- A `TransportPort` wrapper delegates the new `max_npdu()` and `prepare_for_routing`, and a custom transport that carries more or less than 1,497 octets overrides `max_npdu()` (#1611, #1665).
- On the wire the router follows Clause 6 more closely. At startup each port broadcasts Network-Number-Is, then its I-Am-Router-To-Network. An NPDU too long for the port it would leave by draws Reject-Message-To-Network reason 4, and a reject for any reason but 1 or 2 no longer removes a learned route. The router's own list of reachable networks leaves out unreachable ones when it answers Who-Is-Router-To-Network, and a request naming a network behind the arrival port goes unanswered. An NPDU for another router, or a global broadcast, that arrives with a hop count of 1 isn't passed on; delivery onto an attached network is unchanged. An APDU for the router's own MAC on any of its networks is delivered locally (#1665, #1679).
- To a group DADR on a directly connected port the router passes on only an Unconfirmed-Request, counting anything else in `group_dadr_drops()` (#1504, #1677).

## BACnet/SC

- `ScConnection::hub_max_apdu_length` is `hub_max_npdu_length`; its value is unchanged (#1679).
- After a failed return to the primary hub, the next attempt waits a full restore interval, and a restore attempt no longer blocks traffic on the failover hub (#1555, #1575).
- A router port on BACnet/SC carries the NPDUs its hub accepts, read when the port starts (#1665).

## New portable crates

Six crates publish for the first time. They are `no_std`, need no allocator and take their time from the caller, so firmware can use them directly, and like the rest they need Rust 1.93. The Tokio MS/TP and B/IP transports and the router run on them; hosted code that doesn't call them changes nothing for them.

- `bacnet-time`: the nanosecond `Instant` and `Duration` the cores share, so firmware feeds them one clock. It has no features (#1681).
- `bacnet-mstp-codec`: MS/TP frames, standard and extended, with their CRCs and a stream decoder. `alloc`, `bytes` and `heapless` add encode sinks, and `crc32k-table` a table-driven CRC-32K (#1604, #1611).
- `bacnet-bip-codec`: every B/IP and B/IPv6 BVLL message, with typed addresses and VMACs. `alloc`, `bytes` and `heapless` add encode sinks (#1604, #1621).
- `bacnet-mstp-core`: the MS/TP master node. `alloc` adds heap-backed queues (#1605).
- `bacnet-bip-core`: the B/IP node in its three modes, and a BBMD's tables and forwarding. It has no features (#1622, #1623).
- `bacnet-router-core`: the Clause 6 router, with B/IP and MS/TP ports behind its `bip` and `mstp` features (#1679).

Each crate's README and docs.rs page show how to drive it, and the [Rust API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/rust-api.md) covers the three cores' firmware use. `bacnet-types`, `bacnet-encoding` and `bacnet-services` also build as `no_std` with an allocator, through their new `alloc` feature (#1604).

## Verify more than import success

Exercise a known read, error handling, any routing or segmentation paths you depend on, subscription lifecycle, relevant server objects, and controlled cleanup. MS/TP deployments need on-wire validation, since the master node and its timing are new. BACnet/IPv6 deployments need every stored address checked in its VMAC form, and a router tested with the nodes it serves.

Retest your application's output parsing where it depends on CLI addresses, typed Python values or Rust models. Separate regressions from corrected behavior that an older integration may have depended on accidentally. The [integration guides](/rusty-bacnet/development/overview/) explain shared endpoints, Network Port registration and SC setup.

## Sources and release scope

This guide covers the move from **v0.12.0** to **v0.13.0**. Source review is not hardware qualification.

[v0.13.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.13.0) · [Changelog](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/CHANGELOG.md) · [Rust API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/rust-api.md) · [Python API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/python-api.md).
