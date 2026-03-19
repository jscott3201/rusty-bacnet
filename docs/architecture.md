# Architecture Guide

This document explains how the rusty-bacnet crates fit together, how data flows through the stack, and how the major subsystems work.

## Crate Dependency Graph

```
bacnet-types          Enums, primitives, error types (no I/O)
    |
bacnet-encoding       ASN.1 tags, APDU/NPDU codec, property value encode/decode
    |
bacnet-services       Service request/response structs (RP, WP, RPM, COV, etc.)
    |
    +---> bacnet-transport    Data-link transports (BIP, SC, MS/TP, Ethernet, Loopback)
    |         |
    |     bacnet-network      Network layer, BACnetRouter, RouterTable
    |         |
    +---> bacnet-objects      BACnetObject trait, ObjectDatabase, 65 object types
    |         |
    |     bacnet-client       Async BACnet client (TSM, segmentation, discovery)
    |     bacnet-server       Async BACnet server (dispatch, COV, events, scheduling)
    |         |
    +---> bacnet-gateway      HTTP REST API + MCP server (optional, feature-gated)
    |     bacnet-cli          Interactive shell and CLI tool
    |     bacnet-btl          BTL compliance test harness (3,808 tests)
    |
    +---> rusty-bacnet        Python bindings (PyO3)
          bacnet-wasm         WASM/JS BACnet/SC thin client
          bacnet-java         Kotlin/Java bindings (UniFFI)
```

The bottom three rows are "application" crates — they compose the library crates into user-facing tools. They are excluded from `default-members` in the workspace to avoid pulling in their heavy dependencies (clap, axum, rmcp, pyo3, wasm-bindgen, uniffi) during normal development.

## Packet Flow

### Inbound (receiving a BACnet request)

```
Physical network (UDP socket / WebSocket / serial port)
    |
    v
TransportPort::start() -> mpsc::Receiver<ReceivedNpdu>
    |  Decodes data-link framing (BVLL for BIP, BVLC-SC for SC, MS/TP frames)
    |  Extracts NPDU bytes + source MAC address
    v
NetworkLayer::start() -> mpsc::Receiver<ReceivedApdu>
    |  Decodes NPDU header (version, control, DNET/DADR/SNET/SADR)
    |  Filters: drops messages not for this device (wrong DNET)
    |  Extracts APDU bytes + source network/address info
    v
Client dispatch task / Server dispatch task
    |  Decodes APDU header (PDU type, service choice, invoke ID)
    |  Routes to appropriate handler
    v
Service handler (e.g., handle_read_property)
    |  Decodes service request from APDU payload
    |  Reads/writes ObjectDatabase
    |  Encodes response
    v
NetworkLayer::send_apdu() -> TransportPort::send_unicast()
    |  Encodes NPDU header + APDU payload
    |  Sends via transport
    v
Physical network
```

### Multi-network routing

When `BACnetRouter` is used (multi-transport gateway):

```
Transport A (BIP, network 1)  ─┐
Transport B (SC, network 2)   ─┤──> BACnetRouter
Transport C (MS/TP, network 3) ─┤      |
Loopback (local client/server) ─┘      |
                                        v
                                  RouterTable lookup
                                        |
                                  Forward NPDU to correct transport
```

The router receives NPDUs from all transports, checks the destination network number in the NPDU header, and forwards to the appropriate transport. Messages for the local device (DNET matches a loopback port) are delivered to the client/server.

## Transport Abstraction

All transports implement the `TransportPort` trait:

```rust
pub trait TransportPort: Send + Sync {
    fn start(&mut self) -> impl Future<Output = Result<mpsc::Receiver<ReceivedNpdu>, Error>> + Send;
    fn stop(&mut self) -> impl Future<Output = Result<(), Error>> + Send;
    fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
    fn send_broadcast(&self, npdu: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
    fn local_mac(&self) -> &[u8];
    fn max_apdu_length(&self) -> u16;  // BIP/SC: 1476, MS/TP: 480
}
```

MAC address format varies by transport:
- **BIP**: 6 bytes (4-byte IPv4 + 2-byte port, big-endian)
- **BIP6**: 18 bytes (16-byte IPv6 + 2-byte port)
- **MS/TP**: 1 byte (station address 0-254)
- **BACnet/SC**: 6 bytes (VMAC)
- **Ethernet**: 6 bytes (IEEE 802 MAC)
- **Loopback**: arbitrary (synthetic, e.g., `[0x00, 0x01]`)

`AnyTransport<S>` is a type-erased enum wrapping all transport types, enabling mixed-transport routing (e.g., BIP + MS/TP + Loopback on the same router).

## Object Model

Every BACnet object implements the `BACnetObject` trait:

```rust
pub trait BACnetObject: Send + Sync {
    fn object_identifier(&self) -> ObjectIdentifier;
    fn object_name(&self) -> &str;
    fn read_property(&self, property: PropertyIdentifier, array_index: Option<u32>) -> Result<PropertyValue, Error>;
    fn write_property(&mut self, property: PropertyIdentifier, array_index: Option<u32>, value: PropertyValue, priority: Option<u8>) -> Result<(), Error>;
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]>;
    // ... plus COV, intrinsic reporting, scheduling methods
}
```

`ObjectDatabase` stores `Box<dyn BACnetObject>` keyed by `ObjectIdentifier`, with secondary indexes by name (for WhoHas) and by type (for efficient enumeration).

## Concurrency Model

The stack runs on a Tokio multi-threaded runtime.

**Lock ordering** (server): always lock `db` (ObjectDatabase) before `cov_table` (COV subscriptions). Violating this order risks deadlock.

**Resource exhaustion caps**:
- COV subscriptions: 1,024 max
- BBMD FDT entries: 512
- Objects per database: 10,000
- Router table entries: 256
- Segment receivers: 128 (prevents DoS from abandoned segmented transfers)
- COV in-flight: 255 (matches u8 invoke ID range)
- MS/TP frame buffer: 1,507 bytes
- MS/TP queue: 256 pending NPDUs

**Client APDU retry**: 3 retries by default, invoke ID reused across retries, cleaned up on final timeout.

## Server Engine

The `BACnetServer` spawns several background tasks:

| Task | Purpose | Interval |
|------|---------|----------|
| Dispatch | Receives APDUs, routes to service handlers | Event-driven |
| COV purge | Removes expired COV subscriptions | 60s |
| Fault detection | Evaluates analog objects for over/under-range | 10s |
| Event enrollment | Processes intrinsic reporting algorithms | 10s |
| Trend log | Records data samples for trend log objects | Per-object interval |
| Schedule tick | Evaluates weekly schedules and exception dates | 60s |

The server handles 20+ services including ReadProperty, WriteProperty, ReadPropertyMultiple, WritePropertyMultiple, SubscribeCOV, CreateObject, DeleteObject, DeviceCommunicationControl, ReinitializeDevice, GetEventInformation, GetAlarmSummary, LifeSafetyOperation, AtomicReadFile, AtomicWriteFile, TimeSynchronization, and more.

## Gateway Architecture

The `bacnet-gateway` crate adds HTTP and MCP interfaces on top of the core stack. It compiles **zero** web dependencies by default — the `http` and `mcp` features must be explicitly enabled.

```
┌────────────────────────────────┐
│  HTTP REST API  │  MCP Server  │  (feature-gated: http, mcp)
│  (Axum)         │  (rmcp)      │
└───────┬─────────┴──────┬───────┘
        │                │
        v                v
   GatewayState (shared)
        │
        v
   BACnetClient + BACnetServer
        │
        v
   BIP Transport (UDP)
```

`GatewayState` holds Arc references to the client and shared object database. Both REST handlers and MCP tools call the same methods on `GatewayState` — no duplicated BACnet logic.

See [Gateway Reference](gateway.md) for full configuration, endpoint, and tool documentation.
