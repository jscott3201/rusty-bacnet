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
loopback channels. The server holds an Analog Input with Present_Value 72.5.
The client reads it under a two-second timeout and checks the returned object,
property, array index and literal application-tagged REAL bytes. Both sessions
are stopped explicitly; no socket, retry or sleep is needed.

```rust
use bacnet_endpoint::{EndpointSession, SessionConfig, SessionRole};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::{timeout, Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client_port, server_port) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut analog = AnalogInputObject::new(7, "room-temperature", EngineeringUnits::DEGREES_FAHRENHEIT)?;
    analog.set_present_value(72.5);
    let mut database = ObjectDatabase::new();
    database.add(Box::new(analog))?;
    let config = SessionConfig {
        queue_capacity: 16,
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        max_apdu_length: 480,
        ..SessionConfig::default()
    };
    let mut client = EndpointSession::new(client_port, SessionRole::ClientOnly, config.clone())?;
    let mut server = EndpointSession::new(server_port, SessionRole::ServerOnly, config)?
        .with_database(database);
    client.start().await?;
    server.start().await?;

    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7)?;
    let handle = client.client().expect("the client role is enabled");
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
| `sc-tls` | Adds `ScEndpointBuilder::build_hub_session` over the constrained-TLS SC connection; it does not enable full standalone-server dispatch parity. |

The docs.rs profile selects `sc-tls` with default features disabled for
`x86_64-unknown-linux-gnu`. The hub-session method is labeled with its feature
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

## What is proven

- The client role initiates unsegmented ReadProperty, ReadRange and bounded
  explicit ReadPropertyMultiple (1–64 concrete-object references), plus
  [`ClientRoleHandle::write_property`] to direct B/IP IPv4 unicast peers.
  WriteProperty requires an explicit commandability classification and a valid
  encoded value that fits the unsegmented request. Direct
  B/IP source reporting emits one value-free READ record per eligible RP/RR
  operation or RPM occurrence, sharing request cancellation/retry and notification
  ownership. Source filters may suppress records.

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
- Narrow server scope: the endpoint server role executes `ReadProperty`
  (+ `Reject`/`Abort` + segmentation-`Abort`). Explicit
  [`EndpointSession::with_device_writes`](session::EndpointSession::with_device_writes)
  enables authorized writes to the one local Device's Description and its
  installed source Audit recipient, and explicit
  [`EndpointSession::with_reinitialize`](session::EndpointSession::with_reinitialize)
  enables ReinitializeDevice through an application handler, each with
  deterministic and real B/IP loopback tests. Full `bacnet-server`
  dispatch parity is out of scope. Accepted-direct replies preserve the original
  sealed response capability through the bounded endpoint queue, before any
  prompt reply channel. Retired/invalid authority never falls back by address;
  committed writes retain their admitting authorization. Saved direct link
  limits constrain response size; this responder uses its existing Abort when
  a ComplexACK cannot fit and does not add response segmentation.

## Data-link support matrix (explicit, not silent)

| Data link | Endpoint builder | Evidence |
|-----------|------------------|----------|
| B/IP IPv4 unicast + local broadcast | [`bip::BipEndpointBuilder`] | Real loopback UDP proof (RB-16) |
| B/IP BBMD / foreign-device mode | [`bip::BipEndpointBuilder::mode`] | The standalone builders' transport and modes; loopback Network Number wire coverage; [`session::EndpointSession::foreign_registration`] reports a registration; live BVLC queries stay on [`BipTransport`](bacnet_transport::bip::BipTransport) |
| BACnet/SC via hub | [`sc::ScEndpointBuilder`] | Real constrained-TLS hub proof (RB-16, `sc-tls`); loopback composition for unit validation |
| MS/TP | [`mstp::MstpEndpointBuilder`] | **Simulator evidence only** (`LoopbackSerial` ownership + frame sequencing); no bench or on-wire conformance; timing is RB-26 |
| BIPv6 | None | Explicitly unsupported: no endpoint builder; use `bacnet-transport` directly |
| Ethernet | None | Explicitly unsupported: no endpoint builder; use `bacnet-transport` directly |

Do not expect identical administration across data links: BBMD, SC hub
dial, and MS/TP station/timing knobs stay transport-specific by design.

## Ownership and lifecycle

- [`session::EndpointSession`] is the sole lifecycle owner:
  start-once/completed-stop-once; canceled stop can resume joining. `Drop`
  aborts owned tasks and retains Audit membership through their quiescence. Role handles
  expose no lifecycle methods.
- Role handles ([`ClientRoleHandle`], [`ServerRoleHandle`]) hold only a
  [`Weak`](std::sync::Weak) session token plus role state. Dropping or
  stopping the session ends role work even when a handle was cloned out
  beforehand; cloned handles survive the drop as values but fail closed.
- `start(&mut self)` / `stop(&mut self)` take `&mut` so only the owner can
  drive lifecycle; `&self` borrows (`client`, `server`, counters,
  `broadcast_i_am`) stay usable while running. Cancellation is
  await-boundary only: aborting a pending read future releases
  its exact coordinator lease via RAII for ordinary reads. Admitted audited
  reads are session-owned through their terminal outcome. `stop()` seals admission, cancels
  waiters, and joins dispatch exactly once.
- All fallible boundaries return typed [`bacnet_types::error::Error`];
  queue-capacity, BBMD-ordering, SC identity/heartbeat, and MS/TP
  addressing misconfiguration fail fast at build time.

```compile_fail,E0599
// Role handles expose no lifecycle: `stop` exists only on `EndpointSession`.
# use bacnet_endpoint::roles::ClientRoleHandle;
# fn forbidden(handle: ClientRoleHandle) {
handle.stop();
# }
```

```compile_fail,E0599
// `close_for_owner` is owner-only (`pub(crate)`): external callers cannot
// drive role shutdown directly.
# use bacnet_endpoint::roles::ServerRoleHandle;
# fn forbidden(handle: ServerRoleHandle) {
handle.close_for_owner();
# }
```

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
builders when both roles need one transport and lifecycle owner, accounting
for the narrower endpoint responder scope above.

| Standalone construction | Shared endpoint composition |
|---------------------|---------------------|
| `BACnetClient::bip_builder()...build().await` | `BipEndpointBuilder::new(iface, port, bcast).role(ClientOnly).build_session()?` then `start()` |
| `BACnetServer::bip_builder()...build().await` | `BipEndpointBuilder::new(iface, port, bcast).role(ServerOnly).database(db).device(id).build_session()?` then `start()` |
| Client + server on one device | `BipEndpointBuilder::...role(Both).database(db).device(id).build_session()?` (or the SC / MS/TP builder) |
| `BACnetClient::sc_builder()...build().await` | `ScEndpointBuilder::new(vmac, uuid)...build_hub_session(ws)?` (`sc-tls`; dial first, then compose) |
| `BACnetServer::sc_builder()...build().await` | Same `ScEndpointBuilder` with `ServerOnly` + `database` + `identity` |
| `generic_builder().transport(mstp)...` | `MstpEndpointBuilder::new(serial, station)...build_session()?` (one serial owner, simulator evidence) |

Standalone BBMD helpers (`read_bdt` / `write_bdt` / `read_fdt` / foreign
registration) stay on [`BipTransport`](bacnet_transport::bip::BipTransport);
[`bip::BipEndpointBuilder::mode`] sets the endpoint's B/IP mode before start.
BIPv6/Ethernet have no endpoint builder: keep the standalone path there.
See `docs/rust-api.md` for the same table with service-scope notes.

## Module map

- [`session`] — [`EndpointSession`],
  [`SessionRole`], [`SessionConfig`],
  [`PolicyCountersSnapshot`],
  [`SessionExit`].
- [`roles`] — [`ClientRoleHandle`] (`read_property`
  and `read_property_with_destination`, addressed by
  [`EndpointApduDestination`]) +
  [`ServerRoleHandle`] (inbound / liveness / suspend /
  notification admit-complete).
- [`identity`] — [`DeviceIdentity`],
  [`NetworkPortEntry`],
  [`build_database_with_extra`].
- [`bip`], [`sc`], [`mstp`] — the three endpoint builders.

Hidden by design: `SessionToken`, single-admit/decode helpers,
`complete_pre_admitted`, `close_for_owner`, and all `__endpoint_*` hooks.
