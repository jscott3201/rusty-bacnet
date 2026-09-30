# Architecture Guide

This document explains how the rusty-bacnet crates fit together, how data flows through the stack, and how the major subsystems work.

## Crate layout and selected dependencies

This layout groups responsibilities; it is not a complete Cargo dependency graph.
The arrows below mean “depends on.” Lower-level dependencies and feature edges
are omitted; each crate's `Cargo.toml` is the exact dependency authority.

```text
Foundations
  bacnet-types          Enums, primitives, error types (no I/O)
  bacnet-encoding       ASN.1 tags, APDU/NPDU codecs, value encoding
  bacnet-services       Service request/response structures
  bacnet-transport      Data-link transports and framing
  bacnet-network        Network layer, BACnetRouter, RouterTable
  bacnet-objects        BACnetObject, ObjectDatabase, object implementations

Runtime and application roles
  bacnet-endpoint-core  Private lifecycle, ingress, egress, coordination
  bacnet-client         Async requester, transactions, discovery
  bacnet-server         Full server dispatch, COV, events, scheduling
  bacnet-endpoint       Public shared owner composing bounded sibling roles

Selected direct dependencies
  bacnet-endpoint -> bacnet-client, bacnet-server, bacnet-endpoint-core,
                     bacnet-network, bacnet-objects
  bacnet-cli     -> bacnet-client              (CLI application)
  rusty-bacnet   -> bacnet-client, bacnet-server, bacnet-endpoint
                                              (PyO3 bindings)
```

`bacnet-client`, `bacnet-server` and `bacnet-endpoint` are all workspace
`default-members`, along with the foundational crates, endpoint-core, integration
tests and benchmarks. The CLI (`bacnet-cli`) and PyO3 binding (`rusty-bacnet`)
are excluded from default builds: the CLI pulls in heavier application
dependencies, and the Python extension needs its native Python build context.
They remain workspace members and can be selected explicitly.

## Packet Flow

### Inbound (receiving a BACnet request)

```
Physical network (UDP socket / WebSocket / serial port)
    |
    v
TransportPort::start() -> mpsc::Receiver<ReceivedNpdu>
    |  Decodes data-link framing (BVLL for BIP, BVLC-SC for SC, MS/TP frames)
    |  Extracts NPDU bytes + source MAC + raw link-layer group provenance + attributes
    v
NetworkLayer::start() -> mpsc::Receiver<ReceivedApdu>
    |  Decodes NPDU header (version, control, DNET/DADR/SNET/SADR)
    |  Local network controls -> owner control intake -> bounded Number worker
    |  Other raw controls retain their consumer (for example client Reject correlation)
    |  Filters: drops messages not for this device (wrong DNET)
    |  Extracts APDU bytes + source addressing + raw/effective group facts + attributes
    v
Optional private EndpointIngress classifier / Client dispatch task / Server dispatch task
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

Data attributes are carried on `ReceivedNpdu` and `ReceivedApdu`, and attribute-aware send helpers are available on `TransportPort` and `NetworkLayer`. `ReceivedApdu.link_layer_group` preserves whether the incoming data-link destination was a group address, while `ReceivedApdu.is_group` describes the effective BACnet network destination; a routed unicast can therefore have `link_layer_group == true` and `is_group == false`. `ReceivedApdu.global_broadcast` marks an NPDU addressed to the global broadcast network (DNET 65535), which `is_group` alone does not tell apart from a local or one-network broadcast. BACnet/SC maps every inbound Annex AB Data Option to these attributes and maps outbound attributes back to SC Data Options. Unknown Data Options do not block NPDU delivery. For received Encapsulated-NPDUs, an unsupported Must Understand Destination Option returns a BVLC-Result NAK with the original option marker to the unicast source through the hub and drops broadcast without a result. This message-level NAK does not close the source connection. A Destination Option without Must Understand does not block delivery. The current `DataAttribute.must_understand` field stores the Data Option bit-6 value, but Addendum 135-2020cf Every Segment behavior is not implemented. The router preserves inbound data attributes when forwarding unicast or broadcast NPDUs across attribute-capable transports, while data links that do not support attributes expose an empty list on receive and ignore attributes on send.

## Transport Abstraction

All transports implement the `TransportPort` trait:

```rust
pub trait TransportPort: Send + Sync {
    fn start(&mut self) -> impl Future<Output = Result<mpsc::Receiver<ReceivedNpdu>, Error>> + Send;
    fn stop(&mut self) -> impl Future<Output = Result<(), Error>> + Send;
    fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
    fn send_broadcast(&self, npdu: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
    fn local_mac(&self) -> &[u8];
    fn local_receive_apdu_capacity(&self) -> u16; // stable local declaration
    fn egress_apdu_limit(&self) -> u16; // current outgoing path limit
    fn supports_local_nonrouter_number_controls(&self) -> bool; // opt-in, default false
    fn normal_bip_endpoint(&self) -> Option<std::net::SocketAddrV4>; // registration metadata
}
```

`TransportPort` owns data-link framing and link-specific controls. `NetworkLayer` owns NPDU addressing and APDU delivery forms. The private `bacnet-endpoint-core` runtime can own one network lifecycle and expose bounded ingress and network-service egress to application-role adapters; those role handles cannot start or stop the network or transport. The public `bacnet-endpoint` crate composes sibling requester and bounded responder roles on that private foundation. One `EndpointSession` owns one B/IP, SC or MS/TP transport; this is not a multi-link router or full `bacnet-server` responder replacement. See [endpoint scope](rust-api.md#bacnet-endpoint).

Local nonrouter Number controls take a separate bounded path: one serial state owner per standalone client, full server or shared endpoint consumes eligible parsed controls without blocking independent APDU dispatch. Raw network controls remain available for other consumers, including routed Reject correlation. A client or unregistered owner starts UNKNOWN on an opted-in transport. Only an explicitly registered NORMAL B/IP receiving-port object supplies configured number authority; an unrelated database declaration cannot supply it. The capability is separate from registration metadata and from multiport/router behavior. See [Number controls and lifecycle](rust-api.md#local-network-number-controls).

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

`GpioDirectionPort` wraps a `SerialPort` with transmit-complete `drain()` support and controls a GPIO pin through the Linux character device API (`/dev/gpiochipN`). It asserts DE before writing, waits for drain, then applies any configured transceiver guard interval before returning to RX. Unix `TokioSerialPort` drains through the native serial backend on a blocking worker while retaining exclusive ownership of the stream. Ordinary hardware auto-direction and kernel RS-485 writes do not add userspace direction changes or drain waits.

MS/TP turnaround uses an absolute earliest-transmit deadline derived from the latest nonempty host read. Processing time counts toward that silence interval; a later chunk moves the deadline forward. Transmit encoding reuses a buffer and frame boundaries without copying each encoded frame into its own byte vector. These host-side guarantees are not physical UART timing qualification.

`MstpTransport::with_execution_mode(MstpExecutionMode::DedicatedThread)` opts into
an OS thread with a current-thread Tokio runtime for the MAC loop. The default
`Tokio` mode retains the application-runtime spawn path. Both modes run the same
master state machine, frame encoding and ordering, turnaround/deadline logic, and
64-entry NPDU receive channel. Serial wrappers retain their existing drain
boundary; native blocking drain jobs use the isolated runtime's blocking pool
when called from the dedicated loop. No per-frame bridge or second MAC machine
is introduced. Already-open async serial resources still depend on their original
reactor, which must remain running.

`stop()` cancels the MAC task, waits for isolated runtime teardown and outstanding
blocking work, then clears the transmit queue and returns the node to Idle.
`abort()` and drop request teardown without waiting. Cancellation cannot interrupt
a blocking syscall or undo bytes already accepted by a driver; a stuck backend
can delay shutdown. Fast or efficient execution is not guaranteed or measured
deterministic timing. Neither execution mode qualifies real hardware timing.

Deferred from this thread-isolation subset: RT scheduling policy/priority
(`SCHED_FIFO`), CPU affinity/pinning and observable RT setup results; PREEMPT_RT,
IRQ, mlock and buffer-tuning deployment guidance beyond this note; and on-wire
hardware qualification (#502). These RT APIs are not implemented here, and #501
remains open for its residual RT-policy/affinity and full documentation work.

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

**Lock ordering** (server): always lock `db` (ObjectDatabase) before `cov_table` (COV subscriptions). With target Audit configured, a DeviceCommunicationControl change and its timer's expiry take the DCC timer slot first and `db` second, to report the change under the slot (#1387); nothing takes the slot while holding `db`. Violating either order risks deadlock.

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
| Intrinsic reporting | Advances Time_Delay countdowns and fires confirmed transitions via `tick_intrinsic_reporting` | 1s |
| Event enrollment | Evaluates Event Enrollment objects against their monitored properties | 10s |
| Trend log | Records data samples for trend log objects | Per-object interval |
| Schedule tick | Evaluates Schedule objects (exceptions by period and priority, weekly schedule, Schedule_Default, within Effective_Period) against one Device clock frame and Calendar states, and writes changes at Priority_For_Writing; a write that commits to a Schedule runs its pass at once, which also sends on a Present_Value written while out of service and relinquishes slots a change of references or priority left behind; each target's result feeds the Schedule's Reliability | 60s |

The server handles 20+ services including ReadProperty, WriteProperty, ReadPropertyMultiple, WritePropertyMultiple, SubscribeCOV, CreateObject, DeleteObject, DeviceCommunicationControl, GetEventInformation, GetAlarmSummary, LifeSafetyOperation, AtomicReadFile, AtomicWriteFile, TimeSynchronization, and more.

ReinitializeDevice (Clause 16.4) is how a peer asks a device to restart, to
apply changes, or to step through a Clause 19 backup or restore. The server
decodes it and checks the password, then passes the requested state to the
handler set with `on_reinitialize`, which carries it out with the object
database write-locked; an error the handler returns is sent in place of the
SimpleACK. Without a handler, or for a state Clause 16.4 does not define, the
request is refused with `SERVICES / SERVICE_REQUEST_DENIED`. Password failures
and malformed-request errors keep their precedence.
