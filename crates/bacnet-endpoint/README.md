# bacnet-endpoint

Compose client and server roles for one BACnet device under one transport
owner in the [Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
One [`EndpointSession`] owns the transport, ingress and shared outbound
coordinator. Construct sessions directly with a transport or use
[`bip::BipEndpointBuilder`], [`sc::ScEndpointBuilder`] or
[`mstp::MstpEndpointBuilder`].

This hosted crate requires `std`, Tokio and allocating state. It has no
alloc-only or no-heap profile. Its one optional feature, `sc-tls`, enables
composition over the constrained-TLS SC hub connection. The generic [`sc`]
builder module is available without that feature. The lifecycle, service and
data-link limits below apply to every entry point.

## Quickstart from development source

Place an application directory beside a checkout named `rusty-bacnet` and add
this dependency block. It describes development source, not a published crate
version; adjust the paths for another layout. The program declares all its
direct dependencies, including Tokio's current-thread runtime, macros and time
support.

```toml
[dependencies]
bacnet-endpoint = { path = "../rusty-bacnet/crates/bacnet-endpoint" }
bacnet-objects = { path = "../rusty-bacnet/crates/bacnet-objects" }
bacnet-transport = { path = "../rusty-bacnet/crates/bacnet-transport" }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
tokio = { version = "1", features = ["macros", "rt", "time"] }
```

Run a client-only session and a server-only session over paired in-process
loopback channels. The server is Device 1001 and holds an Analog Input with
Present_Value 72.5; a session with a server role runs a full `bacnet-server`
`BACnetServer`, read-only unless given a server builder. The client reads the
input under a two-second timeout and checks the returned object, property,
array index and literal application-tagged REAL bytes. Both sessions are
stopped explicitly; no socket, retry or sleep is needed.

```rust
use bacnet_endpoint::{DeviceIdentity, EndpointSession, SessionConfig, SessionRole};
use bacnet_endpoint::build_database_with_extra;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::{timeout, Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client_port, server_port) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut analog = AnalogInputObject::new(7, "room-temperature", EngineeringUnits::DEGREES_FAHRENHEIT)?;
    analog.set_present_value(72.5);
    let identity = DeviceIdentity::new(1001, 42)?;
    let database = build_database_with_extra(&identity, vec![Box::new(analog)])?;
    let config = SessionConfig {
        queue_capacity: 16,
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        max_apdu_length: 480,
        ..SessionConfig::default()
    };
    let mut client = EndpointSession::new(client_port, SessionRole::ClientOnly, config.clone())?;
    let mut server = EndpointSession::new(server_port, SessionRole::ServerOnly, config)?
        .device(identity)
        .with_database(database);
    client.start().await?;
    server.start().await?;

    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7)?;
    let handle = client.client()?;
    let ack = timeout(
        Duration::from_secs(2),
        handle.read_property(&[2], object, PropertyIdentifier::PRESENT_VALUE, None),
    )
    .await??;
    assert_eq!(ack.object_identifier, object);
    assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);
    assert_eq!(ack.property_array_index, None);
    assert_eq!(ack.property_value, [0x44, 0x42, 0x91, 0x00, 0x00]);

    client.stop().await?;
    server.stop().await?;
    Ok(())
}
```

Loopback delivers to its paired peer. This example establishes an in-process
ReadProperty exchange and lifecycle cleanup; it does not establish destination
filtering, UDP, TLS or serial operation, real-time behavior, firmware behavior,
physical-device interoperability or BACnet conformance.

## Features and documentation

| Feature selection | Availability |
|---|---|
| Default or `--no-default-features` | Hosted endpoint sessions and the B/IP, generic SC and MS/TP builders; `std`, Tokio and allocation remain required. |
| `sc-tls` | Adds `ScEndpointBuilder::build_hub_session` over the constrained-TLS SC connection, and `connect_hub_session`, which dials the hub with the client's reconnect and failover hub (#1946); it does not enable full standalone-server dispatch parity. |

The docs.rs profile selects `sc-tls` with default features disabled for
`x86_64-unknown-linux-gnu`. The hub-session methods are labeled with their feature
requirement; the generic SC module remains available without it. This README
is also the crate landing page, so the primary program runs as a doctest.

## Optional B/IP socket example

Using the same dependencies, this separate example binds a real IPv4 UDP socket
on an ephemeral port and broadcasts I-Am. It is compile-checked, not run as a
doctest; choose interface and broadcast addresses appropriate to your network
before running it. It does not extend the loopback quickstart's qualification.

```no_run
use std::net::Ipv4Addr;

use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::session::SessionRole;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), bacnet_types::error::Error> {
    let identity = DeviceIdentity::new(1001, 42)?
        .with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)?;
    let db = identity.build_database()?;
    let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::Both)
        .database(db)
        .device(identity)
        .build_session()?;
    session.start().await?;
    session.broadcast_i_am().await?;
    session.stop().await?;
    Ok(())
}
```

## The client on the session

[`EndpointSession::client`](session::EndpointSession::client) gives a full
`bacnet-client` `BACnetClient` on the session's one transport, beside the
server role: discovery in every scope, segmented reads, files, COV, router
discovery, and the link's own calls and frame tap through its `transport()`. A
running session of any role gives one, one at a time: it is owned, so keep it
and share it by reference or in an `Arc`. It shares the session's invoke-ID
pool, so its requests and the server role's notifications never collide, and
`bacnet_ops::Client::over` takes it as it is. Beside a server role the client
answers only the COV notifications of its own subscriptions; the server
answers everything else, a ConfirmedEventNotification among them, which the
client hears a copy of. Of one that comes in segments the server hands the
client the whole notification once it has reassembled it (#2743), so the
client publishes it once per transmission either way. The session has the one
owner of the local network number. With a source Audit Reporter the session
audits the client's reads and writes, each owned by the session once admitted
(#2617); an audited write whose value isn't whole application data is refused
before it is sent, where an unaudited one sends its value as given. One stop
stops the session with the client still alive (#2617): its tasks end, its open
requests fail and the link stops. From then on the client sends nothing more:
its requests and unconfirmed sends fail at once, and a call made directly on
its `transport()`, such as a BVLC query, fails with the transport's stopped
error. Dropping the client afterwards changes nothing. Dropping the session
without a stop ends the client the same way and aborts the link the client
still holds: nothing graceful is sent, and the link's sockets close as the
runtime drops its aborted tasks.

## The server role

A session with a server role (`ServerOnly` or `Both`) runs a full
`bacnet-server` `BACnetServer` on the same transport, which the session builds
at start (#2617). Without a builder it is the read-only server of
[`read_only_server`](session::read_only_server): peers read, discover the node
and its objects and subscribe to COV, while every write, create and delete is
denied, ReinitializeDevice and DeviceCommunicationControl are refused, and
time synchronization is ignored, so a node nobody gave a policy is never
writable by a peer. AcknowledgeAlarm is executed, as on any server.
[`EndpointSession::with_server`](session::EndpointSession::with_server) gives
the session a builder instead, such as `BACnetServer::generic_builder()` with
the policies you want, whose policies then apply as given: a builder with none
lets any peer write, create and delete, as a standalone server does.

The session owns the server: [`server`](session::EndpointSession::server)
borrows it while the session runs, and [`stop`](session::EndpointSession::stop)
stops it first, so its work joins and its saves settle while the link is
still up. The session hands it its database, and it answers every request but
the client's COV notifications. The server's seat is attached as the
link starts, before its first request is classified, so a request that
arrives while the server builds waits for it; its queue is as deep as the
session's request queue. A builder setting a server's start would refuse is
refused before the link starts, leaving the session ready to retry with its
database; only target Audit's recipient as the started link sees it is
checked after, and that refusal ends the session, whose start hands the
database back (#2766). It shares the session's invoke-ID pool, in which each role
the session runs keeps a reserve the other can't take: with both roles the
server always finds 64 of the 256 and may hold 128, and the client always
finds 128 and may hold 192. It also shares the session's network-number
owner, which target Audit follows. The session's registered
Network Port and source Audit Reporter work beside it.

The node is one device. A session with a server role needs a Device identity
([`device`](session::EndpointSession::device)); its start refuses one without,
before the link starts. The server serves the node's Device from the
session's settings: its
identity, Max_APDU_Length_Accepted the smaller of the two roles', the session's
APDU timeout and retries (which the server's confirmed notifications use too),
and on a `Both` session the client's executed services beside the server's. A
builder or database that contradicts them is refused. A DeviceCommunicationControl
the server accepts binds the whole node (Clause 16.1): the client's
requests and the session's own I-Am fail with
`Error::InitiationDisabled` until it ends, unless an operator starts a client
with `ClientOptions::with_node_dcc_override`. Both roles segment as the
identity says, both ways by default (#2617): the server reassembles requests
and segments answers, and the client accepts segmented answers, sends
segmented requests and takes the identity's segment timeout only as far as the
Device's Segmentation_Supported allows. Each segment, Segment-ACK and Abort
reaches the role whose transaction it is. When the Device receives segments
the client also reassembles a segmented COV notification of either form, as
the answering side, and answers it once it is whole (#2646). A segmented
ConfirmedEventNotification is the server's to reassemble and answer; the
client gets the whole notification from the server (#2743).

## What is proven

- The client is the full client, with its own tests in `bacnet-client`. On
  the session, in-memory wire tests show source reporting of its reads and
  writes: one record per eligible ReadProperty, ReadRange or WriteProperty
  and per ReadPropertyMultiple reference, sharing the request's retries, and
  kept through a caller that goes away. Source filters may suppress records.

- One socket / one serial owner per session (RB-16 B/IP + SC-hub proofs,
  RB-17 MS/TP simulator proof). No second hidden socket or serial owner is
  created by the builders or the session.
- I-Am identical to Device ReadProperty for the composed [`identity`]
  on real B/IP loopback UDP and on the constrained-TLS SC hub
  (I-Am readback matrix in `rb16_*_proof` tests).
- Loopback-only coverage for port/UUID/capability corners beyond that
  matrix: [`DeviceIdentity`] Network-Port population, SC UUID sync into
  `DEVICE_UUID`, and service-profile alignment are exercised on
  `LoopbackTransport` / `LoopbackWebSocket` for determinism, not as
  on-wire claims.
- The server role is the full server's, with its own tests in
  `bacnet-server`. On the session, in-memory wire tests show the read-only
  server a session runs by default deny every write, create and delete
  (WriteProperty, WritePropertyMultiple, AddListElement, RemoveListElement,
  CreateObject, DeleteObject, AtomicWriteFile, an inbound WriteGroup),
  refuse ReinitializeDevice and DeviceCommunicationControl, leave the clock
  alone on both time synchronizations, and still answer reads, Who-Is,
  Who-Has and COV subscriptions. Make the application's own changes through
  the server's `write_local`, `set_present_value_local` and
  `set_reliability_local`, whose COV and event work follows them.

## Data-link support matrix (explicit, not silent)

| Data link | Endpoint builder | Evidence |
|-----------|------------------|----------|
| B/IP IPv4 unicast + local broadcast | [`bip::BipEndpointBuilder`] | Real loopback UDP proof (RB-16) |
| B/IP BBMD / foreign-device mode | [`bip::BipEndpointBuilder::mode`] | The standalone builders' transport and modes; loopback Network Number wire coverage; [`session::EndpointSession::foreign_registration`] reports a registration; [`session::EndpointSession::bvlc_tables`] reads a BBMD's tables through the session's socket; the BVLC writes stay on [`BipTransport`](bacnet_transport::bip::BipTransport) |
| BACnet/SC via hub | [`sc::ScEndpointBuilder`] | Real constrained-TLS hub proof (RB-16, `sc-tls`); loopback composition for unit validation |
| MS/TP | [`mstp::MstpEndpointBuilder`] | **Simulator evidence only** (`LoopbackSerial` ownership + frame sequencing); no bench or on-wire conformance; timing is RB-26 |
| BIPv6 | None: compose [`EndpointSession::new`](session::EndpointSession::new) over a `Bip6Transport` | Real loopback UDP: a registered Network Port in NORMAL and FOREIGN mode (#1940) |
| Ethernet | None | Explicitly unsupported: no endpoint builder; use `bacnet-transport` directly |

Do not expect identical administration across data links: BBMD, SC hub
dial, and MS/TP station/timing knobs stay transport-specific by design.

## Ownership and lifecycle

- [`session::EndpointSession`] is the sole lifecycle owner:
  start-once/completed-stop-once; canceled stop can resume joining. `Drop`
  aborts owned tasks and the link, a live client or not (#2617), and retains
  Audit membership through their quiescence.
- The client ([`client`](session::EndpointSession::client)) is an owned
  `BACnetClient`. Its own `stop` stops it alone, never the link. Stopping or
  dropping the session ends its work; a client kept past that survives as a
  value and fails each call.
  The server is the session's, borrowed through `server()`.
- `start(&mut self)` / `stop(&mut self)` take `&mut` so only the owner can
  drive lifecycle; `&self` borrows (`client`, `server`, counters,
  `broadcast_i_am`) stay usable while running. Cancellation is
  await-boundary only: dropping a pending request's future cancels it and
  releases its invoke ID. Admitted audited
  requests are session-owned through their terminal outcome. `stop()` seals admission, cancels
  waiters, and joins dispatch exactly once.
- Ingress never waits for its queues and a full one never ends the session
  (#2635): what finds no room is dropped, counted by class in
  [`policy_counters`](session::EndpointSession::policy_counters) and logged
  at most every ten seconds. Unconfirmed requests may fill the request queue
  only down to a quarter of its
  [`queue_capacity`](session::SessionConfig::queue_capacity) (default 256),
  kept for confirmed requests, and answers no lease holds the answer queue
  only down to a quarter kept for answers to the session's own requests (a
  capacity of 1 keeps no reserve); a confirmed request with no room gets no
  answer and is recovered by its requester's retry; an answer with no room
  ends its request by timeout, as if the link had lost it.
- All fallible boundaries return typed [`bacnet_types::error::Error`];
  queue-capacity, BBMD-ordering, SC identity/heartbeat, and MS/TP
  addressing misconfiguration fail fast at build time. The builders'
  `build_session*` and the session's `start()` return [`StartError`],
  which `?` turns into that `Error`, and which hands back the object
  database (#2766): a builder's refusal the one it was given, as given, and
  a start that ends the session (any refusal once the link has started) the
  one it took. A refusal before the link starts leaves it with the session,
  ready to start again, which
  [`can_start`](session::EndpointSession::can_start) tells.

## Trust: verified-origin getters

Provenance stays on [`bacnet_transport::port::TransportProvenance`];
the endpoint preserves it structurally. Configured authorizers decide whether
that provenance permits an operation. Read it with the existing getters:

- `is_unverified()` — legacy origin, no assertion (B/IP, MS/TP, loopback).
- `is_direct_peer()` — accepted direct SC-TLS ingress.
- `direct_sc_identity()` — its sealed verified leaf/incarnation snapshot;
  claimed VMAC and routed addresses remain separate claims.
- `is_hub_channel()` — scope-only channel used by Hub admission callbacks.
- `is_relayed_origin()` — hub-validated relayed origin (post
  source-admission; the hub peer is never substituted for the leaf).
- `is_verified()` — any verified scope; it need not carry a direct identity.

Only trusted SC validation code constructs verified values; every other
transport (including caller-supplied doubles) reports unverified.

## Choosing standalone or shared client/server roles

Standalone `BACnetClient` and `BACnetServer` remain public APIs with their own
service and data-link capabilities; they are not deprecated. Use the endpoint
builders when both roles need one transport and lifecycle owner; the
endpoint's server role is the full server, read-only unless given a builder.

| Standalone construction | Shared endpoint composition |
|---------------------|---------------------|
| `BACnetClient::bip_builder()...build().await` | `BipEndpointBuilder::new(iface, port, bcast).role(ClientOnly).build_session()?` then `start()` |
| `BACnetServer::bip_builder()...build().await` | `BipEndpointBuilder::new(iface, port, bcast).role(ServerOnly).database(db).device(id).build_session()?` then `start()` |
| Client + server on one device | `BipEndpointBuilder::...role(Both).database(db).device(id).build_session()?` (or the SC / MS/TP builder) |
| `BACnetClient::sc_builder()...build().await` | `ScEndpointBuilder::new(vmac, uuid)...connect_hub_session(hubs).await?` (`sc-tls`; reconnect and failover as on the client), or `build_hub_session(ws)?` over a caller-dialed socket |
| `BACnetServer::sc_builder()...build().await` | Same `ScEndpointBuilder` with `ServerOnly` + `database` + `device` |
| `generic_builder().transport(mstp)...` | `MstpEndpointBuilder::new(serial, station)...build_session()?` (one serial owner, simulator evidence) |

Standalone BBMD helpers (`read_bdt` / `write_bdt` / `read_fdt` / foreign
registration) stay on [`BipTransport`](bacnet_transport::bip::BipTransport);
[`bip::BipEndpointBuilder::mode`] sets the endpoint's B/IP mode before start.
B/IPv6 has no endpoint builder: compose the session over a `Bip6Transport`
and register its port with
[`with_registered_network_port`](session::EndpointSession::with_registered_network_port).
Ethernet has no endpoint builder: keep the standalone path there.
See `docs/rust-api.md` for the same table with service-scope notes.

## Module map

- [`session`] — [`EndpointSession`],
  [`SessionRole`], [`SessionConfig`],
  [`PolicyCountersSnapshot`],
  [`SessionExit`].
- [`identity`] — [`DeviceIdentity`],
  [`NetworkPortEntry`],
  [`build_database_with_extra`].
- [`bip`], [`sc`], [`mstp`] — the three endpoint builders.

Hidden by design: `complete_pre_admitted` and all `__endpoint_*` hooks.
