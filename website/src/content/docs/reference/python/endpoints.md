---
title: "Python shared endpoints"
description: "The B/IP, SC and MS/TP endpoints and their client and server roles, generated from the type stub."
---

:::note[On `dev` after the release]
This page is generated from the type stub on the `dev` branch and tracks the next release, so it can list classes, methods and keywords merged after v0.15.0 that the released wheel doesn't have. The [v0.15.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) and the [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d) say what changed.
:::

One transport that runs both the client and server roles. The [Python reference](/rusty-bacnet/reference/python/) lists every group. Each page opens with its main classes, then the other classes, the dictionaries (`TypedDict`) and the type aliases, in stub order.

<!-- generated:start by website/scripts/reference-python.py; do not edit this page -->

## BipEndpoint

B/IP endpoint: one UDP socket that both initiates and executes.

Two separately constructed objects (`BACnetClient` + `BACnetServer`)
are two connections (two sockets/ports). The endpoint is the
one-transport path: one socket serves both roles.

Admission is lifecycle-lock acquisition, not Future creation. Explicit
second start raises BacnetError; context reentry returns this endpoint.
Close cancels pending connection preparation, then joins earlier admitted
startup and teardown. A later start may restart without old registrations.
Cancelled waiters do not abandon admitted session startup or cleanup;
cancellation can race publication, so always await close for cleanup.
Setup errors are delivered by awaiting start/context entry. BIPv6/Ethernet
have no endpoint owner.

`device_instance` takes 0 to 4194302. Instance 4194303 is reserved, as
no object may have it, so it, or anything larger, raises ValueError at
construction (#2171).

Properties:

- `device_instance: int`
- `vendor_id: int`

### BipEndpoint.__init__

```python
def __init__(
    self,
    device_instance: int,
    device_name: str = "BACnet Device",
    vendor_id: int = 555,
    interface: str = "0.0.0.0",
    port: int = 0xBAC0,
    broadcast_address: str = "255.255.255.255",
    network_number: int = 0,
    network_port_instance: int = 1,
    max_apdu: int = 1476,
    segmentation: Optional[Segmentation] = None,
    services: Optional[list[int]] = None,
    device_uuid: Optional[Union[bytes, bytearray]] = None,
    queue_capacity: int = 16,
    apdu_timeout_ms: int = 6000,
    apdu_retries: int = 0,
    registered_network_port: Optional[int] = None,
    *,
    read_work_limit: int = 256,
    share_port_by_address: bool = False,
    min_request_interval_ms: int = 0,
    bbmd: Optional[Bbmd] = None,
    foreign_device: Optional[ForeignDevice] = None,
    multicast: Optional[Multicast] = None,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    reinit_password: Optional[str] = None,
    device_write_policy: Optional[MutationHandler] = None,
) -> None
```

`on_reinitialize` / `after_reinitialize`: execute
ReinitializeDevice, as `BACnetServer`'s keywords do (#1566);
`reinit_password` (needs `on_reinitialize`, else ValueError; 1 to
20 characters, else ValueError) is the password a request must
carry. The callable runs on the session's
one dispatch task, so a slow one stalls every request.
`device_write_policy`: a callable that decides each WriteProperty
to the local Device's Description or installed Audit recipient
(#2033). It gets a `MutationContext` and returns a bool, with the
fail-closed rules of `BACnetServer`'s `mutation_policy` callable:
an exception, a non-bool or interpreter shutdown denies with SERVICES
/ SERVICE\_REQUEST\_DENIED. Without it the endpoint executes no
WriteProperty. It runs with no lock held, but on the session's one
dispatch task, so it too should decide quickly: while it runs, other
requests and `close()` wait for it.
`bbmd` / `foreign_device` (at most one, else ValueError): run
the endpoint as a BBMD, or register it as a foreign device (#938).
`read_work_limit`: result rows one ReadProperty served by the
server role may expand, a Group's member rows included; a read past it
is aborted with OUT\_OF\_RESOURCES. Zero raises ValueError.
`share_port_by_address` (default False): bind the interface address
itself, so endpoints on other addresses of this host can share the
port. Needs an explicit interface and a nonzero port; broadcasts and
unicast are still handled in the order they arrived.
`min_request_interval_ms` (default 0, no pacing; at most 3,600,000,
else ValueError) paces the client role's confirmed requests to each
destination exactly as `BACnetClient`'s keyword does (#1542). A
retry keeps its request's turn; replies, notifications and
unconfirmed requests aren't paced.

### BipEndpoint.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
) -> None
```

### BipEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

### BipEndpoint.add_binary_input

```python
def add_binary_input(self, instance: int, name: str) -> None
```

### BipEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

### BipEndpoint.add_group

```python
def add_group(
    self,
    instance: int,
    name: str,
    members: Optional[list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]] = None,
) -> None
```

Group whose Present\_Value is rebuilt from `members` on each read.

`members` has the `read_property_multiple` spec shape. A member
with no properties, or one reporting a group's Present\_Value, raises
ValueError naming its position and the rule. Any property identifier
is taken, those ASHRAE assigns above 4194303 included. Indexes
outside unsigned32 raise OverflowError.

### BipEndpoint.start

```python
def start(self) -> Awaitable[None]
```

Admit one startup; second start while running raises BacnetError.

### BipEndpoint.close

```python
def close(self) -> Awaitable[None]
```

Join earlier startup and cleanup; idempotent and safe after cancellation.

### BipEndpoint.__aenter__

```python
def __aenter__(self) -> Awaitable[BipEndpoint]
```

### BipEndpoint.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

### BipEndpoint.client

```python
def client(self) -> Awaitable[EndpointClient]
```

Clone the client role (RuntimeError before start/after close).

### BipEndpoint.server

```python
def server(self) -> Awaitable[EndpointServer]
```

Clone the server role (RuntimeError before start/after close).

### BipEndpoint.local_address

```python
def local_address(self) -> Awaitable[str]
```

Active announced IP and actual UDP port; RuntimeError outside active state.

### BipEndpoint.status

```python
def status(self) -> Awaitable[EndpointStatus]
```

Bounded snapshot (RuntimeError before start/after close).

### BipEndpoint.foreign_registration

```python
def foreign_registration(self) -> Awaitable[ForeignRegistrationStatus]
```

How the foreign-device registration is going (#938), as for
`BACnetClient`. RuntimeError before start and after close.

### BipEndpoint.broadcast_i_am

```python
def broadcast_i_am(self) -> Awaitable[None]
```

Broadcast one I-Am consistent with the composed identity.

## ScEndpoint

SC endpoint: one hub connection that both initiates and executes.

`sc_device_uuid` is the single durable lifetime identity for both the
hub dial and DEVICE\_UUID. Lifecycle mirrors `BipEndpoint`.
`device_instance` takes 0 to 4194302, as there: 4194303 or more raises
ValueError at construction (#2171).

Properties:

- `device_instance: int`
- `vendor_id: int`

### ScEndpoint.__init__

```python
def __init__(
    self,
    device_instance: int,
    sc_hub: str,
    sc_vmac: Union[bytes, bytearray],
    sc_ca_cert: str,
    sc_client_cert: str,
    sc_client_key: str,
    *,
    sc_device_uuid: Union[bytes, bytearray],
    device_name: str = "BACnet Device",
    vendor_id: int = 555,
    sc_heartbeat_interval_ms: int = 30000,
    sc_heartbeat_timeout_ms: int = 60000,
    network_number: int = 0,
    network_port_instance: int = 2,
    max_apdu: int = 1476,
    segmentation: Optional[Segmentation] = None,
    services: Optional[list[int]] = None,
    queue_capacity: int = 16,
    read_work_limit: int = 256,
    min_request_interval_ms: int = 0,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    reinit_password: Optional[str] = None,
    device_write_policy: Optional[MutationHandler] = None,
) -> None
```

`read_work_limit`, `min_request_interval_ms`,
`on_reinitialize`, `after_reinitialize`, `reinit_password` and
`device_write_policy` are as for `BipEndpoint`.

### ScEndpoint.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
) -> None
```

### ScEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

### ScEndpoint.add_binary_input

```python
def add_binary_input(self, instance: int, name: str) -> None
```

### ScEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

### ScEndpoint.add_group

```python
def add_group(
    self,
    instance: int,
    name: str,
    members: Optional[list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]] = None,
) -> None
```

Group whose Present\_Value is rebuilt from `members` on each read.

`members` has the `read_property_multiple` spec shape. A member
with no properties, or one reporting a group's Present\_Value, raises
ValueError naming its position and the rule. Any property identifier
is taken, those ASHRAE assigns above 4194303 included. Indexes
outside unsigned32 raise OverflowError.

### ScEndpoint.start

```python
def start(self) -> Awaitable[None]
```

Admit one hub connection; second start while running raises BacnetError.

### ScEndpoint.close

```python
def close(self) -> Awaitable[None]
```

Join earlier startup and cleanup; idempotent and safe after cancellation.

### ScEndpoint.__aenter__

```python
def __aenter__(self) -> Awaitable[ScEndpoint]
```

### ScEndpoint.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

### ScEndpoint.client

```python
def client(self) -> Awaitable[EndpointClient]
```

Clone the client role.

### ScEndpoint.server

```python
def server(self) -> Awaitable[EndpointServer]
```

Clone the server role.

### ScEndpoint.local_address

```python
def local_address(self) -> Awaitable[str]
```

VMAC hex for this SC node.

### ScEndpoint.status

```python
def status(self) -> Awaitable[EndpointStatus]
```

Bounded snapshot (RuntimeError before start/after close).

### ScEndpoint.broadcast_i_am

```python
def broadcast_i_am(self) -> Awaitable[None]
```

Broadcast one I-Am via the hub relay.

## MstpEndpoint

MS/TP endpoint: one serial owner that both initiates and executes.

Opens a real serial device via `TokioSerialPort` like the current
wrappers. Lifecycle mirrors `BipEndpoint`.
`device_instance` takes 0 to 4194302, as there: 4194303 or more raises
ValueError at construction (#2171).

Properties:

- `device_instance: int`
- `vendor_id: int`

### MstpEndpoint.__init__

```python
def __init__(
    self,
    device_instance: int,
    serial_port: str,
    device_name: str = "BACnet Device",
    vendor_id: int = 555,
    mstp_baud: int = 38400,
    mstp_mac: int = 1,
    mstp_max_master: int = 127,
    mstp_max_info_frames: int = 1,
    max_apdu: int = 480,
    segmentation: Optional[Segmentation] = None,
    services: Optional[list[int]] = None,
    device_uuid: Optional[Union[bytes, bytearray]] = None,
    queue_capacity: int = 16,
    apdu_timeout_ms: int = 6000,
    apdu_retries: int = 0,
    *,
    read_work_limit: int = 256,
    min_request_interval_ms: int = 0,
    mstp_extended_frames: bool = False,
    mstp_echo: bool = False,
    mstp_echo_grace_ms: Optional[int] = None,
    mstp_transmit_latency_pad_ms: Optional[int] = None,
    mstp_usage_timeout_ms: Optional[int] = None,
    mstp_reply_timeout_ms: Optional[int] = None,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    reinit_password: Optional[str] = None,
    device_write_policy: Optional[MutationHandler] = None,
) -> None
```

`read_work_limit`, `min_request_interval_ms`,
`on_reinitialize`, `after_reinitialize`, `reinit_password` and
`device_write_policy` are as for `BipEndpoint`. Here
`after_reinitialize` runs once the SimpleACK is queued, before the
token sends it: stopping the endpoint at once can drop the reply, so
wait a short while first.

`max_apdu` must fit the link: 480 by default, or 1476 with
`mstp_extended_frames=True`, which turns on extended frames (Types
32 and 33, NPDUs of up to 1497 octets) as for `BACnetClient`.
Every node on the link must then take extended frames; each port's
transmit queues reserve 768 KiB instead of 272 KiB.

`mstp_echo=True` declares an adapter that returns every octet the
station sends, and `mstp_echo_grace_ms` how long after a frame its
echo may still arrive (None: 40 ms), as for `BACnetClient`. A grace
of 0, or one given without `mstp_echo=True`, raises ValueError here;
a negative grace, or one above 4294967295, raises OverflowError.

`mstp_transmit_latency_pad_ms`, `mstp_usage_timeout_ms` and
`mstp_reply_timeout_ms` are as for `BACnetClient`, but a value
out of range raises ValueError here.

### MstpEndpoint.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
) -> None
```

### MstpEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

### MstpEndpoint.add_binary_input

```python
def add_binary_input(self, instance: int, name: str) -> None
```

### MstpEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

### MstpEndpoint.add_group

```python
def add_group(
    self,
    instance: int,
    name: str,
    members: Optional[list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]] = None,
) -> None
```

Group whose Present\_Value is rebuilt from `members` on each read.

`members` has the `read_property_multiple` spec shape. A member
with no properties, or one reporting a group's Present\_Value, raises
ValueError naming its position and the rule. Any property identifier
is taken, those ASHRAE assigns above 4194303 included. Indexes
outside unsigned32 raise OverflowError.

### MstpEndpoint.start

```python
def start(self) -> Awaitable[None]
```

Admit one serial owner; second start while running raises BacnetError.

### MstpEndpoint.close

```python
def close(self) -> Awaitable[None]
```

Join earlier startup and cleanup; idempotent and safe after cancellation.

### MstpEndpoint.__aenter__

```python
def __aenter__(self) -> Awaitable[MstpEndpoint]
```

### MstpEndpoint.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

### MstpEndpoint.client

```python
def client(self) -> Awaitable[EndpointClient]
```

Clone the client role.

### MstpEndpoint.server

```python
def server(self) -> Awaitable[EndpointServer]
```

Clone the server role.

### MstpEndpoint.local_address

```python
def local_address(self) -> Awaitable[str]
```

Station MAC as a decimal string.

### MstpEndpoint.status

```python
def status(self) -> Awaitable[EndpointStatus]
```

Bounded snapshot (RuntimeError before start/after close).

### MstpEndpoint.broadcast_i_am

```python
def broadcast_i_am(self) -> Awaitable[None]
```

Broadcast one I-Am (MS/TP local broadcast).

## EndpointClient

Client role cloned from a running endpoint (no lifecycle).

Initiates `read_property`, `read_range`, `read_property_multiple` and direct B/IP `write_property` through the owner's single transport.
After the owner closes, calls fail closed with `BacnetError`.

### EndpointClient.read_property

```python
def read_property(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[PropertyValue]
```

Read a property through the shared transport.

ACK object/property/index must match. Device/Network Port instance 4194303
requests accept a same-type concrete ACK. Malformed/mismatched ACKs raise
BacnetError; this method returns the property value, not ACK metadata,
shaped as `PropertyValue` describes for read results.

### EndpointClient.write_property

```python
def write_property(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
    *,
    commandability: Literal["commandable", "noncommandable"],
) -> Awaitable[None]
```

Write to a direct IPv4 B/IP peer through the shared requester.

Commandability is required even without a source Reporter. It controls
Audit filtering, never the wire priority. Omitted commandable priority
means effective 16, including NULL. Invalid literals, priority, raw TLV
framing and APDU size raise ValueError synchronously (out-of-u8 priorities
raise OverflowError). Empty encoded lists are valid. Eligible source
records preserve complete 0–32-byte values and omit longer values.
After source admission, caller cancellation does not cancel the attempt.

### EndpointClient.read_property_multiple

```python
def read_property_multiple(
    self,
    address: str,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]],
) -> Awaitable[list[ReadAccessResult]]
```

Read 1–64 explicit property occurrences on concrete objects.

Empty lists, ALL/REQUIRED/OPTIONAL and wildcard instances raise ValueError
before address parsing or I/O. Index zero is valid. Complete ordered ACK
correlation permits an inline error to omit its requested index; successful
values must echo it. Byte limits and malformed/mismatched ACKs raise
BacnetError. Returned dictionaries share standalone RPM conversion.

### EndpointClient.read_range

```python
def read_range(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
    range_type: Optional[Literal["position", "sequence", "time"]] = None,
    reference_index: Optional[int] = None,
    reference_seq: Optional[int] = None,
    count: Optional[int] = None,
    *,
    reference_time: ReferenceTime | None = None,
    validation: Literal["strict", "lenient"] = "strict",
) -> Awaitable[ReadRangeResult]
```

Read a range of items from a list or log object.

`range_type` is `"position"`, `"sequence"`, `"time"` or
`None` (all-items). `"time"` reads the `count` items newer than
`reference_time` (older, for a negative count): a naive
`datetime.datetime` in the device's local time, or a
`((year, month, day, day_of_week), (hour, minute, second, hundredths))` pair; an aware datetime raises ValueError, since the
device's zone isn't known here (#1533). Invalid selectors, array index
zero, missing or zero counts and a time that isn't specific raise
ValueError before I/O; a count outside INTEGER16 (-32768..=32767)
raises OverflowError before address parsing or I/O (#1360).
Position/sequence reference zero is valid; omitted references default
to zero.

`validation="strict"` refuses an answer that breaks a ReadRange rule
with BacnetReadRangeViolationError, whose `rule` names it;
`"lenient"` keeps it and lists the rules in `violations` (#1531).
Decode a log's records with `decode_log_records`.

### EndpointClient.read_log_page

```python
def read_log_page(
    self,
    address: str,
    object_id: ObjectIdentifier,
    cursor: LogCursor = None,
    page_size: int = 100,
) -> Awaitable[LogPage]
```

Read one page of a log's Log\_Buffer from `cursor` (#1530).

`cursor` is `None` or `"oldest"` (the oldest record, found from
Record\_Count and Total\_Record\_Count), `("sequence", n)`,
`("position", n)` or `("time", reference_time)`; lists work
too, so a checkpoint survives JSON. Loop on `page["next"]` until
`page["done"]`, then keep `next` as the checkpoint. One request
is outstanding at a time. A first record past the one asked for sets
`gap`; when the log drops the record asked for, the read starts
over from the oldest. A page answered from before the one asked for,
or none where the counts say records are, raises
BacnetLogNotAdvancingError: such a device's sequence numbers are
inconsistent, so read it from `("position", 1)`. `wrapped` marks
a page that reached the top of the sequence range. A page whose
records don't decode raises BacnetError, dropping the records before
the failing one; an answer that breaks a rule the reader can't
tolerate, such as an echo that doesn't match, raises
BacnetReadRangeViolationError. A non-log object or a `page_size`
outside 1..=32767 raises ValueError before I/O.

### EndpointClient.service_scope

```python
def service_scope(self) -> dict[str, Any]
```

Narrow scope: initiates reads and direct B/IP `write_property`.

## EndpointServer

Server role cloned from a running endpoint (no lifecycle).

The responder executes `read_property` automatically, WriteProperty to
the local Device through the owner's `device_write_policy`, and
ReinitializeDevice through the owner's `on_reinitialize`, the one
Python callback that runs under a native lock; this handle exposes
liveness plus the MS/TP deferred-reply seam.

### EndpointServer.is_session_alive

```python
def is_session_alive(self) -> bool
```

True while the owning endpoint is alive and open.

### EndpointServer.suspend_next_reply

```python
def suspend_next_reply(self) -> None
```

Arm one-shot deferred-reply suspension (MS/TP wiring).

### EndpointServer.service_scope

```python
def service_scope(self) -> dict[str, Any]
```

Narrow scope: executes `read_property`, plus `write_property`
when the owner has `device_write_policy` and
`reinitialize_device` when it has `on_reinitialize`.

## EndpointStatus

`class EndpointStatus(TypedDict)`

Bounded endpoint snapshot: liveness, identity, transport, leases, policy counts.

Keys:

- `is_running: bool`
- `device_instance: int`
- `vendor_id: int`
- `max_apdu: int`
- `transport: str`
- `local_address: str`
- `active_leases: int`
- `ingress_policy: int`
- `no_server_role: int`
- `no_client_role: int`
- `unclaimed_terminal: int`
- `responder_declined: int`

## ReadRangeResult

`class ReadRangeResult(TypedDict)`

Keys:

- `object_id: ObjectIdentifier`
- `property_id: PropertyIdentifier`
- `array_index: int | None`
- `result_flags: BitString`
- `item_count: int`
- `item_data: bytes`
- `first_sequence_number: int | None`
- `violations: list[str]`

## LogGap

`class LogGap(TypedDict)`

A page whose first record isn't the one asked for.

Keys:

- `expected: int`
- `first: int`
- `skipped: int | None`

## LogPage

`class LogPage(TypedDict)`

One page of `read_log_page`.

`records` are dicts, oldest first, as `decode_log_records` returns
them. `next` is never `None` or `"oldest"`.

Keys:

- `records: list[dict[str, Any]]`
- `first_sequence_number: int | None`
- `result_flags: BitString`
- `gap: LogGap | None`
- `violations: list[str]`
- `next: LogCursor`
- `done: bool`
- `wrapped: bool`

## ReadPropertyResult

`class ReadPropertyResult(TypedDict)`

Keys:

- `property_id: PropertyIdentifier`
- `array_index: int | None`
- `value: PropertyValue | bytes | None`
- `error: tuple[ErrorClass, ErrorCode] | None`

## ReadAccessResult

`class ReadAccessResult(TypedDict)`

Keys:

- `object_id: ObjectIdentifier`
- `results: list[ReadPropertyResult]`

## ReferenceTime

```python
ReferenceTime = Union[
    datetime.datetime,
    tuple[tuple[int, int, int, int], tuple[int, int, int, int]],
]
```

## LogCursor

```python
LogCursor = Union[
    None,
    Literal["oldest"],
    tuple[Literal["sequence"], int],
    tuple[Literal["position"], int],
    tuple[Literal["time"], ReferenceTime],
]
```
<!-- generated:end -->
