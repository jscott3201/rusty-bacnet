---
title: "Python server"
description: "BACnetServer, its policies and counters, and ScHub, generated from the type stub."
---

:::note[On `dev` after the release]
This page is generated from the type stub on the `dev` branch and tracks the next release, so it can list classes, methods and keywords merged after v0.15.0 that the released wheel doesn't have. The [v0.15.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) and the [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d) say what changed.
:::

The server, its policies and counters, and the BACnet/SC hub. The [Python reference](/rusty-bacnet/reference/python/) lists every group. Each page opens with its main classes, then the other classes, the dictionaries (`TypedDict`) and the type aliases, in stub order.

<!-- generated:start by website/scripts/reference-python.py; do not edit this page -->

## BACnetServer

BACnet server that hosts objects and responds to client requests.

For SC, sc\_ca\_cert, sc\_client\_cert, and sc\_client\_key must be nonempty
paths; omission, None, or empty strings raise ValueError at construction.
Their None defaults preserve non-SC use and positional layout only.
start() loads the explicit site CA and operational cert/key before dialing
or draining registrations. Local TLS configuration errors raise RuntimeError;
repair the files and retry on the same server. Later startup failures do not
have a general registration rollback guarantee.

SC also requires keyword-only sc\_device\_uuid: exactly 16 bytes, not all zero,
copied at construction. Missing, None, wrong length, or all-zero values
raise ValueError after credential-presence validation, before file/network I/O.
Provision before deployment and durably reuse the same UUID for the device's
lifetime. No generation, persistence, change detection, or version/variant
enforcement is provided. Non-SC transports ignore this option.

Segmentation defaults to NONE. When enabled, apdu\_segment\_timeout\_ms must
be positive; it supplies Device APDU\_SEGMENT\_TIMEOUT, response SegmentACK
waits, and the 4\*Tseg receive inactivity timer. A separate 16-second
no-progress resource limit aborts abandoned request transfers with OTHER.

Usage:

```python
server = BACnetServer(device_instance=1234, device_name="My Device")
server.add_analog_input(1, "Temperature", units=62)
await server.start()
```

### BACnetServer.__init__

```python
def __init__(
    self,
    device_instance: int,
    device_name: str = "BACnet Device",
    interface: str = "0.0.0.0",
    port: int = 0xBAC0,
    broadcast_address: str = "255.255.255.255",
    transport: str = "bip",
    sc_hub: Optional[str] = None,
    sc_vmac: Optional[bytes] = None,
    sc_ca_cert: Optional[str] = None,
    sc_client_cert: Optional[str] = None,
    sc_client_key: Optional[str] = None,
    sc_heartbeat_interval_ms: Optional[int] = None,
    sc_heartbeat_timeout_ms: Optional[int] = None,
    ipv6_interface: Optional[str] = None,
    dcc_password: Optional[str] = None,
    reinit_password: Optional[str] = None,
    *,
    mutation_policy: Literal["permissive", "deny_all"] | MutationHandler = "permissive",
    dcc_policy: str = "deny_all",
    dcc_source_restriction: list[tuple[int | None, bytes]] | None = None,
    dcc_disable_rate_limit: tuple[int, int] | None = None,
    serial_port: Optional[str] = None,
    mstp_baud: int = 38400,
    mstp_mac: int = 1,
    mstp_max_master: int = 127,
    mstp_max_info_frames: int = 1,
    mstp_extended_frames: bool = False,
    mstp_echo: bool = False,
    mstp_echo_grace_ms: Optional[int] = None,
    mstp_transmit_latency_pad_ms: Optional[int] = None,
    mstp_usage_timeout_ms: Optional[int] = None,
    mstp_reply_timeout_ms: Optional[int] = None,
    max_confirmed_in_flight: int = 64,
    max_unconfirmed_in_flight: int = 32,
    max_confirmed_in_flight_per_peer: int = 16,
    max_unconfirmed_in_flight_per_peer: int = 8,
    confirmed_recovery_reserve: int = 4,
    max_recovery_in_flight_per_peer: int = 1,
    rpm_max_result_elements: int = 256,
    rpm_max_service_ack_bytes: int = 16384,
    alarm_summary_max_objects: int = 4096,
    alarm_summary_max_service_ack_bytes: int = 16384,
    enrollment_summary_max_objects: int = 4096,
    enrollment_summary_max_service_ack_bytes: int = 16384,
    atomic_read_file_max_requested_stream_octets: int = 16384,
    atomic_read_file_max_requested_records: int = 256,
    atomic_read_file_max_service_ack_bytes: int = 16384,
    atomic_write_file_max_stream_payload_octets: int = 16384,
    atomic_write_file_max_records: int = 256,
    atomic_write_file_max_record_payload_bytes: int = 16384,
    read_range_max_returned_items: int = 256,
    read_range_max_service_ack_bytes: int = 16384,
    event_information_max_objects: int = 4096,
    event_information_max_returned_summaries: int = 256,
    event_information_max_service_ack_bytes: int = 16384,
    sc_device_uuid: Optional[bytes | bytearray] = None,
    registered_network_port: Optional[int] = None,
    cov_policy: CovPolicy | None = None,
    time_sync_policy: TimeSyncPolicy | None = None,
    share_port_by_address: bool = False,
    segmentation_supported: Segmentation = Segmentation.NONE,
    apdu_segment_timeout_ms: int = 5000,
    bbmd: Optional[Bbmd] = None,
    foreign_device: Optional[ForeignDevice] = None,
    multicast: Optional[Multicast] = None,
    sc_npdu_per_origin_limit: int | None = None,
    sc_control_rate_limit: tuple[int, int] | None = None,
    on_reinitialize: Optional[ReinitializeHandler] = None,
    after_reinitialize: Optional[AfterReinitialize] = None,
    router: Optional[BACnetRouter] = None,
) -> None
```

`dcc_password` and `reinit_password` hold 1 to 20
characters; a longer one (#2060) or an empty one (#2094) raises
ValueError here. None configures no password.

`transport="router"` with `router` (a `BACnetRouter` built
with `application="client_and_server"`): host the device on the
router's own application entity (#2071), answering at the router's
address on each of its networks, beside a `BACnetClient` on the
same router. `start()` takes the server's share from the running
router, once per router start; before the router starts, or when a
server already took it, it raises RuntimeError. Confirmed requests
to the router reach this server, except confirmed COV notifications,
which go to the client, and confirmed event notifications, which
reach both with only this server's answer sent; confirmed audit
notifications reach this server alone. Unconfirmed requests reach
both. A start refused before the server runs, such as for a
duplicate object name, leaves the share for the next start.
`apdu_segment_timeout_ms` must be under 60000. `router` without
`transport="router"`, or the reverse, raises ValueError.

`on_reinitialize` (default None): carry out ReinitializeDevice
(#1566). It is called with a `ReinitializeRequest` once a request
has passed its password and state checks, while the server holds its
object database, so it must return at once: no awaiting, no blocking,
and no other `rusty_bacnet` call (one would raise RuntimeError).
That check sees only its own thread: never wait on the event loop or
another thread from it (`run_coroutine_threadsafe(...).result()`,
`Thread.join`, any future's `.result()`), which deadlocks the
server. An `async def` handler is answered SERVICES / OTHER.
Return None or True to accept, False to refuse with SERVICES /
SERVICE\_REQUEST\_DENIED, or an `(ErrorClass, ErrorCode)` tuple to
refuse with those. An exception, or any other return value, answers
SERVICES / OTHER and goes to `sys.unraisablehook`. Without it every
request is refused. With it, a registered BBMD port's BDT and
foreign-device rows take writes, which ACTIVATE\_CHANGES or WARMSTART
puts into effect.

`after_reinitialize` (default None; needs `on_reinitialize`):
called with the `ReinitializeRequest` on the event loop that
started the server, once the accepted request's SimpleACK has been
handed to the transport. Restart, back up or restore here. It never
runs for a refused request, nor after a `stop()` that began before
the hand-off. A coroutine it returns runs as a task. On MS/TP, handed
off means queued: the frame still waits for the token, so stopping
at once can drop the reply; wait a short while first.

`share_port_by_address` (B/IP only, default False): bind the
interface address itself, so devices on other addresses of this host
can share the port. Needs an explicit interface and a nonzero port;
broadcasts and unicast are still handled in the order they arrived.

`bbmd` / `foreign_device` (at most one, else ValueError): run
the device as a BBMD (B/IP only), or register it with a BBMD on
another subnet as a foreign device (#938). `foreign_device` also
applies to `transport="ipv6"` with a B/IPv6 BBMD (#1941). See
`Bbmd` and `ForeignDevice`. A registered Network Port reports
the mode in BACnet\_IP\_Mode (normal 0, foreign 1, BBMD 2), or on
B/IPv6 in BACnet\_IPv6\_Mode (normal 0, foreign 1).

`registered_network_port` (B/IP or B/IPv6, 1 to 255): the Network
Port object, added with `add_bip_network_port` (`transport="bip"`)
or `add_bip6_network_port` (`transport="ipv6"`), that represents
the server's link. Start checks the object's address and UDP port
against the transport's, then publishes the bound port. A B/IPv6
port also publishes its VMAC and multicast group, and reports
BACnet\_IPv6\_Mode NORMAL (0), or FOREIGN (1) with `foreign_device=`
and its FD\_BBMD\_Address and FD\_Subscription\_Lifetime rows. With
`on_reinitialize`, a client may write those two rows; a
ReinitializeDevice ACTIVATE\_CHANGES then registers with the new
values at once.

`sc_npdu_per_origin_limit` (SC only, default None): maximum
queued, unconsumed NPDUs per ingress path and source. None keeps 32;
explicit values must be 1 through 256. Invalid values or a non-SC
transport raise ValueError before file or network I/O. Negative or
unrepresentably large values raise OverflowError. The aggregate
receive queue remains 256; see `sc_npdu_drop_counts`.

`sc_control_rate_limit` (SC only, default None): `(burst, per_second)`, the network-layer messages one source may send back to
back and then each second (#1969). None keeps 256 then 32 a second.
The burst must be 1 through 4096 and the rate at least 1; invalid
values or a non-SC transport raise ValueError before file or network
I/O, and negative values raise OverflowError. A message over its
source's budget is dropped and counted in `control_rate_drops`.

`mstp_extended_frames` (MS/TP only, default False): take and send
extended frames (Types 32 and 33), which carry NPDUs of up to 1497
octets, so the device accepts APDUs of up to 1476 instead of 480.

`mstp_echo` and `mstp_echo_grace_ms` (MS/TP only) declare an
adapter that returns every octet this station sends, as for
`BACnetClient`; a negative grace, or one above 4294967295, raises
OverflowError when the server is built.

`mstp_transmit_latency_pad_ms`, `mstp_usage_timeout_ms` and
`mstp_reply_timeout_ms` (MS/TP only) set the transmit-latency pad
and the two timeouts, in milliseconds, as for `BACnetClient`.

### BACnetServer.add_analog_input

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
) -> None
```

`tags`, `profile_location` and `profile_name` provision the
optional Tags, Profile\_Location and Profile\_Name rows; each is served
only when given. Tags takes writes over the network, the other two
are read-only. A tag name with a semicolon, a tag value that isn't
primitive, a location whose URI scheme isn't http, https or bacnet,
or a profile name without its vendor-number prefix (`"555-room"`)
raises BacnetProtocolError, and wrong shapes TypeError or
ValueError.

### BACnetServer.add_analog_output

```python
def add_analog_output(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

### BACnetServer.add_analog_value

```python
def add_analog_value(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Optional AV Audit rows: None is absent; priority 'inherit' is present NULL.

`present_value_access` is `"commandable"` (writes go through
Priority\_Array), `"writable"` (a write replaces Present\_Value) or
`"read_only"` (peers write it only while out of service).
`value_source_tracking` serves Value\_Source on a noncommandable
object: the writer of the last Present\_Value write, who alone may
then correct it. A commandable object tracks its sources through
Priority\_Array already, so asking it to track raises ValueError.
Profile keywords as on `add_analog_input`.

### BACnetServer.add_binary_input

```python
def add_binary_input(
    self,
    instance: int,
    name: str,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

### BACnetServer.add_binary_output

```python
def add_binary_output(
    self,
    instance: int,
    name: str,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

### BACnetServer.add_binary_value

```python
def add_binary_value(
    self,
    instance: int,
    name: str,
    *,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None,
    auditable_operations: int | None = None,
    audit_priority_filter: int | Literal["inherit"] | None = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Optional BV Audit rows: None is absent; priority 'inherit' is present NULL.
Access, Value\_Source and profile keywords as on `add_analog_value`.

### BACnetServer.add_multistate_input

```python
def add_multistate_input(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

### BACnetServer.add_multistate_output

```python
def add_multistate_output(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Profile keywords as on `add_analog_input`.

### BACnetServer.add_multistate_value

```python
def add_multistate_value(
    self,
    instance: int,
    name: str,
    number_of_states: int,
    *,
    present_value_access: Literal["commandable", "writable", "read_only"] = "commandable",
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Access, Value\_Source and profile keywords as on `add_analog_value`.

### BACnetServer.add_calendar

```python
def add_calendar(
    self,
    instance: int,
    name: str,
    *,
    date_list: Optional[list[CalendarEntry]] = None,
) -> None
```

`date_list` seeds Date\_List with calendar entries, the form a typed
read of it gives. An entry the Calendar refuses, such as a month
outside 1 to 14 or 255, raises BacnetProtocolError (VALUE\_OUT\_OF\_RANGE)
and nothing is registered; wrong shapes raise TypeError or
ValueError.

### BACnetServer.add_schedule

```python
def add_schedule(
    self,
    instance: int,
    name: str,
    *,
    weekly_schedule: Optional[list[list[TimeValue]]] = None,
    exception_schedule: Optional[list[SpecialEvent]] = None,
    effective_period: Optional[tuple[BACnetDate, BACnetDate]] = None,
) -> None
```

Add a Schedule object; Schedule\_Default is NULL.

The keywords take the forms typed reads of the properties give, so a
read's `.value` builds the same Schedule: `weekly_schedule` seven
days, Monday first, each a list of `(time, value)` pairs;
`exception_schedule` a list of `SpecialEvent` mappings;
`effective_period` a `(start_date, end_date)` pair, each date
specific or wholly unspecified (the default covers every date). What
the Schedule refuses, such as a time given twice in a day, a value
that isn't primitive or a priority outside 1 to 16, raises
BacnetProtocolError and nothing is registered.

### BACnetServer.add_date_value

```python
def add_date_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_time_value

```python
def add_time_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_date_time_value

```python
def add_date_time_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_date_pattern_value

```python
def add_date_pattern_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_time_pattern_value

```python
def add_time_pattern_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_date_time_pattern_value

```python
def add_date_time_pattern_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_notification_class

```python
def add_notification_class(
    self,
    instance: int,
    name: str,
    notification_class: int = 0,
    storage_path: Optional[str] = None,
    *,
    recipients: Optional[list[Destination]] = None,
) -> None
```

Add a Notification Class (Clause 12.21). With `storage_path`, a
Recipient\_List a client writes is kept in that file across restarts; a
write whose list cannot be saved is refused with DEVICE /
OPERATIONAL\_PROBLEM and the old list stays. Without it the list lives in
memory only.

`storage_path` is a `str`; a `pathlib.Path` raises TypeError, as
for `add_notification_forwarder`. Give each class a file of its own:
the file names the class it belongs to, so one that holds another
object's list, or anything this backend did not write, makes this call
raise BacnetError (BacnetProtocolError for a saved list a client's write
would be refused).

`recipients` seeds Recipient\_List with `Destination` mappings, in
order, with the checks and errors `add_notification_forwarder`'s
`recipients` has: more than 32, or an address MAC past 18 octets,
raises BacnetProtocolError, and nothing is registered. With
`storage_path`, a Recipient\_List a client wrote, once saved, wins:
until a write sets the list, the seed applies at every start and is
not saved.

### BACnetServer.add_notification_forwarder

```python
def add_notification_forwarder(
    self,
    instance: int,
    name: str,
    process_identifier_filter: Optional[int] = None,
    local_forwarding_only: bool = False,
    storage_path: Optional[str] = None,
    *,
    recipients: list[Destination] | None = None,
    port_filter: list[tuple[int, bool]] | None = None,
) -> None
```

Add a Notification Forwarder (Clause 12.51) that sends the event
notifications this server receives on to its Recipient\_List and
Subscribed\_Recipients. `process_identifier_filter=None` forwards every
process identifier. With `storage_path`, Recipient\_List and
Subscribed\_Recipients are kept in that file across restarts.

`recipients` seeds Recipient\_List in order; more than 32 destinations,
or an address MAC past 18 octets, raises BacnetProtocolError, as a
client's write would be refused. With `storage_path`, a
Recipient\_List a client wrote, once saved, wins over the seed. `port_filter` serves Port\_Filter as
`(port_id, enabled)` pairs; the server receives through Port\_ID 0.
Without it Port\_Filter is absent.

### BACnetServer.add_trend_log

```python
def add_trend_log(
    self,
    instance: int,
    name: str,
    buffer_size: int = 100,
    *,
    reference: tuple[ObjectIdentifier, PropertyIdentifier] | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] | DeviceObjectPropertyReference | None = None,
    log_interval: int | None = None,
    logging_type: Literal["polled", "triggered", "cov"] | None = None,
    client_cov_increment: float | None = None,
    cov_resubscription_interval: int | None = None,
    start_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
    stop_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
    align_intervals: bool | None = None,
    interval_offset: int | None = None,
    storage_path: Optional[str] = None,
    total_record_count: int = 0,
) -> None
```

Add a Trend Log (Clause 12.25) that the server polls, triggers or
logs by COV (before starting).

`reference` sets Log\_DeviceObjectProperty, one element in any form
`add_trend_log_multiple` takes for `members`. `log_interval`,
`start_time`, `stop_time`, `align_intervals` and
`interval_offset` are as there. `logging_type` `"polled"` with
no `log_interval` takes a one-minute interval; `"triggered"`
zeroes Log\_Interval and makes it read-only, so a `log_interval`
with it raises BacnetProtocolError (WRITE\_ACCESS\_DENIED); `"cov"`
zeroes it too, and a nonzero `log_interval` then turns the log back
to polled (Clause 12.25.9). Any other string raises ValueError, and
so does `"polled"` with `log_interval=0`, a pair that would make a
COV log; `log_interval=0` alone leaves the new log polled and idle.

While COV, the poller looks at the referenced property of this
device on each pass and logs it, with its object's Status\_Flags,
whenever it changes as a COV notification would report it. Enable
turned off and on again within one pass (about 100 ms) isn't seen.
`client_cov_increment` (a REAL) is the increment a numeric value is
held to; None, the default, is NULL: the object's own COV\_Increment
for a numeric Present\_Value, any change otherwise. A negative or
non-finite one raises BacnetProtocolError (VALUE\_OUT\_OF\_RANGE).
`cov_resubscription_interval` is in seconds, nonzero; it is served
but unused, since the server doesn't subscribe to other devices: a
reference to one logs a single OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED
failure.

Log\_DeviceObjectProperty reads as one
`device_object_property_reference` value whose mapping includes all
four keys; while unset its object is Analog Input
4194303's Present\_Value, and writing a reference to instance 4194303
unsets it. A null written to it succeeds and changes nothing.

`total_record_count` seeds Total\_Record\_Count, an Unsigned32, so
the log's first record is numbered one past it: seeded at
4294967294, records number 4294967295, then 1, 2 and on, which lets
a reader's handling of the wrap be tested. Out of range raises
OverflowError.

`storage_path` keeps the log in files, as on
`add_trend_log_multiple`. `client_cov_increment` and
`cov_resubscription_interval` aren't saved: they apply on every
start, reopened or not.

### BACnetServer.add_trend_log_multiple

```python
def add_trend_log_multiple(
    self,
    instance: int,
    name: str,
    buffer_size: int = 100,
    *,
    members: Optional[list[tuple[ObjectIdentifier, PropertyIdentifier] | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] | DeviceObjectPropertyReference]] = None,
    log_interval: int | None = None,
    logging_type: Literal["polled", "triggered"] | None = None,
    start_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
    stop_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
    align_intervals: bool | None = None,
    interval_offset: int | None = None,
    storage_path: Optional[str] = None,
    total_record_count: int = 0,
) -> None
```

Add a Trend Log Multiple (Clause 12.30) that the server polls or
triggers, logging one value per member in each record.

`members` fills Log\_DeviceObjectProperty in order, each an
`(object, property)` or `(object, property, array_index)` tuple
or a `DeviceObjectPropertyReference` mapping; one naming this
server's Device is kept in its local form. More than 64 raises
BacnetProtocolError (NO\_SPACE\_TO\_WRITE\_PROPERTY), and a
`device_identifier` that isn't a Device raises ValueError.
`log_interval` is in hundredths of a second. `logging_type`
`"polled"` with no `log_interval` takes a one-minute interval;
`"triggered"` zeroes Log\_Interval and makes it read-only, so a
`log_interval` with it raises BacnetProtocolError
(WRITE\_ACCESS\_DENIED). `"cov"` raises BacnetProtocolError
(VALUE\_OUT\_OF\_RANGE), as a client's write of COV does, since a Trend
Log Multiple never logs by COV; any other string raises ValueError. With neither, Log\_Interval stays 0 and
nothing is polled.

`start_time` and `stop_time` bound when records are kept, each a
`((full_year, month, day, day_of_week), (hour, minute, second, hundredths))` pair: every field 255 leaves that side open, 255
seconds or hundredths count as zero, and anything else that isn't an
actual date and time raises BacnetProtocolError (VALUE\_OUT\_OF\_RANGE). Records are kept from the
start up to, not including, the stop, and each opening and closing
is logged. `align_intervals` aligns a polled log's acquisitions to
the clock when Log\_Interval divides a day, shifted by
`interval_offset` hundredths (modulo Log\_Interval).

Peers can write each of these. To ask a triggered log for one
record, write Trigger TRUE, from a peer or with
`write_property_local`; it reads TRUE until the poller acquires
the record. Read the records with `read_range`.

`total_record_count` seeds Total\_Record\_Count as on
`add_trend_log`.

`storage_path` keeps the log in `<storage_path>.slot0` and
`.slot1`. It is a `str`; a `pathlib.Path` raises TypeError. The
log commits a snapshot every 100 records or, while it changes, once a
minute, and once more at `stop()`, so a power loss costs at most
about a minute of records; a failed commit is retried. A log reopened
from those files serves its saved records, Total\_Record\_Count,
BUFFER\_READY counts and Notification\_Threshold, Enable,
Stop\_When\_Full and every setting a peer can write, and appends one
LOG\_INTERRUPTED record to mark the restart (none when it reopens full
and stopped); the saved state wins over the keyword arguments.
`buffer_size` wins over the saved Buffer\_Size: a smaller one keeps
the newest records. A durable Trend Log Multiple holds at most 15849
records (178437 for an Event Log, 828302 for a Trend Log); a larger
`buffer_size`, or a file pair that can't be read or holds another
object's state, raises BacnetError. To tell whether a log will
reopen, check whether either slot file exists before the call. The
files outlive the log: one added again on them reopens.

### BACnetServer.add_event_log

```python
def add_event_log(
    self,
    instance: int,
    name: str,
    buffer_size: int = 100,
    log_received_notifications: bool = False,
    *,
    storage_path: Optional[str] = None,
    total_record_count: int = 0,
) -> None
```

Register an Event Log of `buffer_size` records before start().

The log records each event notification the server builds. With
`log_received_notifications` it also records the Confirmed and
UnconfirmedEventNotifications the server receives, unicast or
broadcast, as they decoded; each source is held to a few records a
second, and the rest show in `event_notification_counters()` as
`received_not_logged`.

The log reports BUFFER\_READY to its Notification Class once its
Notification\_Threshold is set (zero, the default, reports nothing);
write it and Notification\_Class with `write_property_local` or
from a peer.

`total_record_count` seeds Total\_Record\_Count as on
`add_trend_log`; BUFFER\_READY counts from the seeded value.

`storage_path` keeps the log in files, as on
`add_trend_log_multiple`. `log_received_notifications` isn't
saved and applies on every start.

### BACnetServer.add_audit_log

```python
def add_audit_log(
    self,
    instance: int,
    name: str,
    storage_path: str,
    buffer_size: int = 100,
) -> None
```

### BACnetServer.add_device_binding

```python
def add_device_binding(self, device_instance: int, address: str) -> None
```

Register a direct B/IP (IPv4) Device binding on a transport="bip" server.

Accept IPv4:port or exactly six hex bytes (four IPv4 octets and two port octets).
Other transports or parsed address lengths raise ValueError without retention.
The shared parser's broader grammar remains available to other APIs, not bindings.
Invalid instance/address or duplicate Device raises ValueError. No overwrite,
routing or discovery. Syntax validation precedes the frozen-state check; after
start() consumes configuration, parseable calls raise RuntimeError before the
transport/shape checks, including after stop. start() refuses a binding at a
broadcast or other group address of the link (a multicast address,
255.255.255.255, or the broadcast IP at another port), raising BacnetError
naming the device and the address; bind each device at its unicast address.

### BACnetServer.configure_audit_log_parent

```python
def configure_audit_log_parent(
    self,
    instance: int,
    *,
    parent_device_instance: int,
    parent_audit_log_instance: int,
) -> None
```

Set a registered Audit Log's remote parent; valid pre-start calls replace it.

Identifiers are integers in 0..=4194303, not bool. Invalid/missing/duplicate
local logs or a local parent Device raise ValueError without mutation.
Startup revalidates local identity; settings freeze at ownership transfer.
Reapply on a new server after reopen. Only the selected inbound sink forwards,
using a configured direct B/IP binding with the existing one-attempt confirmed
policy, never deletion or a durable queue. IPv6/SC/MS/TP forwarding is not exposed.

### BACnetServer.configure_audit_notification_sink

```python
def configure_audit_notification_sink(
    self,
    instance: int,
    *,
    policy: Literal["deny_all", "allow_all"],
) -> None
```

Select one registered Audit Log before start(); omission denies receipt.

Invalid policy/instance, missing or duplicate log raises ValueError without
mutation. Startup revalidates before transport preparation/registration
transfer. A running server raises RuntimeError. Payload Source\_Device and
Target\_Device remain peer-reported, not verified origin. No Python callbacks.

### BACnetServer.add_audit_reporter

```python
def add_audit_reporter(self, instance: int, name: str) -> None
```

### BACnetServer.configure_audit_recipient

```python
def configure_audit_recipient(self, recipient: AuditRecipientInput) -> None
```

Provision the copied Device-owned recipient before startup.

Accepts a concrete remote Device (not instance 4194303), or a supported
direct unicast IPv4 BACnetAddress on B/IP. The selected target Reporter
requires provision even at Audit\_Level NONE. Add Device routes with
add\_device\_binding independently. Missing Device routes start with
CONFIGURATION\_ERROR; live changes require usable old and new routes.
This setter freezes at ownership transfer. Use Device property writes
after start for atomic old/new delivery. No NULL recipient sentinel.

### BACnetServer.configure_audit_reporters

```python
def configure_audit_reporters(self, reporters: list[AuditReporterConfiguration]) -> None
```

Replace the complete target Reporter set before start (one through 64).

Each configuration identifies a registered concrete Reporter (0..4194302).
Invalid instance values (including bool and non-int), duplicate identities,
invalid settings or missing objects raise ValueError. Wrong container types
and wrong types for other setting fields raise TypeError. The entire input
is copied and validated before pending changes. Startup freezes this API.
Omitted optional fields reset to catch-all selectors, all priorities and
absent delay/control properties. maximum\_send\_delay accepts None or an
integer 0..3600: zero exposes immediate Maximum\_Send\_Delay/Send\_Now;
positive values enable bounded ordinary target batching. Bool/non-integer
delay raises TypeError; an integer outside unsigned32 raises
OverflowError and one past 3600 ValueError. An `instance`,
`auditable_operations` or `audit_priority_filter` outside its
integer type (unsigned32, unsigned64, unsigned16) raises OverflowError
too.
Enabled nominal overlaps expose CONFIGURATION\_ERROR on every affected
Reporter's RELIABILITY; the lowest instance emits, before operation filters.
Empty/all-None selectors select no nominal targets. Mandatory Reporter
writes and Device recipient changes have separate bounded capture rules.
All Reporters use the one provisioned Device recipient and share the same
64-operation admission budget. Source reporting remains exactly one.

### BACnetServer.add_loop

```python
def add_loop(
    self,
    instance: int,
    name: str,
    output_units: EngineeringUnits | int = 62,
    *,
    controlled_variable_units: Optional[EngineeringUnits | int] = None,
    proportional_constant_units: Optional[EngineeringUnits | int] = None,
    integral_constant_units: Optional[EngineeringUnits | int] = None,
    derivative_constant_units: Optional[EngineeringUnits | int] = None,
    priority_for_writing: Optional[int] = None,
) -> None
```

Add a Loop (PID) object to the server (before starting).

The keyword arguments set rows that are read-only over the network:
Controlled\_Variable\_Units and the three gain units rows (NO\_UNITS when
omitted) and Priority\_For\_Writing (16 when omitted). Units above 65535
or a priority outside 1..=16 raise BacnetProtocolError with
VALUE\_OUT\_OF\_RANGE; a priority outside 0..=255 raises OverflowError.
Peers can write Action (DIRECT until written).

Controlled\_Variable\_Reference and Manipulated\_Variable\_Reference read
as `application_data` holding the context-tagged reference; while
unset they name instance 4194303 (an Analog Input and an Analog Output
respectively), and writing a reference to that instance unsets them.
A null written to any Loop reference succeeds and changes nothing.

### BACnetServer.add_command

```python
def add_command(
    self,
    instance: int,
    name: str,
    *,
    action: Optional[list[list[ActionCommand]]] = None,
    action_text: Optional[list[str]] = None,
) -> None
```

Add a Command object; writing N to its Present\_Value runs action\[N-1\].

`action_text` serves Action\_Text and needs one text per list. A
command whose `device_identifier` isn't a Device raises ValueError.
A priority outside 1..=16, a value with no encoding, or a text count
that differs from the list count raises BacnetProtocolError with
VALUE\_OUT\_OF\_RANGE.

### BACnetServer.add_timer

```python
def add_timer(self, instance: int, name: str) -> None
```

### BACnetServer.add_load_control

```python
def add_load_control(self, instance: int, name: str) -> None
```

### BACnetServer.add_program

```python
def add_program(self, instance: int, name: str) -> None
```

### BACnetServer.add_lighting_output

```python
def add_lighting_output(
    self,
    instance: int,
    name: str,
    *,
    high_end_trim: Optional[float] = None,
    low_end_trim: Optional[float] = None,
    trim_fade_time: Optional[int] = None,
    color_reference: Optional[ObjectIdentifier] = None,
    color_override: Optional[bool] = None,
    override_color_reference: Optional[ObjectIdentifier] = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Add a Lighting Output object.

Its Lighting\_Command reads and writes as `application_data` holding
the context-tagged BACnetLightingCommand, operation NONE
(`b"\x09\x00"`) until written. An `octet_string` or any other
datatype is refused with INVALID\_DATA\_TYPE, and a command its operation
can't take with VALUE\_OUT\_OF\_RANGE. The object carries each command
out: a fade or ramp puts its level in Present\_Value at once and moves
Tracking\_Value there over time, with In\_Progress showing FADE\_ACTIVE
or RAMP\_ACTIVE; a step changes the level at once; and with
Blink\_Warn\_Enable TRUE the warn commands hold the level for
Egress\_Time seconds before relinquishing or turning it off.

A Present\_Value or Relinquish\_Default level above 0.0 and below 1.0
is stored as 1.0, and one outside 0.0 to 100.0 is refused with
VALUE\_OUT\_OF\_RANGE, but for Present\_Value's warn values -1.0 (WARN),
-2.0 (WARN\_RELINQUISH) and -3.0 (WARN\_OFF). Tracking\_Value reads the
same level as Present\_Value whenever no fade or ramp is running.

`high_end_trim` and `low_end_trim` (1.0 to 100.0 percent) serve
the trims that bound Tracking\_Value, and with either one
Trim\_Fade\_Time, set in milliseconds (0 to 86400000) by
`trim_fade_time`; without a trim none of the three rows is served.
A value out of range raises BacnetProtocolError (VALUE\_OUT\_OF\_RANGE).
The colour keywords are as on `add_binary_lighting_output`, and the
profile keywords as on `add_analog_input`.

### BACnetServer.add_binary_lighting_output

```python
def add_binary_lighting_output(
    self,
    instance: int,
    name: str,
    *,
    color_reference: Optional[ObjectIdentifier] = None,
    color_override: Optional[bool] = None,
    override_color_reference: Optional[ObjectIdentifier] = None,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Add a Binary Lighting Output object.

`color_reference` serves Color\_Reference, the Color or Color
Temperature object the output takes its colour from (instance
4194303 names none). `override_color_reference` adds Color\_Override
and Override\_Color\_Reference, and `color_override=True` turns the
override on; it needs an `override_color_reference`, and both need
a `color_reference` (ValueError otherwise). `color_override=False`
on its own means no override. A reference to another object type raises
BacnetProtocolError (VALUE\_OUT\_OF\_RANGE). All three rows take writes;
`lighting_color` follows them. Profile keywords as on
`add_analog_input`.

### BACnetServer.add_color

```python
def add_color(
    self,
    instance: int,
    name: str,
    *,
    present_value: Optional[tuple[float, float]] = None,
    default_color: Optional[tuple[float, float]] = None,
    storage_path: Optional[str] = None,
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Add a Color object: a CIE 1931 xy colour, D65 white
`(0.3127, 0.3290)` unless `present_value` gives an `(x, y)`
pair, each 0.0 to 1.0 (BacnetProtocolError otherwise). It reads as a
`list` of two `real` values. `value_source_tracking` serves
Value\_Source, the writer of the last Present\_Value or Color\_Command
that set it. Profile keywords as on `add_analog_input`.

At start the server drives the output to `default_color` (D65
unless given), at once, over any `present_value`. `(0.0, 0.0)`
keeps the colour from before the restart instead, kept in
`storage_path` (the file the object keeps Present\_Value in); with
nothing stored it keeps `present_value`, and with neither,
In\_Progress reads NOT\_CONTROLLED until a write drives the output.
A file that doesn't load raises BacnetError.

### BACnetServer.add_color_temperature

```python
def add_color_temperature(
    self,
    instance: int,
    name: str,
    *,
    present_value: Optional[int] = None,
    min_pres_value: Optional[int] = None,
    max_pres_value: Optional[int] = None,
    default_color_temperature: Optional[int] = None,
    storage_path: Optional[str] = None,
    value_source_tracking: bool = False,
    tags: Optional[list[NameValue]] = None,
    profile_location: Optional[str] = None,
    profile_name: Optional[str] = None,
) -> None
```

Add a Color Temperature object: a correlated colour temperature in
kelvin, 4000 unless `present_value` says otherwise.
`min_pres_value` and `max_pres_value` narrow the 1000 to 30000
range and come together (ValueError otherwise); `present_value` is
clamped to them. A value out of range raises BacnetProtocolError.
`value_source_tracking` is as on `add_color`.
`default_color_temperature` (4000 unless given, clamped to the
limits) and `storage_path` work as `default_color` and
`storage_path` on `add_color`, with 0 keeping the temperature
from before the restart.

### BACnetServer.add_channel

```python
def add_channel(
    self,
    instance: int,
    name: str,
    channel_number: int,
    members: Optional[list[tuple[ObjectIdentifier, PropertyIdentifier] | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] | DeviceObjectPropertyReference]] = None,
    execution_delay: Optional[list[int]] = None,
    control_groups: Optional[list[int]] = None,
    *,
    allow_group_delay_inhibit: bool = False,
) -> None
```

Add a Channel; a write of its Present\_Value is passed on to `members`.

`channel_number` (0..=65535) is the number a WriteGroup names. Each
member is an `(object, property)` or `(object, property, array_index)` tuple for a property in this device, or a
`DeviceObjectPropertyReference` mapping; one naming this server's
Device is kept in its local form, and one naming another Device is
written there through the server's device bindings
(`add_device_binding` or a heard I-Am). `execution_delay` holds
one delay in milliseconds per member (zeros when omitted),
`control_groups` the WriteGroup groups the Channel is in (`[0]`,
none, when omitted), and `allow_group_delay_inhibit` lets a
WriteGroup that asks for no delays skip them. All of them are
writable over the network too.

A wrong shape or type raises TypeError. An unknown or missing mapping
key or a device that isn't a Device raises ValueError. A channel
number outside unsigned16, or an index (a tuple's or a mapping's), a
delay or a group outside unsigned32, raises OverflowError. A delay
count that differs from the member count or an empty group list
raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE; more than
1024 members or 64 groups, NO\_SPACE\_TO\_WRITE\_PROPERTY. Nothing is
registered after any of them.

### BACnetServer.add_life_safety_point

```python
def add_life_safety_point(self, instance: int, name: str) -> None
```

### BACnetServer.add_life_safety_zone

```python
def add_life_safety_zone(self, instance: int, name: str) -> None
```

### BACnetServer.add_group

```python
def add_group(
    self,
    instance: int,
    name: str,
    members: Optional[list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]] = None,
) -> None
```

Group whose Present\_Value is rebuilt from `members` on each read.

`members` has the `read_property_multiple` spec shape and the
endpoint `add_group` checks: a member with no properties, or one
reporting a group's Present\_Value, raises ValueError naming its
position and the rule. Any property identifier is taken, those ASHRAE
assigns above 4194303 included. Indexes outside unsigned32 raise
OverflowError.

### BACnetServer.add_global_group

```python
def add_global_group(
    self,
    instance: int,
    name: str,
    *,
    members: Optional[list[tuple[ObjectIdentifier, PropertyIdentifier] | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]] | DeviceObjectPropertyReference]] = None,
) -> None
```

Add a Global Group object; `members` seeds Group\_Members, in the
form a typed read of it gives. A member naming this server's Device
is stored in its local form, and a device identifier that isn't a
Device raises ValueError.

### BACnetServer.add_structured_view

```python
def add_structured_view(self, instance: int, name: str) -> None
```

### BACnetServer.add_access_door

```python
def add_access_door(
    self,
    instance: int,
    name: str,
    *,
    door_members: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    alarm_values: Optional[list[int]] = None,
    fault_values: Optional[list[int]] = None,
    masked_alarm_values: Optional[list[int]] = None,
) -> None
```

Add an Access Door object to the server (before starting).

`door_members` sets Door\_Members, the objects that make up the door
(read-only over the network). Each element is an `ObjectIdentifier`
in this device or a `(device, object)` pair of identifiers for one
in another device. A pair whose device isn't a Device object
identifier raises ValueError, and nothing is registered.

`alarm_values`, `fault_values` and `masked_alarm_values` set the
starting Alarm\_Values, Fault\_Values and Masked\_Alarm\_Values, as
BACnetDoorAlarmState numbers other than NORMAL (1 to 8, or 256 to
65535); peers can write all three. Door\_Alarm\_State stays NORMAL or a member of the
alarm or fault values, never a masked state: a simulated value
outside them is refused, and masking the current state returns the
door to NORMAL. An alarm value raises a CHANGE\_OF\_STATE alarm after
Time\_Delay, and a fault value makes Reliability MULTI\_STATE\_FAULT. Any
other number, NORMAL (0) included, raises BacnetProtocolError with
VALUE\_OUT\_OF\_RANGE, and nothing is registered.

### BACnetServer.add_access_credential

```python
def add_access_credential(
    self,
    instance: int,
    name: str,
    *,
    assigned_access_rights: Optional[list[AssignedAccessRights]] = None,
    authorization_exemptions: Optional[list[int]] = None,
) -> None
```

Configure a credential before starting; invalid input registers nothing.

Assignments default to an empty array. Each mapping needs its reference
and an actual bool enable flag. Missing/unknown keys or a non-Device
device member raise ValueError; incorrect Python types raise TypeError.
Wrong object kinds raise BacnetProtocolError PROPERTY/VALUE\_OUT\_OF\_RANGE,
except instance 4194303, the existing unused marker.

Omitted/None exemptions leave the optional row absent; \[\] serves an
empty row. Integers 0..6 and 64..255 are accepted by the Rust setter.
Bool/non-int elements raise TypeError; values outside u32 raise
OverflowError; reserved values within u32 raise BacnetProtocolError
PROPERTY/VALUE\_OUT\_OF\_RANGE. Value 2 exempts this rights criterion only.

### BACnetServer.evaluate_access_rights

```python
def evaluate_access_rights(
    self,
    credential: ObjectIdentifier,
    point: ObjectIdentifier,
) -> Awaitable[AccessRightsEvaluation]
```

Inspect the native local rights stage (Clause 12.34.9.2).

Returns a plain mapping. Every key is present; access\_event and rule
are None where inapplicable. A rule index is its one-based position
in the positive/negative array. Unresolved assignments retain their
original one-based indices, order, duplicates and device references;
skipped entries do not compact later indices. Granted may coexist
with unresolved rights; exempt has no unresolved entries.

Granted means this stage passed, not final authorization. Exempt
applies only to this criterion (Clause 12.35.25). The denial event
is evidence, not a recorded Access\_Event. No remote reads, COV/event
reports or physical actions occur. Credential validity, point health,
threat, accompaniment, passback and occupancy remain separate checks.

Requires a started server (RuntimeError otherwise). Missing or wrong
object types raise BacnetProtocolError OBJECT/UNKNOWN\_OBJECT, checking
the credential first. Future unmapped result variants raise RuntimeError.

### BACnetServer.add_access_point

```python
def add_access_point(
    self,
    instance: int,
    name: str,
    *,
    access_doors: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    number_of_authentication_policies: Optional[int] = None,
    authentication_policies: Optional[list[tuple[str, AuthenticationPolicy]]] = None,
    supported_authorization_modes: Optional[list[int]] = None,
    priority_for_writing: Optional[int] = None,
) -> None
```

Add an Access Point object to the server (before starting).

`access_doors` sets Access\_Doors (read-only over the network), in the
element forms `add_access_door` takes for `door_members`. A pair
whose device isn't a Device object identifier raises ValueError, and a
reference to anything but an Access Door raises BacnetProtocolError
with VALUE\_OUT\_OF\_RANGE; either way nothing is registered.

`number_of_authentication_policies` (1 when omitted, never 0) and
`priority_for_writing` (16 when omitted, else 1 to 16; outside
0..=255 OverflowError) set Number\_Of\_Authentication\_Policies and
Priority\_For\_Writing, which are read-only over the network. `supported_authorization_modes` lists
the BACnetAuthorizationMode numbers the application carries out, the
values a write of Authorization\_Mode can take: AUTHORIZE (0) alone
when omitted, and AUTHORIZE must be in any list given; proprietary
modes run from 64 to 65535. A value outside those raises
BacnetProtocolError with VALUE\_OUT\_OF\_RANGE. Peers write
Active\_Authentication\_Policy (1 to the policy count) and
Authorization\_Mode (one of the supported modes).

`authentication_policies` sets Authentication\_Policy\_List and
Authentication\_Policy\_Names as `(name, policy)` pairs, both
read-only over the network, and the policy count to their number. A
policy's entries name the Credential Data Inputs and the step each
serves, from 1. An empty list, or more than 256 pairs, raises
BacnetProtocolError with VALUE\_OUT\_OF\_RANGE; an index or timeout
outside 0..=4294967295 raises OverflowError. A policy with no
entries, a reference to anything but a Credential Data Input, or
indexes that don't start at 1 and climb by at most one is kept but
can't be in effect: while it is, Active\_Authentication\_Policy reads 0
until a peer writes a usable policy. While any such policy is listed,
or the active policy is 0, Reliability reads CONFIGURATION\_ERROR and
the point takes no access events. A
`number_of_authentication_policies` given as well is applied after
and resizes both arrays, new policies empty and new names `""`; it
can be at most 256 with the pairs (VALUE\_OUT\_OF\_RANGE above).

### BACnetServer.add_access_rights

```python
def add_access_rights(
    self,
    instance: int,
    name: str,
    *,
    positive_access_rules: Optional[list[AccessRule]] = None,
    negative_access_rules: Optional[list[AccessRule]] = None,
    enable: bool = True,
    accompaniment: Optional[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]] = None,
    storage_path: Optional[str] = None,
) -> None
```

Add an Access Rights object to the server (before starting).

`positive_access_rules` and `negative_access_rules` set the two
rule arrays as `AccessRule` mappings, and `enable` sets Enable
(`PropertyIdentifier.LOG_ENABLE`, property 133), whose FALSE
disables every rule. Peers can write all three over the network. A
wrong shape or type raises TypeError, an unknown or missing key or a
device that isn't a Device raises ValueError, and a location naming
anything but an Access Point or Access Zone raises BacnetProtocolError
with VALUE\_OUT\_OF\_RANGE, and so does a list of more than 1024 rules
(NO\_SPACE\_TO\_WRITE\_PROPERTY). Nothing is registered after any of
them. The server stores and serves the rules. Applications can call
`evaluate_access_rights` to inspect the local rights stage.

`accompaniment` serves the optional Accompaniment row: the Access
Rights, Access Credential or Access User object that a second
credential, presented with the first, has to match. It takes an
`ObjectIdentifier` in this device or a `(device, object)` pair,
as `door_members` does; instance 4194303 asks for no
accompaniment. Left out, the object has no Accompaniment row and
clients can't add one; once served, clients can write it. A pair
whose device isn't a Device raises ValueError, and any other object
type raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE. A read
gives the reference back in the form the keyword takes.

With `storage_path`, a rule array, Enable or Accompaniment that a
client writes is kept in that file across restarts, and wins over the
keyword given for it at the next start, which is then checked but not
applied; a saved Accompaniment is served even without the keyword.
To lift a saved requirement, write the no-accompaniment reference
(instance 4194303), which keeps the row; to drop the row, remove the
storage file, which also drops the saved rules and Enable. A
write that cannot be saved is refused with DEVICE /
OPERATIONAL\_PROBLEM and the old value stays. Without it, written
values live in memory only. `storage_path` is a `str`; a
`pathlib.Path` raises TypeError. Give each object a file of its
own: one that holds another object's state, or anything this backend
did not write, makes this call raise BacnetError
(BacnetProtocolError for a saved rule or Accompaniment the object
would refuse).

### BACnetServer.add_access_user

```python
def add_access_user(
    self,
    instance: int,
    name: str,
    *,
    credentials: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    members: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    member_of: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
) -> None
```

Add an Access User object to the server (before starting).

`credentials` sets Credentials, the Access Credential objects the
user holds, and `members` and `member_of` set Members and
Member\_Of, the Access Users one level below and above this one in a
hierarchy of users. All three are read-only over the network and
take the element forms `add_access_door` takes for
`door_members`. A pair whose device isn't a Device object
identifier raises ValueError, and a reference to anything but an
Access Credential in `credentials`, or an Access User in the other
two, raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE; either way
nothing is registered. A read of a list gives the references back in
the forms the keywords take, or `[]` while it is empty.

### BACnetServer.add_access_zone

```python
def add_access_zone(
    self,
    instance: int,
    name: str,
    *,
    entry_points: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    exit_points: Optional[list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]] = None,
    alarm_values: Optional[list[int]] = None,
) -> None
```

Add an Access Zone object to the server (before starting).

`entry_points` and `exit_points` set Entry\_Points and Exit\_Points,
the Access Points leading into and out of the zone (read-only over
the network), in the element forms `add_access_door` takes for
`door_members`. A pair whose device isn't a Device object
identifier raises ValueError, and a reference to anything but an
Access Point raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE;
either way nothing is registered. A read of either list gives the
references back in the forms the keywords take.

`alarm_values` sets the starting Alarm\_Values, the occupancy states
the zone reports with a CHANGE\_OF\_STATE alarm after Time\_Delay, as
BACnetAccessZoneOccupancyState numbers other than NORMAL (1 to 6, or
64 to 65535); peers can write it too. Any other number, NORMAL (0)
included, raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE and
`first_failed_element_number` naming it, from 1, and nothing is
registered. Left out, the list starts empty.

### BACnetServer.add_credential_data_input

```python
def add_credential_data_input(
    self,
    instance: int,
    name: str,
    *,
    supported_formats: Optional[list[tuple[int | tuple[int, int | None, int | None], int]]] = None,
) -> None
```

Add a Credential Data Input object to the server (before starting).

`supported_formats` sets Supported\_Formats and
Supported\_Format\_Classes (read-only over the network) as
`(format, format_class)` pairs. A format is a
BACnetAuthenticationFactorType number, or a
`(format_type, vendor_id, vendor_format)` triple whose vendor
members may each be None (absent), as a read gives them; a CUSTOM
format (2) needs both. A format outside the closed production, a CUSTOM
format without its vendor members or a nonzero vendor member on
another format raises BacnetProtocolError with VALUE\_OUT\_OF\_RANGE; a
vendor member above 65535 raises OverflowError. While Out\_Of\_Service is TRUE a client's simulated
Present\_Value must name one of these formats with its class, or be
the UNDEFINED or ERROR factor with class 0.

### BACnetServer.add_alert_enrollment

```python
def add_alert_enrollment(
    self,
    instance: int,
    name: str,
    initial_source: ObjectIdentifier,
) -> None
```

### BACnetServer.add_event_enrollment

```python
def add_event_enrollment(
    self,
    instance: int,
    name: str,
    event_type: EventType = ...,
) -> None
```

Add an Event Enrollment (Clause 12.12) to the server (before starting).

Event\_Type follows Event\_Parameters: it reads NONE while they are
unset (the `none` alternative) and the written algorithm's type
after. `event_type` only applies to the legacy octet-string
Event\_Parameters, which name no algorithm: it is the algorithm they
run and the Event\_Type served while they are held.

Object\_Property\_Reference, read-only to peers, reads as one typed
`device_object_property_reference` mapping. It names Analog Input
4194303's Present\_Value while the enrollment has no reference.
Fault\_Parameters reads as the context-tagged `none` choice while no
fault algorithm is set, and writing that clears it; a null written to
it succeeds and changes nothing.

### BACnetServer.add_elevator_group

```python
def add_elevator_group(
    self,
    instance: int,
    name: str,
    machine_room_id: Optional[ObjectIdentifier] = None,
) -> None
```

Add an Elevator Group; machine\_room\_id must be a Positive Integer Value.

### BACnetServer.add_escalator

```python
def add_escalator(self, instance: int, name: str) -> None
```

### BACnetServer.add_lift

```python
def add_lift(self, instance: int, name: str, num_floors: int) -> None
```

### BACnetServer.add_staging

```python
def add_staging(
    self,
    instance: int,
    name: str,
    present_value: float,
    min_present_value: float,
    units: EngineeringUnits | int,
    priority_for_writing: int,
    stages: list[tuple[float, list[bool], float]],
    target_references: list[ObjectIdentifier],
    stage_names: list[str] | None = None,
) -> None
```

Add a validated local-target Staging object before start().

### BACnetServer.add_averaging

```python
def add_averaging(
    self,
    instance: int,
    name: str,
    *,
    window_interval: int | None = None,
    window_samples: int | None = None,
) -> None
```

Add an Averaging object before start().

`window_interval` sets Window\_Interval in seconds (900 when omitted)
and `window_samples` sets Window\_Samples (15 when omitted). Peers
can write both, and each write discards the samples. An interval of 0,
or a sample count of 0 or above 1440, raises VALUE\_OUT\_OF\_RANGE.

Object\_Property\_Reference reads as one
`device_object_property_reference` value whose mapping includes all
four keys; while unset its object is Analog Input
4194303's Present\_Value, and writing a reference whose object or
Device is at instance 4194303 unsets it. A null written to it succeeds
and changes nothing.

### BACnetServer.add_integer_value

```python
def add_integer_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_positive_integer_value

```python
def add_positive_integer_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_large_analog_value

```python
def add_large_analog_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_character_string_value

```python
def add_character_string_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_octet_string_value

```python
def add_octet_string_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_bit_string_value

```python
def add_bit_string_value(self, instance: int, name: str) -> None
```

### BACnetServer.add_accumulator

```python
def add_accumulator(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
    *,
    scale: float | int | None = None,
    prescale: Optional[tuple[int, int]] = None,
) -> None
```

Add an Accumulator (Clause 12.61) to the server (before starting).

`scale` sets Scale: a `float` is a float scale, sent as a
single-precision REAL that multiplies Present\_Value, and an `int`
an integer scale, the power of ten Present\_Value is multiplied by.
Left out, Scale is the float scale 1.0. `prescale`, a
`(multiplier, modulo_divide)` pair of unsigned32 values, serves the
optional Prescale; left out, the object doesn't serve it and a read
of it is UNKNOWN\_PROPERTY. Both read back as these values (tags
`"scale"` and `"prescale"`). A bool or another type raises
TypeError, an integer outside its type OverflowError, a float that
isn't finite as a REAL or a `prescale` of another length
ValueError; nothing is registered then.

### BACnetServer.add_pulse_converter

```python
def add_pulse_converter(
    self,
    instance: int,
    name: str,
    units: EngineeringUnits | int = 62,
) -> None
```

Add a Pulse Converter (Clause 12.23) to the server (before starting).

Input\_Reference reads as `application_data` holding the
context-tagged reference; while unset it reads as Accumulator
4194303's Present\_Value, and writing a reference to instance 4194303
unsets it. A null written to it succeeds and changes nothing.
While the reference names a missing object, or a property that isn't
an Unsigned or INTEGER, Reliability reads CONFIGURATION\_ERROR and
Status\_Flags FAULT; out of service, Reliability is writable for
simulation. The running server counts each increase of the named
property into Count at least once a second, across an Accumulator's
wrap at Max\_Pres\_Value.

### BACnetServer.add_file

```python
def add_file(
    self,
    instance: int,
    name: str,
    file_type: str = "application/octet-stream",
) -> None
```

### BACnetServer.set_file_access_method

```python
def set_file_access_method(self, /, instance: int, access_method: str) -> None
```

Select `"stream"` or `"record"` on a pending built-in File.

This synchronous operation is valid only before `start()`. Select the
access method before loading the corresponding payload; changing modes
does not convert stream data to records or vice versa.

### BACnetServer.set_file_data

```python
def set_file_data(self, /, instance: int, data: bytes) -> None
```

Copy bytes into a pending stream-access built-in File before `start()`.

### BACnetServer.get_file_data

```python
def get_file_data(self, /, instance: int) -> bytes
```

Return a fresh bytes copy from a pending stream File before `start()`.

### BACnetServer.set_file_records

```python
def set_file_records(self, /, instance: int, records: list[bytes]) -> None
```

Copy records into a pending record-access built-in File before `start()`.

### BACnetServer.get_file_records

```python
def get_file_records(self, /, instance: int) -> list[bytes]
```

Return a fresh list and fresh bytes from a pending record File before `start()`.

### BACnetServer.set_max_file_size

```python
def set_max_file_size(self, /, instance: int, max_octets: int) -> int
```

Set and return the effective octet growth cap before `start()`.

The built-in File clamp is authoritative. This does not truncate
preloaded content.

### BACnetServer.set_max_record_count

```python
def set_max_record_count(self, /, instance: int, max_records: int) -> int
```

Set and return the effective record growth cap before `start()`.

The built-in File clamp is authoritative. This does not truncate
preloaded records.

### BACnetServer.add_bip_network_port

```python
def add_bip_network_port(
    self,
    instance: int,
    name: str,
    *,
    ip_address: str = "0.0.0.0",
    udp_port: int = 47808,
    network_number: int = 0,
    apdu_length: int = 1476,
    subnet_mask: str = "0.0.0.0",
    default_gateway: str = "0.0.0.0",
    dns_servers: Optional[List[str]] = None,
) -> None
```

Add a configured, unbound IPV4 (B/IP) Network Port snapshot.

Every configuration row is read-only and nothing binds a socket.
Raises ValueError for an address that isn't IPv4 (`ip_address`,
`subnet_mask`, `default_gateway` or a DNS server);
BacnetProtocolError for an instance outside 1..255, a
`network_number` of 65535, an `apdu_length` under 50 or an empty
`dns_servers`; OverflowError for a number too large for its type;
RuntimeError once the server has started.

### BACnetServer.add_bip6_network_port

```python
def add_bip6_network_port(
    self,
    instance: int,
    name: str,
    *,
    ip_address: str = "::",
    udp_port: int = 47808,
    network_number: int = 0,
    apdu_length: int = 1476,
    prefix_length: int = 64,
    default_gateway: str = "::",
    dns_servers: Optional[List[str]] = None,
    multicast_address: str = "ff05::bac0",
    vmac: Optional[bytes] = None,
) -> None
```

Add a configured, unbound IPV6 (B/IPv6) Network Port snapshot.

The B/IPv6 counterpart of `add_bip_network_port`: every row is
read-only and nothing binds a socket. `vmac` is the 3-octet
MAC\_Address until a registered `transport="ipv6"` server publishes
the VMAC it uses; `multicast_address` is the broadcast group.
Raises ValueError for an address that isn't IPv6 or a `vmac` that
isn't 3 octets; BacnetProtocolError for an instance outside 1..255,
a `network_number` of 65535, an `apdu_length` under 50, an empty
`dns_servers`, a `prefix_length` outside 1..128, a
`multicast_address` that isn't multicast or an IPv4-mapped
`ip_address`; OverflowError for a number too large for its type;
RuntimeError once the server has started.

### BACnetServer.start

```python
def start(self) -> Awaitable[None]
```

Start the server and begin accepting BACnet requests.

### BACnetServer.stop

```python
def stop(self) -> Awaitable[None]
```

Join admitted work and release the owned transport.

Cancelling the returned Future retains shutdown ownership. Call stop()
again to join it; a cleanup error also retains the owner for retry.
Local mutation is rejected once shutdown starts.

A Notification Forwarder, Notification Class or Audit Log write staged
for a request that stop() cut short is dropped, and stop() waits, with
no limit, until every save those objects queued has run, so storage
holds what they serve. While storage holds it up, a warning naming the
objects still saving is logged after 5 s and every 30 s after that.

### BACnetServer.local_address

```python
def local_address(self) -> Awaitable[str]
```

Get the retained server instance's last bound address snapshot.

B/IP gives `"ip:port"`. B/IPv6 gives the VMAC peers address the
server by, `"bip6:0x0004D2"` for `device_instance=1234`: an ipv6
server's VMAC is its device instance (Clauses U.1.3, H.7.2), and
startup fails when another node on the link already uses it. MS/TP
gives the station, SC the colon-hex VMAC.

Available during interrupted shutdown; successful stop clears the
instance and subsequent address queries raise RuntimeError.

### BACnetServer.foreign_registration

```python
def foreign_registration(self) -> Awaitable[ForeignRegistrationStatus]
```

How the foreign-device registration is going (#938), as for
`BACnetClient`. RuntimeError before start and after stop.

### BACnetServer.read_property

```python
def read_property(
    self,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[PropertyValue]
```

Read a property from a local object through the server's
ReadProperty evaluator, the one network reads use.

The result equals what a network `read_property` of the same
property returns: a Group's Present\_Value is rebuilt from its
members, Device instance 4194303 names this server's Device, and the
Device's COV subscription lists are live. Its Device\_Address\_Binding
is a list with one `"address_binding"` element per device binding
the server holds (each `add_device_binding`, and each device whose
I-Am it heard in the last ten minutes): a mapping of
`device_identifier`, `network_number` (0 on this network) and
`mac_address`. An unknown object or
property raises `BacnetProtocolError` with the network error, and a
Group whose member rows exceed `rpm_max_result_elements` raises
`BacnetAbortError` (reason 9, OUT\_OF\_RESOURCES).

### BACnetServer.write_property_local

```python
def write_property_local(
    self,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
    *,
    source_object: Optional[ObjectIdentifier],
) -> Awaitable[None]
```

Write locally with an explicit source: None selects the server Device.

A source object must exist in the database. Tracked commands require a
concrete server Device; corrections retain the original Device owner.
The value is encoded and decoded as a network WriteProperty's would
be, so any value a network write takes works, including whatever
`read_property` returns, and array-index errors match the network's.

### BACnetServer.lighting_color

```python
def lighting_color(
    self,
    object_id: ObjectIdentifier,
) -> Awaitable[Optional[tuple[ObjectIdentifier, tuple[float, float] | int]]]
```

The colour the lighting output `object_id` shows now: the
Tracking\_Value of the colour object its Color\_Reference names, or its
Override\_Color\_Reference while Color\_Override is TRUE, so a colour
fade shows as it runs.

Returns `(source, colour)`, the colour object's identifier and an
`(x, y)` pair for a Color object or an `int` in kelvin for a
Color Temperature object, or `None` when the output has no
Color\_Reference, the reference in use names no object (instance
4194303), the server holds no colour object by that identifier, or
that object reads In\_Progress NOT\_CONTROLLED (nothing has driven it).
Raises RuntimeError before `start()`.

### BACnetServer.set_present_value_local

```python
def set_present_value_local(
    self,
    object_id: ObjectIdentifier,
    value: PropertyValue,
) -> Awaitable[None]
```

Update Present\_Value through the narrow, non-generic application Input authority.

Accepted values are a finite REAL for Analog Input and for a Loop's
control output, logical Enumerated 0/1 (INACTIVE/ACTIVE) for Binary Input,
and Unsigned 1..=Number\_Of\_States for Multi-state Input. A noncommandable
Analog, Binary or Multi-state Value takes the same datatype as its Input
counterpart; a commandable one raises OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED
(use `write_property_local` with a priority). Binary values are
BACnet logical values after Polarity, not raw hardware/interface levels.
An object with Out\_Of\_Service set rejects this update to preserve
network simulation ownership. A Life Safety Point or Zone takes an
Enumerated BACnetLifeSafetyState (standard, or 256..=65535) whether or
not Out\_Of\_Service is set, since clients never write its Present\_Value;
Tracking\_Value, Silenced and Operation\_Expected stay as they are. Other
object types are not writable through this method.

### BACnetServer.report_access_event_local

```python
def report_access_event_local(
    self,
    object_id: ObjectIdentifier,
    event: int,
    tag: int,
    *,
    time: Optional[BACnetTimeStamp] = None,
    credential: Optional[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]] = None,
    authentication_factor: Optional[tuple[int, int, bytes]] = None,
) -> Awaitable[None]
```

Report an access event at an Access Point the server holds.

Access\_Event (`event`, a BACnetAccessEvent number),
Access\_Event\_Tag (`tag`, the access transaction), Access\_Event\_Time
(`time`, the Device clock's when omitted), Access\_Event\_Credential
(`credential`, the no-credential reference when omitted) and
Access\_Event\_Authentication\_Factor (`authentication_factor` as
`(format_type, format_class, value)`, the UNDEFINED factor when
omitted) change together. A credential that isn't an Access
Credential, a factor format outside the closed production, or an
event that is neither a named BACnetAccessEvent nor a proprietary one
from 512 to 65535, raises VALUE\_OUT\_OF\_RANGE. While the point is out
of service, or its Reliability isn't NO\_FAULT\_DETECTED, the event is
refused with WRITE\_ACCESS\_DENIED and nothing changes. An unknown
object raises UNKNOWN\_OBJECT and any object other than an Access
Point OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. A new time sends the
point's COV report, and a time the point stamps itself always moves,
so events of one transaction each send one.

### BACnetServer.report_credential_read_local

```python
def report_credential_read_local(
    self,
    object_id: ObjectIdentifier,
    factor: tuple[int, int, bytes],
    *,
    update_time: Optional[BACnetTimeStamp] = None,
) -> Awaitable[None]
```

Report a factor a Credential Data Input's reader has read.

Present\_Value takes `factor`, `(format_type, format_class, value)`, and Update\_Time `update_time`, the Device clock's when
omitted, together. The factor must name one of the reader's
`supported_formats` with its class, or be the UNDEFINED (0) or
ERROR (1) factor with class 0, else VALUE\_OUT\_OF\_RANGE. While the
reader is out of service the read is kept aside, in place of the
reader's earlier one: a client's simulated values stay served, no
COV report goes out, and the return to service serves the latest
read. An unknown object raises UNKNOWN\_OBJECT and any object other
than a Credential Data Input OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. A
new Update\_Time sends the reader's COV report.

### BACnetServer.report_door_state_local

```python
def report_door_state_local(
    self,
    object_id: ObjectIdentifier,
    *,
    door_status: Optional[int] = None,
    lock_status: Optional[int] = None,
    door_alarm_state: Optional[int] = None,
) -> Awaitable[None]
```

Report an Access Door's hardware state.

Whichever of Door\_Status, Lock\_Status and Door\_Alarm\_State is given,
as BACnetDoorStatus, BACnetLockStatus and BACnetDoorAlarmState
numbers, changes together; the others keep their values. A number
outside its production, or an alarm state the door's Alarm\_Values,
Fault\_Values and Masked\_Alarm\_Values don't admit, raises
VALUE\_OUT\_OF\_RANGE and nothing changes. While the door is out of
service the values are kept aside, in place of the device's earlier
ones: a client's simulated values stay served, no COV report or event
follows, and the return to service serves the latest values
reported. An unknown object raises UNKNOWN\_OBJECT and any object
other than an Access Door OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. In
service a new Door\_Alarm\_State sends the door's COV report, and the
door's event algorithm sees it at once.

### BACnetServer.set_tracking_value_local

```python
def set_tracking_value_local(
    self,
    object_id: ObjectIdentifier,
    value: PropertyValue,
) -> Awaitable[None]
```

Set a Life Safety Point's or Zone's Tracking\_Value, the live state the application derived.

The value is an Enumerated BACnetLifeSafetyState, standard or from
256..=65535: another number raises VALUE\_OUT\_OF\_RANGE and another
datatype INVALID\_DATA\_TYPE. An unknown object raises UNKNOWN\_OBJECT and
any object other than a Life Safety Point or Zone
OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. Present\_Value, Silenced and
Operation\_Expected stay as they are, so an application that latches
Present\_Value until reset keeps reporting the live state here. While
Out\_Of\_Service is set the value is held for the return to service and
a client's simulated Tracking\_Value stays in place. In service a
SubscribeCOVProperty on Tracking\_Value is notified of the change; a
SubscribeCOV report doesn't carry it.

### BACnetServer.report_device_reset_local

```python
def report_device_reset_local(self) -> Awaitable[None]
```

Record a reset the application makes for its own reasons: a
DEVICE\_RESET Audit record from this device, with no invoke ID
(#1564). A peer's accepted restart is recorded already. RuntimeError
when the server is not running.

### BACnetServer.purge_audit_log

```python
def purge_audit_log(self, object_id: ObjectIdentifier) -> Awaitable[None]
```

Purge an Audit Log: clear its records and append a BUFFER\_PURGED status record.

No peer can purge an Audit Log, since its Record\_Count is read-only,
so this is the application's route. The record is appended whether or
not logging is enabled, and also carries LOG\_DISABLED while logging is
off; Total\_Record\_Count keeps counting. The purge reaches storage
before the log serves it. An unknown object
raises UNKNOWN\_OBJECT, any object other than an Audit Log
OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED, and a missing clock or a failed
commit DEVICE / OPERATIONAL\_PROBLEM, leaving the log as it was. A
server that is not running raises RuntimeError.

### BACnetServer.set_controlled_variable_value_local

```python
def set_controlled_variable_value_local(
    self,
    object_id: ObjectIdentifier,
    value: PropertyValue,
) -> Awaitable[None]
```

Feed a Loop's measured Controlled\_Variable\_Value while the application runs its algorithm.

The value must be a finite REAL: another datatype raises
INVALID\_DATA\_TYPE and NaN or an infinity VALUE\_OUT\_OF\_RANGE. An unknown
object raises UNKNOWN\_OBJECT and any object other than a Loop
OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. Unlike Present\_Value, the update
is accepted while Out\_Of\_Service is set. A SubscribeCOVProperty on the
property is notified of the change; a SubscribeCOV on the Loop is not,
and its next notification carries the new value. The property stays
read-only over the network.

### BACnetServer.add_averaging_sample_local

```python
def add_averaging_sample_local(
    self,
    object_id: ObjectIdentifier,
    value: PropertyValue | None,
) -> Awaitable[None]
```

Record one sample for an Averaging object, taken by the application.

The server samples an object holding an Object\_Property\_Reference
itself, every Window\_Interval / Window\_Samples seconds. For an object
without one (its reference reads as the unset form, instance
4194303), the application samples about that often and passes each
result here; on one the server samples, a call is one more attempt and
doesn't move the server's schedule. The value is a BOOLEAN (FALSE and
TRUE count as 0 and 1), Signed, Unsigned, Enumerated or finite REAL;
`None` records an attempt that produced no value, which counts
toward Attempted\_Samples but not Valid\_Samples. Another datatype,
Double included, raises INVALID\_DATA\_TYPE and NaN or an infinity
VALUE\_OUT\_OF\_RANGE, and a refused sample isn't counted. An unknown
object raises UNKNOWN\_OBJECT and any object other than an Averaging
object OPTIONAL\_FUNCTIONALITY\_NOT\_SUPPORTED. Each call fills the next
slot of the Window\_Samples window, dropping the oldest once it is
full, and Minimum\_Value, Maximum\_Value, Average\_Value and the sample
counts change together; with no valid sample in the window they read
positive infinity, negative infinity and NaN. A SubscribeCOVProperty
on one of them is notified when it changes (by the subscription's COV
increment, if it gave one); SubscribeCOV on an Averaging object is
refused.

### BACnetServer.comm_state

```python
def comm_state(self) -> Awaitable[EnableDisable]
```

Get the DeviceCommunicationControl state.

Returns `EnableDisable.ENABLE` or `EnableDisable.DISABLE_INITIATION`;
the server refuses the deprecated `DISABLE`, so it never reports it.
Raises RuntimeError before start and after stop.

### BACnetServer.cov_counters

```python
def cov_counters(self) -> Awaitable[CovCounters]
```

Sample COV subscription and notification telemetry; zero at each start.

Every field of the Rust CovCounters under the same name.
subscriptions\_active is a gauge; the rest are running totals.
timed\_changes\_dropped counts timestamped COV-multiple changes lost for
good, in whole or in part, the running signal for a subscriber whose
maximum APDU can't hold one timestamped value.
untimed\_references\_oversized counts each report that left out an
untimestamped reference too large for one notification. Raises
RuntimeError before start and after stop.

### BACnetServer.event_notification_counters

```python
def event_notification_counters(self) -> Awaitable[EventNotificationCounters]
```

Sample the totals of event notifications the server did not deliver.

Every field of the Rust EventNotificationCounters under the same name,
zero at each start. The recipient-list fields count transitions whose
Notification Class lookup failed closed: the class is missing, its
Recipient\_List can't be read or decoded, or it serves more than 32
destinations. An empty or fully filtered list, DCC and Event\_Enable are
not counted. The next three count matched destinations skipped while
their route was resolved: a Device recipient with no current binding,
a recipient that can't be routed as configured, and a confirmed
recipient at a group address (a broadcast or a multicast one). The
confirmed fields count
notifications to one recipient that found no free invoke ID, were
answered with an Error, Reject or Abort, or drew no acknowledgment
after the last retry. unconfirmed\_send\_failed counts unconfirmed
notifications the transport refused to send, once per destination.
apdu\_too\_large counts notifications not sent to one destination
because they exceed the local APDU size, received\_not\_forwarded
counts received event notifications no Notification Forwarder took,
and forwarding\_cap\_dropped counts destinations dropped because one
notification already had 64 across the forwarders.
Fields are sampled independently. Raises
RuntimeError before start and after stop.

### BACnetServer.forwarder_save_counters

```python
def forwarder_save_counters(self) -> Awaitable[dict[int, ForwarderSaveCounters]]
```

Sample each Notification Forwarder's save counters, keyed by instance.

The totals belong to the objects: they count from registration, and
a forwarder without `storage_path` stays at zero. Raises
RuntimeError before start and after stop.

### BACnetServer.dcc_outcome_counters

```python
def dcc_outcome_counters(self) -> Awaitable[DccOutcomeCounters]
```

Sample completed admitted DCC handlers; zero on each new server lifetime.

Accepted means state/timer commit, not response delivery. Independent
samples are not an atomic aggregate. Excludes incomplete cancellation,
duplicates, pre-handler rejection and timer expiry. No trace bridge or
subscriber installation. Raises RuntimeError before start/after stop.

### BACnetServer.sc_npdu_drop_counts

```python
def sc_npdu_drop_counts(self) -> Awaitable[ScNpduDropCounts]
```

Copy SC ingress counters; requires a started SC owner.

Available while disconnected. Each call returns an independent dictionary;
a new transport starts fresh counters. No network I/O is performed.

### BACnetServer.request_admission_counters

```python
def request_admission_counters(self) -> Awaitable[RequestAdmissionCounters]
```

Sample counters; RuntimeError before start and after stop.

Admitted totals count registered work, not successful response sends.

## ScHub

BACnet/SC Hub for relaying messages between SC nodes.

Usage:

```python
hub = ScHub("0.0.0.0:47809", "cert.pem", "key.pem", b"\x02\x00\x00\x00\x00\x01",
            ca_cert="ca.pem", device_uuid=provisioned_hub_uuid)
await hub.start()
print(await hub.url())
print(await hub.status())
print(await hub.shutdown_gracefully())  # "graceful" or "forced"
```

The hub also works as an async context manager (starts on entry,
forcefully stops on exit):

```python
async with ScHub(..., device_uuid=provisioned_hub_uuid) as hub:
    ...
```

`ca_cert` must name trusted issuer CA PEM certificates for mutual TLS.
Omitted, None, or empty values raise ValueError at construction. The None
default retains the existing positional layout only; there is no insecure
mode. Invalid/unreadable credentials raise BacnetError from start(), before
binding. Connections require TLS 1.3 and a valid trusted client certificate.

`device_uuid` is required, keyword-only and copied from bytes/bytearray into
owned storage. Missing/None, wrong length or all-zero UUID raises ValueError
before file I/O. The caller provisions this hosting device identity before
deployment and durably reuses the same bytes for its entire lifetime, including
stop/start and fresh objects. No generation/storage or UUID-bit validation is
provided. The hosting port's `vmac` must be six bytes (RuntimeError for wrong
length), neither all zero nor all ff (ValueError). CA presence remains first,
then VMAC validation, then UUID. No certificate-to-identity binding is implied.

Admission and timeout policy is constructor-validated, before bind:
`max_clients`/`max_handshakes` caps (zero/overflowing raise ValueError,
negative values raise OverflowError); `admission_policy` is the static
string `"allow_all"` (default), `"deny_all"` or
`"deny_uuid_replacement"` (unknown strings raise
ValueError, non-strings including callables raise TypeError — no Python
callback can run under the native registry lock). Replacement refusal is a
local security policy before protocol acceptance: it preserves an incumbent
when the same claimed UUID requests its current or another VMAC. Default
mode retains Annex AB replacement; a different-UUID VMAC collision keeps its
standard NAK. UUID equality is not certificate identity proof. Graceful per-peer ack /
close / overall millisecond bounds and handshake TLS / WebSocket-upgrade /
Connect-Request millisecond bounds (out-of-range values raise ValueError).
Optional Hub probes use `probe_scan_interval_ms` / `probe_idle_age_ms` /
`probe_ack_age_ms` / `probe_send_budget_ms` (defaults 30000/60000/5000/5000).
Strictly exceeded ages are checked at scans; ACK age starts at reservation,
not completed send, and is not a hard closure deadline. Delayed ticks are
skipped. Only matching valid ACKs refresh activity. Node keepalive is separate.
A connector need not answer probes (#2141): an unanswered probe retires it
only after `probe_liveness_bound_ms` (default 360000, at most 86400000)
passes with nothing received from it since the probe, and is not repeated
until it sends something. Its own Heartbeat-Request keeps it registered; a
dead connector lingers that much longer.
`relay_send_budget_ms` bounds each NPDU/opaque unicast, concurrent broadcast
recipient and forwarded BVLC-Result acquisition+send attempt (default 5000),
without timeout-driven retirement, retry or fabricated Result. Probe, control,
cleanup and graceful shutdown stay separate. Unicast counters keep their scope.
These values must be positive whole milliseconds \<=2\*\*63-1 and fit the native
monotonic clock. Broadcast sender/global burst/per-second settings use native
token buckets: bounds 1..=(2\*\*64-1)//1\_000\_000\_000, silent drops, existing counters.
All are constructor-validated before I/O. Negative/over-u64 integers raise
OverflowError, nonintegers TypeError, other invalid bounds ValueError.
`certificate_bindings=None` retains CA-valid admission. A nonempty group
sequence enables mapped-only UUID/VMAC reservations, including offline nodes;
existing admission\_policy can further deny. Shared native validation rejects
overlapping ownership and local Hub VMAC conflicts before PEM I/O/bind.
Inputs are copied; status/repr/errors do not expose certificate contents.
This is installation policy, not end-to-end relayed authentication.
`stop()` is forceful and idempotent; `shutdown_gracefully()` runs the
Disconnect/Ack/close exchange and consumes the hub; dropping the hub
without awaiting close only seals admission and cannot guarantee cleanup.

### ScHub.__init__

```python
def __init__(
    self,
    listen: str,
    cert: str,
    key: str,
    vmac: bytes,
    ca_cert: Optional[str] = None,
    *,
    device_uuid: Optional[Union[bytes, bytearray]] = None,
    max_clients: int = 256,
    max_handshakes: int = 256,
    admission_policy: Literal["allow_all", "deny_all", "deny_uuid_replacement"] = "allow_all",
    graceful_disconnect_ack_ms: int = 5000,
    graceful_ws_close_ms: int = 5000,
    graceful_overall_ms: int = 15000,
    handshake_tls_ms: int = 10000,
    handshake_websocket_upgrade_ms: int = 10000,
    handshake_connect_request_ms: int = 10000,
    probe_scan_interval_ms: int = 30000,
    probe_idle_age_ms: int = 60000,
    probe_ack_age_ms: int = 5000,
    probe_send_budget_ms: int = 5000,
    probe_liveness_bound_ms: int = 360000,
    broadcast_sender_burst: int = 1024,
    broadcast_sender_per_second: int = 128,
    broadcast_global_burst: int = 4096,
    broadcast_global_per_second: int = 512,
    relay_send_budget_ms: int = 5000,
    certificate_bindings: Sequence[ScHubCertificateBinding] | None = None,
) -> None
```

### ScHub.start

```python
def start(self) -> Awaitable[None]
```

Start the SC hub.

### ScHub.stop

```python
def stop(self) -> Awaitable[None]
```

Stop the SC hub (forceful, idempotent).

### ScHub.shutdown_gracefully

```python
def shutdown_gracefully(self) -> Awaitable[Literal["graceful", "forced"]]
```

Graceful shutdown; consumes the hub (RuntimeError if not started).

### ScHub.status

```python
def status(self) -> Awaitable[ScHubStatus]
```

Bounded redacted snapshot (RuntimeError before start/after stop).

### ScHub.__aenter__

```python
def __aenter__(self) -> Awaitable[ScHub]
```

### ScHub.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

### ScHub.address

```python
def address(self) -> Awaitable[Optional[str]]
```

Get the address the hub is listening on (None before start).

### ScHub.url

```python
def url(self) -> Awaitable[Optional[str]]
```

Get the WebSocket URL of the hub (None before start).

## ReinitializeRequest

A ReinitializeDevice request handed to `on_reinitialize` and
`after_reinitialize` (#1566). The addresses are what the request
claims, not an authenticated identity. Not copyable or picklable.

Properties:

- `state: ReinitializedState`: The requested state.
- `source_mac: bytes`: The immediate link source: the requester, or the last router.
- `source_network: Optional[int]`: The originating network of a routed request, else None.
- `source_address: Optional[bytes]`: The originating MAC of a routed request, else None.
- `invoke_id: int`

## MutationContext

One mutation a `mutation_policy` callable is asked to allow
(#1966): a frozen, redacted copy of the request. It holds the target's
coordinates, never written values or file data. The addresses are what
the request claims, not an authenticated identity. Its repr shows the
service and invoke ID only. Not copyable or picklable.

Properties:

- `service: Literal['write_property', 'write_property_multiple', 'create_object', 'delete_object', 'add_list_element', 'remove_list_element', 'atomic_write_file', 'subscribe_cov', 'subscribe_cov_property', 'subscribe_cov_property_multiple', 'write_group']`: The service. WritePropertyMultiple asks once per element and WriteGroup once per Channel.
- `object_identifier: Optional[ObjectIdentifier]`: The target object (the Channel for WriteGroup, the File for AtomicWriteFile); None for a CreateObject that names only a type and for SubscribeCOVPropertyMultiple.
- `object_type: Optional[ObjectType]`: The target's type, also for a CreateObject by type; None for SubscribeCOVPropertyMultiple.
- `property_identifier: Optional[PropertyIdentifier]`: The property written, listed or subscribed to, else None.
- `array_index: Optional[int]`
- `priority: Optional[int]`: The write priority of a WriteProperty, a WritePropertyMultiple element or a WriteGroup, when there is one.
- `source_mac: bytes`: The immediate link source: the requester, or the last router.
- `source_network: Optional[int]`: The originating network a routed request claims, else None.
- `source_address: Optional[bytes]`: The originating MAC a routed request claims, else None.
- `provenance: Literal["unverified", "relayed_origin", "hub_channel", "direct_peer"]`: How the request arrived: `unverified` for B/IP, B/IPv6, MS/TP and Ethernet, `relayed_origin` through a BACnet/SC hub.
- `principal: Optional[bytes]`: The SHA-256 of the originating peer's verified leaf certificate, for a direct BACnet/SC connection without a routed source. A Python server has no direct SC listener, so this is None.
- `forwarding_peer_principal: Optional[bytes]`: The SHA-256 of the forwarding direct SC peer's leaf certificate, a router included; None, like `principal`, for a Python server.
- `certificate: None`: Always None: a Python server has no direct BACnet/SC listener, and hub-relayed, routed and non-SC requests carry no certificate.
- `forwarding_peer_certificate: None`: Always None, for the reason `certificate` gives.
- `invoke_id: Optional[int]`: A confirmed request's invoke ID; None for WriteGroup.

## ScHubCertificateBinding

Frozen installation group; exact leaf DER SHA-256, with redacted repr.

Inputs are copied. UUID must be nonzero; VMAC and digest lists must be
nonempty and distinct; reserved VMACs are invalid. Raises ValueError.

Properties:

- `uuid: bytes`
- `allowed_vmacs: tuple[bytes, ...]`
- `leaf_sha256: tuple[bytes, ...]`

### ScHubCertificateBinding.__init__

```python
def __init__(
    self,
    *,
    uuid: bytes,
    allowed_vmacs: Sequence[bytes],
    leaf_sha256: Sequence[bytes],
) -> None
```

## CovCounters

`class CovCounters(TypedDict)`

COV telemetry, zero at each start; fields are sampled one by one, not atomically.

Keys:

- `subscriptions_active: int`
- `subscriptions_created: int`
- `subscriptions_rejected_quota: int`
- `subscriptions_rejected_capacity: int`
- `subscriptions_rejected_indefinite: int`
- `subscriptions_cancelled: int`
- `subscriptions_purged: int`
- `notifications_sent: int`
- `notifications_confirmed: int`
- `notifications_unconfirmed: int`
- `notification_bytes_sent: int`
- `notifications_throttled_fanout: int`
- `notifications_throttled_peer: int`
- `timed_changes_dropped: int`
- `untimed_references_oversized: int`

## CovPolicy

`class CovPolicy(TypedDict, total=False)`

BACnetServer(cov\_policy=...) COV limits, checked at construction; omitted keys keep their defaults.

Keys:

- `max_subscriptions_global: int`
- `max_subscriptions_per_peer: int`
- `reserved_capacity: int`
- `reserved_peers: list[bytes]`
- `reserved_recipients: list[tuple[int | None, bytes]]`
- `allow_indefinite_subscriptions: bool`
- `max_indefinite_per_peer: int`
- `max_notifications_per_event: int`
- `max_notification_bytes_per_event: int`
- `max_confirmed_in_flight_per_peer: int`

## TimeSyncPolicy

`class TimeSyncPolicy(TypedDict, total=False)`

BACnetServer(time\_sync\_policy=...) inbound clock limits, checked at construction; omitted keys keep their defaults.

Keys:

- `enabled: bool`
- `source_restriction: list[tuple[int | None, bytes]] | None`
- `max_step_ms: int | None`
- `per_source_rate: tuple[float, int] | None`
- `global_rate: tuple[float, int] | None`
- `coalesce_window_ms: int`
- `global_coalesce_window_ms: int`
- `max_sources: int`

## ForwarderSaveCounters

`class ForwarderSaveCounters(TypedDict)`

One Notification Forwarder's Subscribed\_Recipients save totals, saturating at 2\*\*64-1.

Keys:

- `failed_saves: int`

## EventNotificationCounters

`class EventNotificationCounters(TypedDict)`

Undelivered event notifications, zero at each start; independent u64 totals, saturating at 2\*\*64-1.

Keys:

- `notification_class_missing: int`
- `recipient_list_unavailable: int`
- `recipient_list_invalid: int`
- `recipient_list_too_long: int`
- `device_recipient_unbound: int`
- `recipient_unroutable: int`
- `confirmed_broadcast_recipient: int`
- `confirmed_no_invoke_id: int`
- `confirmed_rejected: int`
- `confirmed_unanswered: int`
- `unconfirmed_send_failed: int`
- `apdu_too_large: int`
- `received_not_forwarded: int`
- `forwarding_cap_dropped: int`
- `received_not_logged: int`

## DccOutcomeCounters

`class DccOutcomeCounters(TypedDict)`

Independent u64 lifetime totals, saturating at 2\*\*64-1; not an audit log.

Keys:

- `accepted_total: int`
- `policy_denied_total: int`
- `password_failure_total: int`
- `deprecated_denied_total: int`
- `malformed_total: int`

## ScNpduDropCounts

`class ScNpduDropCounts(TypedDict)`

Owned, queue-wide lifetime counters for one started SC transport.

Keys:

- `control_rate_drops: int`
- `fairness_drops: int`
- `full_drops: int`
- `closed_drops: int`

## RequestAdmissionCounters

`class RequestAdmissionCounters(TypedDict)`

Independent counter samples, not a transactionally atomic aggregate.

Keys:

- `recovery_active: int`
- `recovery_admitted_total: int`
- `recovery_overloaded_total: int`
- `confirmed_active: int`
- `confirmed_admitted_total: int`
- `confirmed_overloaded_total: int`
- `confirmed_global_overloaded_total: int`
- `confirmed_peer_overloaded_total: int`
- `confirmed_shutdown_rejected_total: int`
- `unconfirmed_active: int`
- `unconfirmed_admitted_total: int`
- `unconfirmed_overloaded_total: int`
- `unconfirmed_global_overloaded_total: int`
- `unconfirmed_peer_overloaded_total: int`
- `unconfirmed_shutdown_rejected_total: int`
- `abort_active: int`
- `abort_admitted_total: int`
- `confirmed_fallback_dropped_total: int`
- `abort_shutdown_rejected_total: int`

## ScHubOutcomeCounts

`class ScHubOutcomeCounts(TypedDict)`

Per-start saturating outcomes; selected refusals are not delivered NAK counts.

Keys:

- `uuid_replacements: int`
- `vmac_collision_rejections: int`
- `registered_capacity_rejections: int`
- `total_active_accept_drops: int`
- `handshake_accept_drops: int`
- `tls_timeouts: int`
- `websocket_timeouts: int`
- `connect_timeouts: int`
- `unicast_no_target: int`
- `unicast_target_limit: int`
- `unicast_send_timeout: int`
- `unicast_send_error: int`
- `heartbeat_retirements: int`

## ScHubStatus

`class ScHubStatus(TypedDict)`

Bounded hub snapshot: counts and kind labels only (no keys/VMAC maps).

Keys:

- `listening: bool`
- `max_clients: int`
- `max_handshakes: int`
- `client_count: int`
- `handshake_count: int`
- `admin_denied: int`
- `broadcast_sender_exhausted: int`
- `broadcast_global_exhausted: int`
- `outcomes: ScHubOutcomeCounts`

## ReinitializeHandler

```python
ReinitializeHandler = Callable[
    [ReinitializeRequest], Union[None, bool, tuple[ErrorClass, ErrorCode]]
]
```

## AfterReinitialize

```python
AfterReinitialize = Callable[[ReinitializeRequest], Optional[Awaitable[None]]]
```

## MutationHandler

```python
MutationHandler = Callable[[MutationContext], bool]
```
<!-- generated:end -->
