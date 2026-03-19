# Rust API Reference

Rusty BACnet is a workspace of 8 published crates implementing the BACnet protocol stack (ASHRAE 135-2020).

## Crate Dependency Order

```
bacnet-types → bacnet-encoding → bacnet-services → bacnet-transport → bacnet-network
                                                                          ↓
                                                    bacnet-objects → bacnet-client
                                                                          ↓
                                                                   bacnet-server
```

---

## bacnet-types

Core BACnet types, enums, and error definitions.

### Enums (`bacnet_enum!` macro)

All BACnet enums are generated with `bacnet_enum!`, which produces a newtype struct with:
- `from_raw(value)` / `to_raw()` — convert to/from raw integer
- `ALL_NAMED: &[(&str, Self)]` — named constant list for iteration
- `Display` / `Debug` / `PartialEq` / `Eq` / `Hash` / `Copy` / `Clone`

```rust
use bacnet_types::enums::*;

let ot = ObjectType::ANALOG_INPUT;
assert_eq!(ot.to_raw(), 0);
assert_eq!(ObjectType::from_raw(0), ot);
```

**Key enums:** `ObjectType` (u32), `PropertyIdentifier` (u32), `ErrorClass` (u16), `ErrorCode` (u16), `EnableDisable` (u32), `ReinitializedState` (u32), `Segmentation` (u8), `EventState` (u32), `EventType` (u32), `NotifyType` (u32), `Polarity` (u32), `Reliability` (u32), `LifeSafetyOperation` (u32), `MessagePriority` (u32)

### Primitives

```rust
use bacnet_types::primitives::*;

// Object Identifier (type + instance, max 4194303)
let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
assert_eq!(oid.object_type(), ObjectType::ANALOG_INPUT);
assert_eq!(oid.instance_number(), 1);

// Property Value — tagged union
let val = PropertyValue::Real(72.5);
let val = PropertyValue::Boolean(true);
let val = PropertyValue::CharacterString("hello".into());
let val = PropertyValue::Null;
```

### Error

```rust
use bacnet_types::error::Error;

// Protocol error from a remote device
let e = Error::Protocol { class: 2, code: 31 }; // ErrorClass(2)=PROPERTY, ErrorCode(31)=UNKNOWN_PROPERTY

// Other variants: Timeout, Reject, Abort, Io, Encoding, InvalidState, etc.
```

---

## bacnet-encoding

ASN.1/BER tag encoding, APDU/NPDU codecs, property value serialization, and segmentation.

### Property Value Encoding

```rust
use bacnet_encoding::primitives::{encode_property_value, decode_application_value};
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

// Encode
let mut buf = BytesMut::new();
encode_property_value(&mut buf, &PropertyValue::Real(72.5));
let bytes = buf.to_vec();

// Decode
let (value, bytes_consumed) = decode_application_value(&bytes, 0)?;
assert_eq!(value, PropertyValue::Real(72.5));
```

### APDU Types

```rust
use bacnet_encoding::apdu::*;

// Confirmed request, Complex ACK, Simple ACK, Error, Reject, Abort
// Segmentation: SegmentAck, segmented confirmed requests
```

### NPDU

```rust
use bacnet_encoding::npdu::{NpduHeader, encode_npdu, decode_npdu};

// Handles source/destination network addresses, hop count, priority
```

---

## bacnet-services

23 BACnet service modules with request/response encoding and decoding.

### ReadProperty / WriteProperty

```rust
use bacnet_services::rp::{ReadPropertyRequest, ReadPropertyACK};
use bacnet_services::wp::WritePropertyRequest;
```

### ReadPropertyMultiple / WritePropertyMultiple

```rust
use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleACK, ReadAccessResult};
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::common::{PropertyReference, BACnetPropertyValue};

let spec = ReadAccessSpecification {
    object_identifier: oid,
    list_of_property_references: vec![
        PropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        },
    ],
};
```

### COV

```rust
use bacnet_services::cov::{
    SubscribeCOVRequest, COVNotificationRequest, UnsubscribeCOVRequest,
    SubscribeCOVPropertyMultipleRequest,
};
```

### Discovery

```rust
use bacnet_services::who_is::{WhoIsRequest, IAmRequest};
use bacnet_services::who_has::{WhoHasRequest, WhoHasObject, IHaveRequest};
```

### Device Management

```rust
use bacnet_services::device_mgmt::{
    DeviceCommunicationControlRequest, ReinitializeDeviceRequest,
};
```

### Object Management

```rust
use bacnet_services::object_mgmt::{
    CreateObjectRequest, ObjectSpecifier, DeleteObjectRequest,
};
```

### File Services

```rust
use bacnet_services::file::{FileAccessMethod, FileWriteAccessMethod};
```

### ReadRange

```rust
use bacnet_services::read_range::{RangeSpec, ReadRangeAck};
```

### Alarm/Event

```rust
use bacnet_services::alarm_event::{
    AcknowledgeAlarmRequest, GetEventInformationRequest,
    GetAlarmSummaryRequest, GetEnrollmentSummaryRequest,
};
```

### List Manipulation

```rust
use bacnet_services::list_manipulation::{AddListElementRequest, RemoveListElementRequest};
```

### Private Transfer

```rust
use bacnet_services::private_transfer::{
    ConfirmedPrivateTransferRequest, UnconfirmedPrivateTransferRequest,
};
```

### Text Message

```rust
use bacnet_services::text_message::{
    ConfirmedTextMessageRequest, UnconfirmedTextMessageRequest,
};
```

### Life Safety

```rust
use bacnet_services::life_safety::LifeSafetyOperationRequest;
```

### Write Group

```rust
use bacnet_services::write_group::WriteGroupRequest;
```

### Virtual Terminal

```rust
use bacnet_services::vt::{VtOpenRequest, VtCloseRequest, VtDataRequest};
```

### Audit

```rust
use bacnet_services::audit::{
    ConfirmedAuditNotificationRequest, UnconfirmedAuditNotificationRequest,
    AuditLogQueryRequest,
};
```

---

## bacnet-transport

Transport-layer implementations. All implement the `TransportPort` trait.

### Feature Flags

| Feature | Platforms | Transport |
|---------|-----------|-----------|
| (default) | all | BIP (UDP/IPv4) |
| `ipv6` | all | BIP6 (UDP/IPv6 multicast) |
| `sc-tls` | all | BACnet/SC (WebSocket + TLS) + SC Hub |
| `serial` | Linux | MS/TP (serial token-passing) |
| `ethernet` | Linux | BACnet Ethernet (BPF) |

### BIP (IPv4)

```rust
use bacnet_transport::bip::BipTransport;

let transport = BipTransport::new(
    Ipv4Addr::new(0, 0, 0, 0),  // bind interface
    0xBAC0,                       // port (47808)
    Ipv4Addr::BROADCAST,          // broadcast address
);
```

### BIP6 (IPv6)

```rust
use bacnet_transport::bip6::Bip6Transport;

let transport = Bip6Transport::new(
    Ipv6Addr::UNSPECIFIED,  // bind interface
    0xBAC0,                 // port
    None,                   // device_instance (auto VMAC)
);
// 3-byte VMAC, 3 multicast scopes, collision detection
```

### BACnet/SC (Client Transport)

```rust
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_tls::TlsWebSocket;

let ws = TlsWebSocket::connect("wss://hub:1234", tls_config).await?;
let transport = ScTransport::new(ws, vmac)
    .with_heartbeat_interval_ms(30_000)
    .with_heartbeat_timeout_ms(60_000);
```

### BACnet/SC Hub

```rust
use bacnet_transport::sc_hub::ScHub;

let hub = ScHub::new(listen_addr, tls_acceptor, hub_vmac);
let addr = hub.start().await?;  // Returns SocketAddr
// hub.stop().await;
```

The SC hub is a TLS WebSocket relay. Both clients and servers connect to it as spoke nodes. Messages are routed by VMAC address.

### Loopback Transport

```rust
use bacnet_transport::loopback::LoopbackTransport;

let (side_a, side_b) = LoopbackTransport::pair(
    vec![0x00, 0x01],  // MAC for side A
    vec![0x00, 0x02],  // MAC for side B
);
```

In-process channel-based transport for composing a gateway's client and server without real network sockets. `LoopbackTransport::pair()` creates two connected transports backed by `tokio::sync::mpsc` channels — sending on one delivers to the other. Available as `AnyTransport::Loopback` for use with the enum dispatch wrapper.

### AnyTransport (enum dispatch)

```rust
use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::NoSerial; // placeholder when serial feature is off

let transport: AnyTransport<NoSerial> = AnyTransport::Bip(bip_transport);
```

Variants: `Bip`, `Bip6`, `Mstp`, `Sc` (boxed), `Loopback`.

### BBMD

```rust
use bacnet_transport::bbmd::BbmdConfig;

// BBMD with broadcast distribution table + foreign device table
// management_acl gates Write-BDT/Delete-FDT (empty = allow all)
```

---

## bacnet-network

Network layer routing, router tables, and the multi-port router.

```rust
use bacnet_network::network_layer::NetworkLayer;
use bacnet_network::router::BACnetRouter;
```

---

## bacnet-objects

BACnet object model: trait, database, and 65 object type implementations.

### BACnetObject Trait

```rust
use bacnet_objects::traits::BACnetObject;

// Every object type implements:
trait BACnetObject {
    fn object_identifier(&self) -> ObjectIdentifier;
    fn object_name(&self) -> &str;
    fn object_type(&self) -> ObjectType;
    fn read_property(&self, property: PropertyIdentifier, array_index: Option<u32>)
        -> Result<PropertyValue, Error>;
    fn write_property(&mut self, property: PropertyIdentifier, array_index: Option<u32>,
        value: PropertyValue, priority: Option<u8>) -> Result<(), Error>;
    fn property_list(&self) -> Vec<PropertyIdentifier>;
}
```

### ObjectDatabase

```rust
use bacnet_objects::database::ObjectDatabase;

let mut db = ObjectDatabase::new();
db.add(Box::new(analog_input));

let obj = db.get(&oid);                // Option<&dyn BACnetObject>
let obj = db.get_mut(&oid);            // Option<&mut Box<dyn BACnetObject>>
```

### Object Types (64)

#### Core I/O (9)

| Type | Constructor |
|------|-------------|
| `AnalogInputObject` | `::new(instance, name, units)` |
| `AnalogOutputObject` | `::new(instance, name, units)` |
| `AnalogValueObject` | `::new(instance, name, units)` |
| `BinaryInputObject` | `::new(instance, name)` |
| `BinaryOutputObject` | `::new(instance, name)` |
| `BinaryValueObject` | `::new(instance, name)` |
| `MultiStateInputObject` | `::new(instance, name, number_of_states)` |
| `MultiStateOutputObject` | `::new(instance, name, number_of_states)` |
| `MultiStateValueObject` | `::new(instance, name, number_of_states)` |

#### Schedule & Notification (6)

| Type | Constructor |
|------|-------------|
| `CalendarObject` | `::new(instance, name)` |
| `ScheduleObject` | `::new(instance, name, default_value)` |
| `NotificationClass` | `::new(instance, name)` |
| `NotificationForwarderObject` | `::new(instance, name)` |
| `AlertEnrollmentObject` | `::new(instance, name)` |
| `EventEnrollmentObject` | `::new(instance, name, event_type)` |

#### Logging & Trending (5)

| Type | Constructor |
|------|-------------|
| `TrendLogObject` | `::new(instance, name, buffer_size)` |
| `TrendLogMultipleObject` | `::new(instance, name, buffer_size)` |
| `EventLogObject` | `::new(instance, name, buffer_size)` |
| `AuditLogObject` | `::new(instance, name, buffer_size)` |
| `AuditReporterObject` | `::new(instance, name)` |

#### Building Control (8)

| Type | Constructor |
|------|-------------|
| `LoopObject` | `::new(instance, name, output_units)` |
| `CommandObject` | `::new(instance, name)` |
| `TimerObject` | `::new(instance, name)` |
| `LoadControlObject` | `::new(instance, name)` |
| `ProgramObject` | `::new(instance, name)` |
| `AveragingObject` | `::new(instance, name)` |
| `ChannelObject` | `::new(instance, name, channel_number)` |
| `StagingObject` | `::new(instance, name, num_stages)` |

#### Lighting & Color (4)

| Type | Constructor |
|------|-------------|
| `LightingOutputObject` | `::new(instance, name)` |
| `BinaryLightingOutputObject` | `::new(instance, name)` |
| `ColorObject` | `::new(instance, name)` |
| `ColorTemperatureObject` | `::new(instance, name)` |

#### Life Safety (2)

| Type | Constructor |
|------|-------------|
| `LifeSafetyPointObject` | `::new(instance, name)` |
| `LifeSafetyZoneObject` | `::new(instance, name)` |

#### Access Control (7)

| Type | Constructor |
|------|-------------|
| `AccessDoorObject` | `::new(instance, name)` |
| `AccessPointObject` | `::new(instance, name)` |
| `AccessCredentialObject` | `::new(instance, name)` |
| `AccessUserObject` | `::new(instance, name)` |
| `AccessRightsObject` | `::new(instance, name)` |
| `AccessZoneObject` | `::new(instance, name)` |
| `CredentialDataInputObject` | `::new(instance, name)` |

#### Transportation (3)

| Type | Constructor |
|------|-------------|
| `ElevatorGroupObject` | `::new(instance, name)` |
| `EscalatorObject` | `::new(instance, name)` |
| `LiftObject` | `::new(instance, name, num_floors)` |

#### Groups & Views (3)

| Type | Constructor |
|------|-------------|
| `GroupObject` | `::new(instance, name)` |
| `GlobalGroupObject` | `::new(instance, name)` |
| `StructuredViewObject` | `::new(instance, name)` |

#### Measurement (2)

| Type | Constructor |
|------|-------------|
| `AccumulatorObject` | `::new(instance, name, units)` |
| `PulseConverterObject` | `::new(instance, name, units)` |

#### System (3)

| Type | Constructor |
|------|-------------|
| `DeviceObject` | `::new(DeviceConfig { .. })` |
| `FileObject` | `::new(instance, name, file_type)` |
| `NetworkPortObject` | `::new(instance, name, network_type)` |

#### Extended Value Types (12)

| Type | Constructor |
|------|-------------|
| `IntegerValueObject` | `::new(instance, name)` |
| `PositiveIntegerValueObject` | `::new(instance, name)` |
| `LargeAnalogValueObject` | `::new(instance, name)` |
| `CharacterStringValueObject` | `::new(instance, name)` |
| `OctetStringValueObject` | `::new(instance, name)` |
| `BitStringValueObject` | `::new(instance, name)` |
| `DateValueObject` | `::new(instance, name)` |
| `TimeValueObject` | `::new(instance, name)` |
| `DateTimeValueObject` | `::new(instance, name)` |
| `DatePatternValueObject` | `::new(instance, name)` |
| `TimePatternValueObject` | `::new(instance, name)` |
| `DateTimePatternValueObject` | `::new(instance, name)` |

---

## bacnet-client

Async BACnet client with transaction state machine, segmentation, and discovery.

### Building a Client

```rust
use bacnet_client::client::BACnetClient;

// Generic builder — accepts any pre-built TransportPort
let client = BACnetClient::generic_builder()
    .transport(transport)
    .apdu_timeout_ms(6000)
    .build()
    .await?;

// BIP-specific builder — constructs BipTransport from interface/port/broadcast
let client = BACnetClient::bip_builder()
    .interface(Ipv4Addr::UNSPECIFIED)
    .port(0)
    .broadcast_address(Ipv4Addr::BROADCAST)
    .build()
    .await?;

// SC-specific builder (requires `sc-tls` feature)
let client = BACnetClient::sc_builder()
    .hub_url("wss://hub:1234")
    .tls_config(tls_config)
    .vmac([0, 1, 2, 3, 4, 5])
    .build()
    .await?;
```

`BACnetClient::builder()` is an alias for `bip_builder()`.

### Property Access

```rust
// ReadProperty
let ack = client.read_property(&mac, oid, PropertyIdentifier::PRESENT_VALUE, None).await?;
let (value, _) = decode_application_value(&ack.property_value, 0)?;

// WriteProperty
let mut buf = BytesMut::new();
encode_property_value(&mut buf, &PropertyValue::Real(72.5));
client.write_property(&mac, oid, PropertyIdentifier::PRESENT_VALUE, None, buf.to_vec(), Some(8)).await?;

// ReadPropertyMultiple
let specs = vec![ReadAccessSpecification { object_identifier: oid, list_of_property_references: refs }];
let ack = client.read_property_multiple(&mac, specs).await?;

// WritePropertyMultiple
let specs = vec![WriteAccessSpecification { object_identifier: oid, list_of_properties: props }];
client.write_property_multiple(&mac, specs).await?;
```

### COV Subscriptions

```rust
// Subscribe
client.subscribe_cov(&mac, process_id, oid, true, Some(300)).await?;

// Subscribe to multiple properties at once
client.subscribe_cov_property_multiple(&mac, process_id, specs, Some(10), Some(true)).await?;

// Receive notifications (broadcast channel — multiple consumers OK)
let mut rx = client.cov_notifications();
let notification: COVNotificationRequest = rx.recv().await?;

// Unsubscribe
client.unsubscribe_cov(&mac, process_id, oid).await?;
```

### Discovery

```rust
client.who_is(None, None).await?;                       // broadcast
client.who_has(WhoHasObject::Name("Zone Temp".into()), None, None).await?;
client.who_am_i().await?;                               // network path verification

let devices = client.discovered_devices().await;         // Vec<DiscoveredDevice>
let device = client.get_device(1234).await;              // Option<DiscoveredDevice>
client.clear_devices().await;                            // reset table
```

### Device Management

```rust
client.device_communication_control(&mac, EnableDisable::DISABLE, Some(60), Some("password".into())).await?;
client.reinitialize_device(&mac, ReinitializedState::WARMSTART, None).await?;
```

### Object Management

```rust
client.create_object(&mac, ObjectSpecifier::Type(ObjectType::ANALOG_INPUT), initial_values).await?;
client.delete_object(&mac, oid).await?;
```

### Alarms & Events

```rust
client.acknowledge_alarm(&mac, process_id, oid, event_state, "operator").await?;
let raw = client.get_event_information(&mac, None).await?;
let raw = client.get_alarm_summary(&mac).await?;
let raw = client.get_enrollment_summary(&mac, ack_filter, event_state, event_type, min_pri, max_pri, notif_class).await?;
```

### Life Safety

```rust
client.life_safety_operation(&mac, process_id, "operator", LifeSafetyOperation::SILENCE, Some(oid)).await?;
```

### File Services

```rust
let raw = client.atomic_read_file(&mac, file_oid, FileAccessMethod::Stream { file_start_position: 0, requested_octet_count: 1024 }).await?;
client.atomic_write_file(&mac, file_oid, FileWriteAccessMethod::Stream { file_start_position: 0, file_data: data }).await?;
```

### ReadRange

```rust
let ack = client.read_range(&mac, oid, PropertyIdentifier::LOG_BUFFER, None, Some(RangeSpec::ByPosition { reference_index: 1, count: 10 })).await?;
```

### List Manipulation

```rust
client.add_list_element(&mac, oid, PropertyIdentifier::OBJECT_LIST, None, element_bytes).await?;
client.remove_list_element(&mac, oid, PropertyIdentifier::OBJECT_LIST, None, element_bytes).await?;
```

### Private Transfer

```rust
let raw = client.confirmed_private_transfer(&mac, vendor_id, service_number, Some(params)).await?;
client.unconfirmed_private_transfer(&mac, vendor_id, service_number, Some(params)).await?;
```

### Text Messages

```rust
let raw = client.confirmed_text_message(&mac, device_oid, priority, "Fire alarm", class_type, class_value).await?;
client.unconfirmed_text_message(&mac, device_oid, priority, "Status update", None, None).await?;
```

### Write Group

```rust
client.write_group(&mac, group_number, write_priority, change_list, Some(false)).await?;
```

### Virtual Terminal

```rust
let raw = client.vt_open(&mac, vt_class).await?;
client.vt_close(&mac, &session_ids).await?;
let raw = client.vt_data(&mac, session_id, &data, data_flag).await?;
```

### Audit Services

```rust
let raw = client.confirmed_audit_notification(&mac, service_data).await?;
client.unconfirmed_audit_notification(&mac, service_data).await?;
let raw = client.audit_log_query(&mac, ack_filter, query_options).await?;
```

---

## bacnet-server

Async BACnet server that hosts objects and dispatches incoming requests.

### Building a Server

```rust
use bacnet_server::server::BACnetServer;

// Generic builder — accepts any pre-built TransportPort
let server = BACnetServer::generic_builder()
    .database(db)
    .transport(transport)
    .build()
    .await?;

// BIP-specific builder — constructs BipTransport from interface/port/broadcast
let server = BACnetServer::bip_builder()
    .database(db)
    .interface(Ipv4Addr::UNSPECIFIED)
    .port(0xBAC0)
    .broadcast_address(Ipv4Addr::BROADCAST)
    .build()
    .await?;

// SC-specific builder (requires `sc-tls` feature)
let server = BACnetServer::sc_builder()
    .database(db)
    .hub_url("wss://hub:1234")
    .tls_config(tls_config)
    .vmac([0, 1, 2, 3, 4, 5])
    .build()
    .await?;

// Access the database at runtime
let db = server.database().lock().await;
let value = db.get(&oid).unwrap().read_property(pid, None)?;

// Check communication state
let state = server.comm_state(); // 0=Enable, 1=Disable, 2=DisableInitiation

// Stop
server.stop().await?;
```

`BACnetServer::builder()` is an alias for `bip_builder()`.

### Handled Services

The server automatically dispatches:

**Confirmed:**
- ReadProperty, WriteProperty
- ReadPropertyMultiple, WritePropertyMultiple
- SubscribeCOV, SubscribeCOVProperty, SubscribeCOVPropertyMultiple
- CreateObject, DeleteObject
- DeviceCommunicationControl, ReinitializeDevice
- GetEventInformation, AcknowledgeAlarm
- GetAlarmSummary, GetEnrollmentSummary
- ConfirmedTextMessage
- LifeSafetyOperation
- ReadRange
- AtomicReadFile, AtomicWriteFile
- AddListElement, RemoveListElement

**Unconfirmed:**
- WhoIs / IAm
- WhoHas / IHave
- TimeSynchronization, UTCTimeSynchronization
- WriteGroup
- UnconfirmedTextMessage

**Outgoing (server-initiated):**
- COV notifications (confirmed and unconfirmed, with ServerTsm retry for confirmed)
- Event notifications (confirmed and unconfirmed, routed via NotificationClass recipients)

### Concurrency

- Lock ordering: always `db` before `cov_table`
- `seg_receivers` capped at 128 (DoS prevention)
- `cov_in_flight` semaphore: max 255 concurrent confirmed COV notifications
- `comm_state`: `Arc<AtomicU8>` — lock-free read

---

## bacnet-gateway

BACnet HTTP REST API and MCP (Model Context Protocol) server gateway. Bridges BACnet networks to web clients and AI tools.

### Feature Flags

| Feature | Description |
|---------|-------------|
| `http` | Axum-based REST API (read/write properties, discover devices) |
| `mcp` | MCP server for AI tool integration (via `rmcp`) |
| `bin` | Binary target with CLI (`clap`), enables both `http` and `mcp` |
| `sc-tls` | BACnet/SC transport support |
| `serial` | MS/TP transport support (Linux only) |

See `docs/gateway.md` for full REST API and MCP tool documentation.

---

## Error Handling

All async operations return `Result<T, bacnet_types::error::Error>`. Key variants:

| Variant | Meaning |
|---------|---------|
| `Error::Protocol { class, code }` | Remote BACnet error response |
| `Error::Timeout(msg)` | APDU retry exhausted |
| `Error::Reject { reason }` | Remote device rejected request |
| `Error::Abort { reason }` | Remote device aborted request |
| `Error::Encoding(msg)` | Malformed packet |
| `Error::Io(io_error)` | Transport I/O failure |

---

## Transport Configuration Examples

### BIP Client + Server

```rust
use bacnet_client::client::BACnetClient;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use std::net::Ipv4Addr;

// Client
let client = BACnetClient::bip_builder()
    .interface(Ipv4Addr::UNSPECIFIED)
    .port(0)
    .broadcast_address(Ipv4Addr::BROADCAST)
    .build()
    .await?;

// Server
let server = BACnetServer::bip_builder()
    .database(db)
    .interface(Ipv4Addr::UNSPECIFIED)
    .port(0xBAC0)
    .broadcast_address(Ipv4Addr::BROADCAST)
    .build()
    .await?;
```

### BIP6 (IPv6)

```rust
use bacnet_transport::bip6::Bip6Transport;
use std::net::Ipv6Addr;

let transport = Bip6Transport::new(Ipv6Addr::UNSPECIFIED, 0xBAC0, None);
let client = BACnetClient::generic_builder().transport(transport).build().await?;
```

### BACnet/SC with Hub

```rust
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_hub::ScHub;
use bacnet_transport::sc_tls::{TlsWebSocket, build_tls_config};

// Start hub
let hub = ScHub::new(listen_addr, tls_acceptor, [0xFF, 0, 0, 0, 0, 1]);
let hub_addr = hub.start().await?;

// Connect client to hub
let client = BACnetClient::sc_builder()
    .hub_url(&format!("wss://127.0.0.1:{}", hub_addr.port()))
    .tls_config(tls_config)
    .vmac([0, 1, 2, 3, 4, 5])
    .build()
    .await?;
```
