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
    +---> bacnet-cli          Interactive shell and CLI tool
    |
    +---> rusty-bacnet        Python bindings (PyO3)
          bacnet-wasm         WASM/JS BACnet/SC thin client
```

The bottom rows are "application" crates — they compose the library crates into user-facing tools. They are excluded from `default-members` in the workspace to avoid pulling in their heavy dependencies (clap, pyo3, wasm-bindgen) during normal development.

The HTTP/MCP gateway and BTL compliance test harness now live in dedicated repositories:
- [`rusty-bacnet-mcp`](https://github.com/jscott3201/rusty-bacnet-mcp) — Axum REST API + rmcp MCP server
- [`rusty-bacnet-btl-harness`](https://github.com/jscott3201/rusty-bacnet-btl-harness) — BTL Test Plan 26.1 compliance harness

Both consume the published `bacnet-*` crates from this workspace.

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

### RS-485 Direction Control (MS/TP)

RS-485 is half-duplex — the transceiver's DE/RE pin must be toggled between transmit and receive. The stack supports three modes:

```
                              ┌──────────────────────────┐
USB RS-485 Adapter ──────────>│  TokioSerialPort         │  Auto-direction
(FTDI, CH340, etc.)           │  (no config needed)      │  (hardware handles DE/RE)
                              └──────────────────────────┘

UART + RTS → DE/RE ──────────>│  TokioSerialPort         │  Kernel RS-485
(DE wired to UART RTS pin)    │  .enable_kernel_rs485()  │  (TIOCSRS485 ioctl)
                              └──────────────────────────┘

UART + GPIO → DE/RE ─────────>│  GpioDirectionPort<S>    │  GPIO direction
(Pi hat, GPIO pin for DE)     │  wraps any SerialPort    │  (gpiocdev, serial-gpio feature)
                              └──────────────────────────┘
```

`GpioDirectionPort` is a composable wrapper — it wraps any `SerialPort` and toggles a GPIO pin via the Linux character device API (`/dev/gpiochipN`) before and after each write. This keeps `TokioSerialPort` simple and platform-independent.

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

## Companion projects

The HTTP/MCP gateway and BTL compliance test harness live in separate repositories that consume this workspace's published crates:

- **[`rusty-bacnet-mcp`](https://github.com/jscott3201/rusty-bacnet-mcp)** — HTTP REST API (Axum) and MCP server (rmcp) on top of `BACnetClient` + `BACnetServer`. Single shared `GatewayState` handles both surfaces — no duplicated BACnet logic.
- **[`rusty-bacnet-btl-harness`](https://github.com/jscott3201/rusty-bacnet-btl-harness)** — BTL Test Plan 26.1 compliance test harness. 3,808 tests across all 13 BTL sections; in-process self-test runs in <1 second.
