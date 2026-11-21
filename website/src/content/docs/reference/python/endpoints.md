---
title: "Python shared endpoints"
description: "The B/IP, SC and MS/TP endpoints and what their status reports, generated from the type stub."
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

The server role is a full `BACnetServer` (#2617), from
`await endpoint.server()`: it executes the services a standalone server
does, answers Who-Is and Who-Has, and lists those services in the
Device's Protocol\_Services\_Supported. Without `mutation_policy` it is
read-only: a peer's write, create or delete is denied with SERVICES /
SERVICE\_REQUEST\_DENIED, and reads, discovery, COV subscriptions and alarm
acknowledgments are answered. Time synchronization is off on every
endpoint until `time_sync_policy` turns it on, whatever the
`mutation_policy`: a default that differs from `BACnetServer()`'s.

`device_instance` takes 0 to 4194302. Instance 4194303 is reserved, as
no object may have it, so it, or anything larger, raises ValueError at
construction (#2171).

For B/IP, `interface` accepts an IPv4 literal or the exact OS interface
name (the adapter friendly name on Windows), resolved once, at
construction, to one usable IPv4 address; `broadcast_address` is not
derived from it. See "Selecting an interface" in docs/python-api.md.

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
    device_uuid: Optional[Union[bytes, bytearray]] = None,
    queue_capacity: int = 256,
    apdu_timeout_ms: int = 6000,
    apdu_retries: int = 3,
    registered_network_port: Optional[int] = None,
    *,
    read_work_limit: int = 256,
    share_port_by_address: bool = False,
    share_wildcard_port: bool = False,
    min_request_interval_ms: int = 0,
    bbmd: Optional[Bbmd] = None,
    foreign_device: Optional[ForeignDevice] = None,
    multicast: Optional[Multicast] = None,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    reinit_password: Optional[str] = None,
    mutation_policy: Literal["permissive", "deny_all"] | MutationHandler | None = None,
    cov_policy: CovPolicy | None = None,
    time_sync_policy: TimeSyncPolicy | None = None,
    source_audit_reporter: Optional[ObjectIdentifier] = None,
    audit_recipient: Optional[AuditRecipientInput] = None,
    audit_device_bindings: Optional[list[tuple[ObjectIdentifier, str]]] = None,
) -> None
```

`mutation_policy` (#2617): who may change the endpoint's
objects, as `BACnetServer`'s keyword of the same name takes it:
`"permissive"` (any peer), `"deny_all"` (no peer, COV
subscriptions included) or a callable that gets a `MutationContext`
for each mutation and returns a bool, with that keyword's fail-closed
rules. Without it (None, the default) the server is read-only: every
write, create and delete is denied, and reads, discovery, COV
subscriptions and alarm acknowledgments are answered. Given one, the
server is the one `BACnetServer` with that policy is, but for time
synchronization. `cov_policy`: the server's `CovPolicy`, read as
`BACnetServer`'s is. `time_sync_policy`: None, the default, leaves
TimeSynchronization and UTCTimeSynchronization off whatever the
`mutation_policy`, a default that differs from `BACnetServer()`'s,
which takes every valid request; a `TimeSyncPolicy` dict, read as
`BACnetServer`'s is, turns them on (`{}` takes every valid
request). The application changes its objects through
`await endpoint.server()`, whose COV and event notifications follow
as on a standalone server, and records its own resets through that
server's `report_device_reset_local()`. `ScEndpoint` and
`MstpEndpoint` take all three too.

`source_audit_reporter` / `audit_recipient` /
`audit_device_bindings` (#2348): the Audit Reporter, added with
`add_audit_reporter`, through which the session reports the
ReinitializeDevice resets, backups and restores the server carries
out, its server's `report_device_reset_local()` calls and the reads
and writes of its `client()` (#2617), each filtered by the
Reporter's level and
operations. It needs `audit_recipient` (the Device's Audit
recipient) and a `mutation_policy` that decides writes, so that no
peer redirects the audit trail unasked: none (read-only),
`"deny_all"` or a callable, never `"permissive"` (ValueError).
The bindings are `(Device identifier, "ipv4:port")`
routes to recipient Devices, or `"network:0xMAC"` for one behind a
router on that network (#2371; network 1 to 65534, MAC 1 to 18
octets). A Reporter identifier of another type and
the other shapes construction can check raise ValueError; a Reporter
that wasn't added raises BacnetError at start. `ScEndpoint` and
`MstpEndpoint` take them too, each binding address in its link's
format (#2357) or the routed form.
`on_reinitialize` / `after_reinitialize`: execute
ReinitializeDevice, as `BACnetServer`'s keywords do (#1566);
`reinit_password` (needs `on_reinitialize`, else ValueError; 1 to
20 characters, else ValueError) is the password a request must
carry. Without `on_reinitialize` the server denies every
ReinitializeDevice.
`bbmd` / `foreign_device` (at most one, else ValueError): run
the endpoint as a BBMD, or register it as a foreign device (#938).
`read_work_limit`: the server's ReadPropertyMultiple budget of
result rows, `BACnetServer`'s `rpm_max_result_elements`: a
ReadProperty counts its own row and a Group's member rows, a
ReadPropertyMultiple every result, and one past it is aborted with
OUT\_OF\_RESOURCES. Zero raises ValueError.
`share_port_by_address` (default False): bind the interface address
itself, so endpoints on other addresses of this host can share the
port. Needs an explicit interface and a nonzero port; broadcasts and
unicast are still handled in the order they arrived.
`share_wildcard_port` (default False): share an explicitly
requested port with other sockets on this host that bind it the
same way. By default the endpoint claims its port, and `start()`
raises when another socket holds it; with sharing, each unicast
reaches only one of the sockets on the port.
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
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register an Analog Input before start.

`present_value` (a finite number, default 0.0) sets Present\_Value
directly.

Profile keywords as on `BACnetServer.add_analog_input`: each
provisions its optional row, served only when given. The parameter
keywords set those parameters as there, with the same checks.

### BipEndpoint.add_analog_output

```python
def add_analog_output(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    present_value: float | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register an Analog Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### BipEndpoint.add_binary_output

```python
def add_binary_output(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    polarity: Polarity | int = 0,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Binary Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### BipEndpoint.add_multistate_input

```python
def add_multistate_input(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_values: list[int] | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Input before start.

`present_value` sets Present\_Value directly. `None` keeps the
object type's default.

### BipEndpoint.add_multistate_output

```python
def add_multistate_output(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### BipEndpoint.add_multistate_value

```python
def add_multistate_value(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_values: list[int] | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Value before start.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

### BipEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    present_value: float | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

Access, Value\_Source, profile and parameter keywords as on
`BACnetServer.add_analog_value`; tracking on a commandable object
raises ValueError.

### BipEndpoint.add_binary_input

```python
def add_binary_input(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    polarity: Polarity | int = 0,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_value: int | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Binary Input before start.

`present_value` is the logical value (already after Polarity): it
sets Present\_Value, and the object derives its physical input state
from it. Left as `None`, the object keeps its unset physical-input
state.

Profile keywords as on `add_analog_input`, `polarity`,
`alarm_value` and the delays as on `BACnetServer.add_binary_input`.

### BipEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_value: int | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

Access, Value\_Source and profile keywords as on `add_analog_value`,
`alarm_value` and the delays as on `BACnetServer.add_binary_value`.

### BipEndpoint.add_audit_reporter

```python
def add_audit_reporter(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["none", "audit_config", "audit_all"],
    auditable_operations: int,
    issue_confirmed_notifications: bool = False,
) -> None
```

Add an Audit Reporter for `source_audit_reporter` to select
(#2348). `audit_level` and `auditable_operations` (bit n for
AuditOperation n) are required, as on
`BACnetServer.configure_audit_reporters`, and filter what the
session reports. Peers can't change them through an endpoint. A
Reporter that `source_audit_reporter` doesn't select records
nothing.

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
def client(self) -> Awaitable[BACnetClient]
```

The `BACnetClient` on this endpoint's socket (#2617): the
standalone class, with all its calls, on the endpoint's one address.
While it runs, each call returns the same object; once its `stop()`
has finished, a fresh one. Its `stop()` and `async with` exit end
the client role alone; the endpoint's close stops it too, and calls
in flight end as a stopping client's do. RuntimeError before start,
after close, and while the last client is still stopping or its stop
failed (the error names the cause). On an endpoint built with
`source_audit_reporter`, the endpoint audits the client's reads
and writes (#2617).

### BipEndpoint.server

```python
def server(self) -> Awaitable[BACnetServer]
```

The `BACnetServer` the endpoint's session runs on its socket
(#2617): the standalone class, with its runtime calls, on the
endpoint's one address and its objects. Each call returns a new
handle on the one server. Its `start()` and `stop()` raise
RuntimeError: the endpoint starts it, and the endpoint's
`close()` stops it, after which its calls raise RuntimeError
rather than wait. RuntimeError before start and after close.

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

### BipEndpoint.frames

```python
def frames(self, capacity: int = 1024) -> Awaitable[FrameIterator]
```

Observe the raw frames on the endpoint's own socket from now on,
beside both roles' traffic, `capacity` frames queued at most (1 to
65536, else ValueError). RuntimeError before start and after close.
Close ends the iterator; a restart needs a new one.

### BipEndpoint.i_am_arrivals

```python
def i_am_arrivals(self, capacity: int = 1024) -> Awaitable[IAmArrivalIterator]
```

Follow every I-Am the endpoint's socket receives from now on, as
`BACnetClient.i_am_arrivals` does (#2612). The endpoint's own I-Am
come back to it, from its own address and naming its own device
instance, is left out and counted in `own_echoes`; another
instance from that address is a record. RuntimeError before start
and after close. Close ends the iterator.

### BipEndpoint.read_bdt

```python
def read_bdt(self, address: str) -> Awaitable[list[tuple[str, str]]]
```

Read the Broadcast Distribution Table of the BBMD at `address`
(`"ip"` or `"ip:port"`) through this endpoint's own socket, beside
the roles' traffic (#2612), as `BACnetClient.read_bdt` does: the
same `("ip:port", "mask")` rows, and the same errors: a NAK raises
BacnetBvlcNakError with its `result_code` (0x0020 from a node that
is no BBMD), no answer within 3 seconds BacnetTimeoutError. A bad
address raises ValueError before I/O; RuntimeError before start and
after close, and for a read that `close()` cuts off (`close()`
doesn't wait for one). A cancelled read gives its request up, so the
same read can start again at once; the same read beside one still
running is refused, as `BACnetClient`'s is.

### BipEndpoint.read_fdt

```python
def read_fdt(self, address: str) -> Awaitable[list[FdtEntry]]
```

Read the Foreign Device Table of the BBMD at `address` through
this endpoint's own socket (#2612), as `BACnetClient.read_fdt`
does, with the errors of `read_bdt`. The BVLC writes stay on
`BACnetClient`.

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
    queue_capacity: int = 256,
    read_work_limit: int = 256,
    min_request_interval_ms: int = 0,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    reinit_password: Optional[str] = None,
    mutation_policy: Literal["permissive", "deny_all"] | MutationHandler | None = None,
    cov_policy: CovPolicy | None = None,
    time_sync_policy: TimeSyncPolicy | None = None,
    source_audit_reporter: Optional[ObjectIdentifier] = None,
    audit_recipient: Optional[AuditRecipientInput] = None,
    audit_device_bindings: Optional[list[tuple[ObjectIdentifier, str]]] = None,
    reconnect: Optional[ScReconnect] = None,
    sc_failover_hub: str | None = None,
    sc_connect_timeout_ms: int | None = None,
) -> None
```

`read_work_limit`, `min_request_interval_ms`,
`on_reinitialize`, `after_reinitialize`, `reinit_password`,
`mutation_policy`, `cov_policy` and `time_sync_policy` are as
for `BipEndpoint`.

`source_audit_reporter` / `audit_recipient` /
`audit_device_bindings`: as on `BipEndpoint` (#2357), each
binding address a VMAC as `local_address()` gives it
(`"xx:xx:xx:xx:xx:xx"`) or as twelve hex digits, or a routed
`"network:0xMAC"`. An address
`audit_recipient` takes another node's VMAC: never the broadcast
or all-zero VMAC or this endpoint's own.

`reconnect`, `sc_failover_hub` and `sc_connect_timeout_ms`: as
on `BACnetClient` (#1946). `reconnect` (an `ScReconnect`)
redials a lost hub; `sc_failover_hub` (needs `reconnect`, else
ValueError) is a second hub's `wss://` URL, dialled with the same
TLS settings when the primary can't be reached, and the endpoint
keeps trying the primary and returns to it. `sc_connect_timeout_ms`
(at least 1, default 10000) bounds each hub dial and Connect-Request.
Follow the link with `connection_events()` and
`connection_status()`.

### ScEndpoint.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register an Analog Input before start.

`present_value` (a finite number, default 0.0) sets Present\_Value
directly.

Profile keywords as on `BACnetServer.add_analog_input`: each
provisions its optional row, served only when given. The parameter
keywords set those parameters as there, with the same checks.

### ScEndpoint.add_analog_output

```python
def add_analog_output(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    present_value: float | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register an Analog Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### ScEndpoint.add_binary_output

```python
def add_binary_output(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    polarity: Polarity | int = 0,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Binary Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### ScEndpoint.add_multistate_input

```python
def add_multistate_input(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_values: list[int] | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Input before start.

`present_value` sets Present\_Value directly. `None` keeps the
object type's default.

### ScEndpoint.add_multistate_output

```python
def add_multistate_output(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### ScEndpoint.add_multistate_value

```python
def add_multistate_value(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_values: list[int] | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Value before start.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

### ScEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    present_value: float | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

Access, Value\_Source, profile and parameter keywords as on
`BACnetServer.add_analog_value`; tracking on a commandable object
raises ValueError.

### ScEndpoint.add_binary_input

```python
def add_binary_input(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    polarity: Polarity | int = 0,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_value: int | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Binary Input before start.

`present_value` is the logical value (already after Polarity): it
sets Present\_Value, and the object derives its physical input state
from it. Left as `None`, the object keeps its unset physical-input
state.

Profile keywords as on `add_analog_input`, `polarity`,
`alarm_value` and the delays as on `BACnetServer.add_binary_input`.

### ScEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_value: int | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

Access, Value\_Source and profile keywords as on `add_analog_value`,
`alarm_value` and the delays as on `BACnetServer.add_binary_value`.

### ScEndpoint.add_audit_reporter

```python
def add_audit_reporter(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["none", "audit_config", "audit_all"],
    auditable_operations: int,
    issue_confirmed_notifications: bool = False,
) -> None
```

Add an Audit Reporter for `source_audit_reporter` to select
(#2348). `audit_level` and `auditable_operations` (bit n for
AuditOperation n) are required, as on
`BACnetServer.configure_audit_reporters`, and filter what the
session reports. Peers can't change them through an endpoint. A
Reporter that `source_audit_reporter` doesn't select records
nothing.

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
def client(self) -> Awaitable[BACnetClient]
```

The `BACnetClient` on this endpoint's hub connection (#2617): the
standalone class, with all its calls, on the endpoint's one address.
While it runs, each call returns the same object; once its `stop()`
has finished, a fresh one. Its `stop()` and `async with` exit end
the client role alone; the endpoint's close stops it too, and calls
in flight end as a stopping client's do. RuntimeError before start,
after close, and while the last client is still stopping or its stop
failed (the error names the cause). On an endpoint built with
`source_audit_reporter`, the endpoint audits the client's reads
and writes (#2617).

It starts after the hub link is up, so its `connection_events()`
begins at that call, after the first `connected`: read the link's
state at once with its `connection_status()`, or use this
endpoint's `connection_events()`, whose first call holds the
events from the start.

### ScEndpoint.server

```python
def server(self) -> Awaitable[BACnetServer]
```

The `BACnetServer` the endpoint's session runs on its hub
connection (#2617), as `BipEndpoint.server()` gives one.

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

### ScEndpoint.connection_events

```python
def connection_events(self) -> Awaitable[ScEventIterator]
```

Subscribe to the SC hub-connector events (#1946), as
`BACnetClient.connection_events()` does. The first call after a
start gets the events since the session was built, starting with
the first `connected` (#1908); a later call gets the events from
that call on. A slow reader never holds up the connector: see
`ScEventIterator.missed`. Iteration ends when the endpoint closes.
RuntimeError before start or after close.

### ScEndpoint.connection_status

```python
def connection_status(self) -> Awaitable[ScLinkStatus]
```

Snapshot the SC hub link without I/O (#1946), as
`BACnetClient.connection_status()` does. RuntimeError before start
or after close.

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
    device_uuid: Optional[Union[bytes, bytearray]] = None,
    queue_capacity: int = 256,
    apdu_timeout_ms: int = 6000,
    apdu_retries: int = 3,
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
    mutation_policy: Literal["permissive", "deny_all"] | MutationHandler | None = None,
    cov_policy: CovPolicy | None = None,
    time_sync_policy: TimeSyncPolicy | None = None,
    source_audit_reporter: Optional[ObjectIdentifier] = None,
    audit_recipient: Optional[AuditRecipientInput] = None,
    audit_device_bindings: Optional[list[tuple[ObjectIdentifier, str]]] = None,
) -> None
```

`read_work_limit`, `min_request_interval_ms`,
`on_reinitialize`, `after_reinitialize`, `reinit_password`,
`mutation_policy`, `cov_policy` and `time_sync_policy` are as
for `BipEndpoint`. Here
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

`source_audit_reporter` / `audit_recipient` /
`audit_device_bindings`: as on `BipEndpoint` (#2357), each
binding address a station in decimal digits, as `local_address()`
gives it (`"2"`), or a routed `"network:0xMAC"`. An address
`audit_recipient` takes another station: never 255 or this
endpoint's own.

### MstpEndpoint.add_analog_input

```python
def add_analog_input(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    present_value: float = 0.0,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register an Analog Input before start.

`present_value` (a finite number, default 0.0) sets Present\_Value
directly.

Profile keywords as on `BACnetServer.add_analog_input`: each
provisions its optional row, served only when given. The parameter
keywords set those parameters as there, with the same checks.

### MstpEndpoint.add_analog_output

```python
def add_analog_output(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    present_value: float | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register an Analog Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### MstpEndpoint.add_binary_output

```python
def add_binary_output(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    polarity: Polarity | int = 0,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Binary Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### MstpEndpoint.add_multistate_input

```python
def add_multistate_input(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_values: list[int] | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Input before start.

`present_value` sets Present\_Value directly. `None` keeps the
object type's default.

### MstpEndpoint.add_multistate_output

```python
def add_multistate_output(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Output before start.

`present_value` sets Relinquish\_Default. The priority array starts
empty, so Present\_Value reads as that value until something is
commanded. `None` keeps the object type's default.

### MstpEndpoint.add_multistate_value

```python
def add_multistate_value(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value: int | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_values: list[int] | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Multi-state Value before start.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

### MstpEndpoint.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    present_value: float | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    high_limit: float | None = None,
    low_limit: float | None = None,
    deadband: float | None = None,
    limit_enable: BitString | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

Access, Value\_Source, profile and parameter keywords as on
`BACnetServer.add_analog_value`; tracking on a commandable object
raises ValueError.

### MstpEndpoint.add_binary_input

```python
def add_binary_input(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    polarity: Polarity | int = 0,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_value: int | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Register a Binary Input before start.

`present_value` is the logical value (already after Polarity): it
sets Present\_Value, and the object derives its physical input state
from it. Left as `None`, the object keeps its unset physical-input
state.

Profile keywords as on `add_analog_input`, `polarity`,
`alarm_value` and the delays as on `BACnetServer.add_binary_input`.

### MstpEndpoint.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    present_value: int | None = None,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
    alarm_value: int | None = None,
    time_delay: int | None = None,
    time_delay_normal: int | None = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.

`present_value` is the initial value. With
`present_value_access="commandable"` (the default) it sets
Relinquish\_Default and the priority array starts empty, so
Present\_Value reads as that value until something is commanded; with
"writable" or "read\_only" it sets Present\_Value directly. `None`
keeps the object type's default.

Access, Value\_Source and profile keywords as on `add_analog_value`,
`alarm_value` and the delays as on `BACnetServer.add_binary_value`.

### MstpEndpoint.add_audit_reporter

```python
def add_audit_reporter(
    self,
    instance: int,
    name: str,
    *,
    audit_level: Literal["none", "audit_config", "audit_all"],
    auditable_operations: int,
    issue_confirmed_notifications: bool = False,
) -> None
```

Add an Audit Reporter for `source_audit_reporter` to select
(#2348). `audit_level` and `auditable_operations` (bit n for
AuditOperation n) are required, as on
`BACnetServer.configure_audit_reporters`, and filter what the
session reports. Peers can't change them through an endpoint. A
Reporter that `source_audit_reporter` doesn't select records
nothing.

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
def client(self) -> Awaitable[BACnetClient]
```

The `BACnetClient` on this endpoint's serial link (#2617): the
standalone class, with all its calls, on the endpoint's one address.
While it runs, each call returns the same object; once its `stop()`
has finished, a fresh one. Its `stop()` and `async with` exit end
the client role alone; the endpoint's close stops it too, and calls
in flight end as a stopping client's do. RuntimeError before start,
after close, and while the last client is still stopping or its stop
failed (the error names the cause). On an endpoint built with
`source_audit_reporter`, the endpoint audits the client's reads
and writes (#2617).

Its requests wait for this node's token, as the server role's
answers do.

### MstpEndpoint.server

```python
def server(self) -> Awaitable[BACnetServer]
```

The `BACnetServer` the endpoint's session runs on its serial
link (#2617), as `BipEndpoint.server()` gives one. Its answers
wait for this node's token, as the client's requests do.

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

## EndpointStatus

`class EndpointStatus(TypedDict)`

Bounded endpoint snapshot: liveness, identity, transport, leases, policy counts.

The `*_full_drops` keys count what a full queue had no room for, by
class; a full queue never ends the session (#2635).
`unconfirmed_full_drops` counts unconfirmed requests (I-Am, Who-Is,
notifications) past their share of the request queue, which keeps a
quarter for confirmed requests; `confirmed_full_drops` confirmed
requests for the endpoint's own roles dropped unanswered, which their
requesters send again (one the client's seat refuses counts in
`client_seat_full_drops`);
`answer_full_drops` answers and segments (one no lease holds may not
take the last quarter of the answer queue); `policy_full_drops`
refused APDUs (malformed, an unknown PDU type). Each queue of
`queue_capacity` 2 or more keeps at least one slot in reserve; a
capacity of 1 keeps none.

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
- `unconfirmed_full_drops: int`
- `confirmed_full_drops: int`
- `answer_full_drops: int`
- `policy_full_drops: int`
- `client_seat_apdus: int`: APDUs handed to the client on the link (#2617), over the session's run.
- `client_seat_full_drops: int`: APDUs the client's queue refused, never waited for: those it had no room for, and those its tiers held back (copies of unconfirmed requests use at most half of it, confirmed requests to the client all but the quarter kept for its answers). On an endpoint with no server role, a confirmed request refused here is counted here, not in `confirmed_full_drops`.
- `client_seat_control_full_drops: int`: Network controls dropped the same way.
- `server_seat_apdus: int`: APDUs handed to the endpoint's server (#2617), over the session's run.
- `server_seat_full_drops: int`: APDUs the server's queue refused, never waited for, as the client's are counted: a confirmed request counted here went unanswered, and its requester's retry is its recovery.

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
