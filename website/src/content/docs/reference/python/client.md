---
title: "Python client"
description: "BACnetClient and its options, results and SC credential inspection, generated from the type stub."
---

:::note[On `dev` after the release]
This page is generated from the type stub on the `dev` branch and tracks the next release, so it can list classes, methods and keywords merged after v0.15.0 that the released wheel doesn't have. The [v0.15.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) and the [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d) say what changed.
:::

The client, its options and what its methods return. The [Python reference](/rusty-bacnet/reference/python/) lists every group. Each page opens with its main classes, then the other classes, the dictionaries (`TypedDict`) and the type aliases, in stub order.

<!-- generated:start by website/scripts/reference-python.py; do not edit this page -->

## BACnetClient

Async BACnet client for reading/writing properties on remote devices.

Supports BACnet/IP (`"bip"`), BACnet/IPv6 (`"ipv6"`),
BACnet/SC (`"sc"`), and BACnet MS/TP (`"mstp"`) transports, and a
`BACnetRouter`'s own application entity (`"router"`, #2071).

For SC, sc\_ca\_cert, sc\_client\_cert, and sc\_client\_key must be nonempty
paths; omission, None, or empty strings raise ValueError at construction.
Their None defaults preserve non-SC use and positional layout only.
Async entry loads the explicit site CA and operational cert/key; invalid
TLS configuration raises RuntimeError before dialing. No system-root or
unauthenticated-client fallback is available.

SC also requires keyword-only sc\_device\_uuid: exactly 16 bytes, not all zero,
copied at construction. Missing, None, wrong length, or all-zero values
raise ValueError after credential-presence validation, before file/network I/O.
Provision before deployment and durably reuse the same UUID for the device's
lifetime. No generation, persistence, change detection, or version/variant
enforcement is provided. Non-SC transports ignore this option.

Keyword-only reconnect=None preserves the default without automatic
redial. Supply ScReconnect for SC retries with the same loaded TLS policy,
VMAC, UUID and heartbeat settings. Other transports reject this option.

stop() and context exit refuse new work, drain outstanding native operations
and join teardown. Cancelling the close Future leaves shutdown running;
await stop() again to join it. Drain duration depends on outstanding requests.
Entry is exclusive; entering an already running or closing client fails.

Usage:

```python
async with BACnetClient() as client:
    await client.who_is()
    devices = await client.discovered_devices()
    value = await client.read_property("192.168.1.100:47808", oid, pid)
    print(value.tag, value.value)

# MS/TP peer address is a station MAC string ("7" or "mstp:7")
async with BACnetClient(transport="mstp", serial_port="/dev/ttyUSB0", mstp_mac=3) as client:
    value = await client.read_property("7", oid, pid)

# A B/IPv6 peer is named by its VMAC: "bip6:1234" for device 1234,
# "bip6:0x40ABCD" for any VMAC. resolve_vmac finds it from "[ip]:port".
# ipv6_device_instance gives the client a stable VMAC of its own.
async with BACnetClient(transport="ipv6", port=0, ipv6_device_instance=4321) as client:
    vmac = await client.resolve_vmac("[2001:db8::10]:47808")
    value = await client.read_property(vmac, oid, pid)
```

Every `address` takes `"ip:port"` (B/IP), `"bip6:<device instance>"` or `"bip6:0x<6 hex digits>"` (a B/IPv6 VMAC, ipv6 only),
colon hex such as `"01:02:03:04:05:06"` (SC, or `"00:04:d2"` for a
VMAC), or an MS/TP station. `"[ip]:port"` raises ValueError: use
`resolve_vmac`. So does a `bip6:` address on any other transport.

### BACnetClient.__init__

```python
def __init__(
    self,
    interface: str = "0.0.0.0",
    port: int = 0xBAC0,
    broadcast_address: str = "255.255.255.255",
    apdu_timeout_ms: int = 6000,
    transport: str = "bip",
    sc_hub: Optional[str] = None,
    sc_vmac: Optional[bytes] = None,
    sc_ca_cert: Optional[str] = None,
    sc_client_cert: Optional[str] = None,
    sc_client_key: Optional[str] = None,
    sc_heartbeat_interval_ms: Optional[int] = None,
    sc_heartbeat_timeout_ms: Optional[int] = None,
    ipv6_interface: Optional[str] = None,
    *,
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
    sc_device_uuid: Optional[bytes | bytearray] = None,
    share_port_by_address: bool = False,
    min_request_interval_ms: int = 0,
    ipv6_device_instance: Optional[int] = None,
    bbmd: Optional[Bbmd] = None,
    foreign_device: Optional[ForeignDevice] = None,
    multicast: Optional[Multicast] = None,
    reconnect: Optional[ScReconnect] = None,
    sc_npdu_per_origin_limit: int | None = None,
    sc_failover_hub: str | None = None,
    sc_connect_timeout_ms: int | None = None,
    sc_control_rate_limit: tuple[int, int] | None = None,
    router: Optional[BACnetRouter] = None,
) -> None
```

`transport="router"` with `router` (a `BACnetRouter` built
with `application="client"` or `"client_and_server"`): run the
client on the router's own application entity (#2071), with the
router's address on each network as its own. Entry takes the client's
share from the running router, once per router start: before the
router starts, or when a client already took it, entry raises
RuntimeError. Requests go out through the router's table, so name
each device by its network: discover it, then use the
`_from_device` / `_to_device` methods. A send to an `ip:port`
names no network and raises BacnetError. Once the router closes,
each request fails at once with BacnetTransportError (errno
ENOTCONN); stop the client as usual. With
`application="client_and_server"` the APDU timeout must be under
60000 ms. `router` without `transport="router"`, or the reverse,
raises ValueError.

`sc_failover_hub` (SC only, needs `reconnect`, else
ValueError): a second hub's `wss://` URL, dialled with the same TLS
settings when the primary can't be reached; the client keeps trying
the primary and returns to it. `sc_connect_timeout_ms` (SC only, at
least 1, default 10000) bounds each hub dial and Connect-Request.
Follow the link with `connection_events()` and
`connection_status()` (#1829).

`bbmd` / `foreign_device` (at most one, else ValueError): run
the client as a BBMD (B/IP only), or register it with a BBMD on
another subnet as a foreign device, so its Who-Is reaches that
subnet (#938). `foreign_device` also applies to
`transport="ipv6"` with a B/IPv6 BBMD, and then needs
`ipv6_device_instance` (#1941). See `Bbmd` and
`ForeignDevice`.

`ipv6_device_instance` (B/IPv6 only, default None): the device
instance whose VMAC the client uses (Clause H.7.2), 0 to 4194303, so
peers know it by a stable VMAC such as `bip6:0x0010E1`. Give a
client an instance to get a stable VMAC; without one it picks a
random VMAC at each start, and that needs a multicast-capable link at
start, which Linux loopback is not. Another transport, or an instance
out of range, raises ValueError.

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
octets, so APDUs of up to 1476; without it MS/TP carries 480-octet
APDUs and counts an extended frame it hears as invalid.

`mstp_echo` (MS/TP only, default False): the adapter returns every
octet this station sends (a transceiver whose receiver stays on while
it transmits), so the transport drops that echo rather than take it
for line traffic. `mstp_echo_grace_ms` (default None, 40 ms; needs
`mstp_echo=True` and at least 1, else ValueError at start) is how
long after a frame its echo may still be arriving. A negative value,
or one above 4294967295, raises OverflowError when the client is
built.

`mstp_transmit_latency_pad_ms`, `mstp_usage_timeout_ms` and
`mstp_reply_timeout_ms` (MS/TP only, default None, the transport's
own): the time added to a frame's wire time on a port that can't
report its end (4 ms on Windows, where the port has none, 0 elsewhere;
at most 15), and T\_usage\_timeout (20 to 35, default 35) and
T\_reply\_timeout (255 to 300, default 300), in milliseconds. A value
outside those ranges raises ValueError at start, before the port
opens.

`share_port_by_address` (B/IP only, default False): bind the
interface address itself, so clients and devices on other addresses
of this host can share the port. Needs an explicit interface and a
nonzero port; broadcasts and unicast are still handled in the order
they arrived.

`min_request_interval_ms` (default 0, no pacing; at most
3,600,000, else ValueError) paces the confirmed requests to each
destination: each waits until that long after the latest one sent to
the same destination finished (reply, error or the caller giving up),
or after it was sent while it is still outstanding, checking again
when it wakes. Requests waiting together go one at a time, in no
promised order. Paging a log or polling
then leaves a slow device room for its other clients; requests to
different destinations don't wait on each other. Behind a router the
pause after a reply holds for requests made one after another only
(#1535).

### BACnetClient.__aenter__

```python
def __aenter__(self) -> Awaitable[BACnetClient]
```

### BACnetClient.__aexit__

```python
def __aexit__(
    self,
    _exc_type: Any = None,
    _exc_val: Any = None,
    _exc_tb: Any = None,
) -> Awaitable[None]
```

### BACnetClient.read_property

```python
def read_property(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[PropertyValue]
```

Read a single property from a remote device.

ACK object/property/index must match. Device/Network Port instance 4194303
requests accept a same-type concrete ACK. Malformed/mismatched ACKs raise
BacnetError; this method returns the property value, not ACK metadata,
shaped as `PropertyValue` describes for read results.

### BACnetClient.write_property

```python
def write_property(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Write a single property on a remote device.

Priority is None or 1..16. Invalid u8 priorities raise ValueError
synchronously; integers outside u8 raise OverflowError.

### BACnetClient.read_property_multiple

```python
def read_property_multiple(
    self,
    address: str,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]],
) -> Awaitable[list[ReadAccessResult]]
```

Read multiple properties from a remote device (ReadPropertyMultiple).

`specs` is a list of `(object_id, [(property_id, array_index), ...])` tuples.
Returns ordered object/result dictionaries. Empty request/property lists
are rejected; this standalone profile still permits property selectors.

### BACnetClient.write_property_multiple

```python
def write_property_multiple(
    self,
    address: str,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]]],
) -> Awaitable[None]
```

Write multiple properties on a remote device (WritePropertyMultiple).

`specs` is `[(object_id, [(property_id, value, priority, array_index), ...]), ...]`.
Empty request/property lists, ALL/REQUIRED/OPTIONAL targets and u8
priorities outside 1..16 raise ValueError synchronously for the whole
request. Priorities outside u8 raise OverflowError. None, index zero,
proprietary properties, NULL and empty list values remain allowed.
A device error raises BacnetProtocolError with
first\_failed\_write\_attempt set from its WritePropertyMultiple-Error.

### BACnetClient.read_property_from_devices

```python
def read_property_from_devices(
    self,
    requests: list[tuple[int, ObjectIdentifier, PropertyIdentifier, Optional[int]]],
    max_concurrent: Optional[int] = None,
) -> Awaitable[list[_DeviceReadBatchResult]]
```

Read a property from multiple discovered devices concurrently.

`requests` is `[(device_instance, object_id, property_id, array_index), ...]`.
`max_concurrent` must be positive and fit the native usize; None uses 32.
Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
Returns an asyncio Future, accepted by await or asyncio.ensure\_future,
not a coroutine for asyncio.create\_task. Results are returned in completion
order; request\_index is the zero-based
input occurrence, including identical duplicates. Errors are BacnetError
instances with their protocol attributes. Python construction failures
raise from the whole call; cancellation returns no partial list.
Returns `[{"request_index": int, "device_instance": int, "value": PropertyValue | None, "error": BacnetError | None}, ...]`.

### BACnetClient.read_property_multiple_from_devices

```python
def read_property_multiple_from_devices(
    self,
    requests: list[tuple[int, list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]]],
    max_concurrent: Optional[int] = None,
) -> Awaitable[list[_DeviceRpmBatchResult]]
```

Read multiple properties from multiple devices concurrently (RPM batch).

`max_concurrent` must be positive and fit the native usize; None uses 32.
Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
Returns an asyncio Future, accepted by await or asyncio.ensure\_future,
not a coroutine for asyncio.create\_task. Results are returned in completion
order; request\_index is the zero-based
input occurrence, including identical duplicates. Errors are BacnetError
instances with their protocol attributes. Python construction failures
raise from the whole call; cancellation returns no partial list.
Returns `[{"request_index": int, "device_instance": int, "results": list[ReadAccessResult] | None, "error": BacnetError | None}, ...]`.

### BACnetClient.write_property_to_devices

```python
def write_property_to_devices(
    self,
    requests: list[tuple[int, ObjectIdentifier, PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]],
    max_concurrent: Optional[int] = None,
) -> Awaitable[list[_DeviceWriteBatchResult]]
```

Write a property to multiple devices concurrently.

Every priority must be None or 1..16; invalid u8 priorities raise
ValueError synchronously before any batch dispatch. Integers outside
u8 raise OverflowError.

`requests` is `[(device_instance, object_id, property_id, value, priority, array_index), ...]`.
`max_concurrent` must be positive and fit the native usize; None uses 32.
Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
Returns an asyncio Future, accepted by await or asyncio.ensure\_future,
not a coroutine for asyncio.create\_task. Results are returned in completion
order; request\_index is the zero-based
input occurrence, including identical duplicates. Errors are BacnetError
instances with their protocol attributes. Python construction failures
raise from the whole call; cancellation returns no partial list.
Returns `[{"request_index": int, "device_instance": int, "error": BacnetError | None}, ...]`.

### BACnetClient.who_is

```python
def who_is(
    self,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[None]
```

Broadcast a Who-Is request. Responses are collected asynchronously;
use `discovered_devices()` to retrieve them.

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

### BACnetClient.discover

```python
def discover(
    self,
    timeout_ms: int = 3000,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[list[DiscoveredDevice]]
```

Convenience: send WhoIs, wait `timeout_ms`, return discovered devices.

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

### BACnetClient.who_has_by_id

```python
def who_has_by_id(
    self,
    object_id: ObjectIdentifier,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[None]
```

Broadcast Who-Has by object identifier.

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

### BACnetClient.who_has_by_name

```python
def who_has_by_name(
    self,
    name: str,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[None]
```

Broadcast Who-Has by object name.

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

### BACnetClient.who_has_replies

```python
def who_has_replies(
    self,
    object: ObjectIdentifier | str,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
    timeout_ms: int = 3000,
) -> Awaitable[list[IHave]]
```

Broadcast a Who-Has for `object` (an identifier, or a name) and
collect the I-Haves that answer it for `timeout_ms`: one per device
and object, in arrival order; an empty list when none answer. The
limits follow `who_has_by_id()`'s rule (#1850). It holds the
started client for the whole wait, so `stop()` waits up to
`timeout_ms` for it.

### BACnetClient.i_have_events

```python
def i_have_events(self) -> Awaitable[IHaveIterator]
```

Follow every I-Have the client receives from now on, an answer to
its own Who-Has or not (#1850). RuntimeError before start and after
stop.

### BACnetClient.discover_routers

```python
def discover_routers(
    self,
    network: Optional[int] = None,
    timeout_ms: int = 3000,
) -> Awaitable[list[DiscoveredRouter]]
```

Broadcast a Who-Is-Router-To-Network for `network` (or every
network) and collect the routers that answer for `timeout_ms`, one
per router MAC in the order first heard; for a `network`, only the
routers listing it. An empty list when none answer. `network` 0 or
65535 raises BacnetError (#1850). It holds the started client for
the whole wait, so `stop()` waits up to `timeout_ms` for it.

### BACnetClient.network_reports

```python
def network_reports(self) -> Awaitable[NetworkReportIterator]
```

Follow the I-Am-Router-To-Network, Network-Number-Is and
Reject-Message-To-Network messages the client receives from now on
(#1850). RuntimeError before start and after stop.

### BACnetClient.discovered_devices

```python
def discovered_devices(self) -> Awaitable[list[DiscoveredDevice]]
```

Return all devices discovered so far.

### BACnetClient.get_device

```python
def get_device(self, instance: int) -> Awaitable[Optional[DiscoveredDevice]]
```

Look up a discovered device by instance number.

### BACnetClient.clear_devices

```python
def clear_devices(self) -> Awaitable[None]
```

Clear the discovered device table.

### BACnetClient.resolve_vmac

```python
def resolve_vmac(self, address: str) -> Awaitable[str]
```

Find the VMAC of the B/IPv6 node at `address`, given as
`"[ip]:port"`, and return it as `"bip6:0x0004D2"`, the form the
other methods take.

A B/IPv6 node is addressed by its VMAC, not its IPv6 address (Clause
U.1.3). This asks the node with a Virtual-Address-Resolution sent by
unicast, up to three times a second apart (Clause U.5), unless the
client already knows which VMAC is at that address. A link-local
address may carry its interface as a numeric zone
(`"[fe80::1%3]:47808"`).

Needs a started client with `transport="ipv6"`, else RuntimeError.
Any other address form raises ValueError. Raises BacnetTransportError
(errno ETIMEDOUT) when nothing answers within three seconds, or when
the node refuses.

### BACnetClient.who_am_i

```python
def who_am_i(
    self,
    vendor_id: int,
    model_name: str,
    serial_number: str,
) -> Awaitable[None]
```

Broadcast a Who-Am-I request announcing this device's identity.

The three arguments should match the sender's Device object properties
(`vendor_id` is 0-65535). Raises `ValueError` if a string cannot be
encoded, or `OverflowError` for a `vendor_id` that doesn't fit.

### BACnetClient.who_is_directed

```python
def who_is_directed(
    self,
    address: str,
    low_limit: Optional[int] = None,
    high_limit: Optional[int] = None,
) -> Awaitable[None]
```

Send a Who-Is to a specific device address (unicast).

Give `low_limit` and `high_limit` together or not at all, each from
0 to 4194303: one alone, `low_limit` above `high_limit`, or a limit
past 4194303 raises `ValueError` before anything is sent.

### BACnetClient.time_synchronization

```python
def time_synchronization(
    self,
    address: str,
    date: tuple[int, int, int, int],
    time: tuple[int, int, int, int],
) -> Awaitable[None]
```

Send a TimeSynchronization request (unconfirmed).

`date` is `(year, month, day, day_of_week)`; `time` is
`(hour, minute, second, hundredths)`. The request sets a clock, so
both must be specific: a real day with the full year (1900..2154),
month 1..12, day 1..31 and `day_of_week` that day's own weekday
(1 = Monday), and every time field in range. A field that is
`UNSPECIFIED` (255) or a pattern value (an odd month, the last
day) raises ValueError before anything is sent.

### BACnetClient.utc_time_synchronization

```python
def utc_time_synchronization(
    self,
    address: str,
    date: tuple[int, int, int, int],
    time: tuple[int, int, int, int],
) -> Awaitable[None]
```

Send a UTCTimeSynchronization request (unconfirmed); arguments as
for `time_synchronization`.

### BACnetClient.read_property_from_device

```python
def read_property_from_device(
    self,
    device_instance: int,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[PropertyValue]
```

Read a property from a device by instance number (auto-routing).

### BACnetClient.read_property_multiple_from_device

```python
def read_property_multiple_from_device(
    self,
    device_instance: int,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]],
) -> Awaitable[Any]
```

Read multiple properties from a device by instance number (auto-routing).

### BACnetClient.write_property_to_device

```python
def write_property_to_device(
    self,
    device_instance: int,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Write a property on a device by instance number (auto-routing).

Priority is None or 1..16. Invalid u8 priorities raise ValueError
synchronously before lookup; integers outside u8 raise OverflowError.

### BACnetClient.write_property_multiple_to_device

```python
def write_property_multiple_to_device(
    self,
    device_instance: int,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]]],
) -> Awaitable[None]
```

Write multiple properties to a device by instance number (auto-routing).

Same whole-request validation as write\_property\_multiple, synchronously
before discovery or dispatch: nonempty request/property lists, no special
selectors, priority None or 1..16. Invalid u8 priorities/structure raise
ValueError; priorities outside u8 raise OverflowError.

### BACnetClient.add_device

```python
def add_device(self, device_instance: int, address: str) -> Awaitable[None]
```

Add a device to the discovery table manually (useful when address is known without WhoIs).

### BACnetClient.subscribe_cov

```python
def subscribe_cov(
    self,
    address: str,
    subscriber_process_identifier: int,
    monitored_object_identifier: ObjectIdentifier,
    confirmed: bool,
    lifetime: Optional[int] = None,
) -> Awaitable[None]
```

Subscribe to Change-of-Value notifications for an object.

lifetime=None or 0 is indefinite; positive values are seconds.
Use unsubscribe\_cov for cancellation. confirmed always supplies the mode.

### BACnetClient.unsubscribe_cov

```python
def unsubscribe_cov(
    self,
    address: str,
    subscriber_process_identifier: int,
    monitored_object_identifier: ObjectIdentifier,
) -> Awaitable[None]
```

Cancel a COV subscription.

### BACnetClient.subscribe_cov_property_multiple

```python
def subscribe_cov_property_multiple(
    self,
    address: str,
    subscriber_process_identifier: int,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int], Optional[float], bool]]]],
    issue_confirmed_notifications: bool,
    max_notification_delay: Optional[int] = None,
    lifetime: Optional[int] = None,
) -> Awaitable[None]
```

Subscribe to COV notifications for multiple properties on multiple objects.

`specs` is `[(object_id, [(property_id, array_index, cov_increment, timestamped), ...]), ...]`.
`issue_confirmed_notifications` is required, including for cancellation requests.
For subscriptions and re-subscriptions, `lifetime` and `max_notification_delay` are both required.
For whole-context cancellations, pass an empty `specs` list and omit both fields.
Complete timing and nested-reference validation raises `ValueError` synchronously,
before an awaitable, address parsing, client-state access or I/O. Each supplied
object needs references; ALL/REQUIRED/OPTIONAL selectors are prohibited and the
cumulative reference limit is 10,000. Empty outer lists remain encodable with
valid finite timing, without claiming bundled-server empty-context support.
A device error about one COV reference raises BacnetProtocolError with
first\_failed\_subscription set; a general failure leaves it None.

### BACnetClient.cov_notifications

```python
def cov_notifications(self) -> Awaitable[CovNotificationIterator]
```

Get an async iterator for incoming COV notifications.

### BACnetClient.delete_object

```python
def delete_object(self, address: str, object_id: ObjectIdentifier) -> Awaitable[None]
```

Delete an object on a remote device (DeleteObject service).

### BACnetClient.create_object

```python
def create_object(
    self,
    address: str,
    object_specifier: Union[ObjectType, ObjectIdentifier],
    initial_values: Optional[list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]] = None,
) -> Awaitable[ObjectIdentifier]
```

Create an object on a remote device (CreateObject service).

`object_specifier` is an `ObjectType` (server assigns instance) or
`ObjectIdentifier` (specific instance). `initial_values` is
`[(property_id, value, priority, array_index), ...]`.
Returns the created object's identifier from the CreateObject-ACK; an
acknowledgment that doesn't decode raises BacnetError (the device may
still have created the object). A device error raises
BacnetProtocolError with first\_failed\_element\_number set when the
device answers with a CreateObject-Error.

### BACnetClient.device_communication_control

```python
def device_communication_control(
    self,
    address: str,
    enable_disable: EnableDisable,
    time_duration: Optional[int] = None,
    password: Optional[str] = None,
) -> Awaitable[None]
```

Send DeviceCommunicationControl to a remote device.

Raises ValueError, before anything is sent, for a password over 20
characters or an empty one (pass None to send no password).

### BACnetClient.reinitialize_device

```python
def reinitialize_device(
    self,
    address: str,
    reinitialized_state: ReinitializedState,
    password: Optional[str] = None,
) -> Awaitable[None]
```

Send ReinitializeDevice to a remote device.

Raises ValueError, before anything is sent, for a password over 20
characters or an empty one (pass None to send no password).

### BACnetClient.acknowledge_alarm_request

```python
def acknowledge_alarm_request(
    self,
    address: str,
    acknowledging_process_identifier: int,
    event_object_identifier: ObjectIdentifier,
    event_state_acknowledged: EventState,
    timestamp: BACnetTimeStamp,
    acknowledgment_source: str,
    time_of_acknowledgment: BACnetTimeStamp,
) -> Awaitable[None]
```

Acknowledge an alarm with exact caller-supplied BACnetTimeStamp values.

`timestamp` must echo the original event notification timestamp;
`time_of_acknowledgment` is selected by the caller.

### BACnetClient.acknowledge_alarm

```python
def acknowledge_alarm(
    self,
    address: str,
    acknowledging_process_identifier: int,
    event_object_identifier: ObjectIdentifier,
    event_state_acknowledged: EventState,
    acknowledgment_source: str,
) -> Awaitable[None]
```

Deprecated compatibility method.

This method fabricates sequence-number zero for both timestamps. Use
`acknowledge_alarm_request` for a lossless request.

### BACnetClient.get_event_information

```python
def get_event_information(
    self,
    address: str,
    last_received_object_identifier: Optional[ObjectIdentifier] = None,
) -> Awaitable[dict[str, Any]]
```

Get event information from a remote device: one GetEventInformation answer, decoded.

Returns `{"summaries": [...], "more_events": bool}`. Each summary is
`{"object_id": ObjectIdentifier, "event_state": EventState, "acknowledged_transitions": BitString, "event_timestamps": [BACnetTimeStamp, BACnetTimeStamp, BACnetTimeStamp], "notify_type": NotifyType, "event_enable": BitString, "event_priorities": [int, int, int]}`: the bit strings name `TO_OFFNORMAL`, `TO_FAULT` and
`TO_NORMAL`, and the timestamps and priorities are in that order.
While `more_events` is true, call again with the last summary's
`object_id` as `last_received_object_identifier`.

### BACnetClient.get_alarm_summary

```python
def get_alarm_summary(self, address: str) -> Awaitable[list[dict[str, Any]]]
```

Get alarm summary from a remote device.

Returns `[{"object_id": ObjectIdentifier, "alarm_state": EventState, "acknowledged_transitions": BitString}, ...]`, the bit string naming
`TO_OFFNORMAL`, `TO_FAULT` and `TO_NORMAL`.

### BACnetClient.get_enrollment_summary

```python
def get_enrollment_summary(
    self,
    address: str,
    acknowledgment_filter: AcknowledgmentFilter = ...,
    event_state_filter: Optional[EnrollmentSummaryEventStateFilter] = None,
    event_type_filter: Optional[EventType] = None,
    min_priority: Optional[int] = None,
    max_priority: Optional[int] = None,
    notification_class_filter: Optional[int] = None,
) -> Awaitable[list[dict[str, Any]]]
```

Get enrollment summary from a remote device.

Returns `[{"object_id": ObjectIdentifier, "event_type": EventType, "event_state": EventState, "priority": int, "notification_class": Optional[int]}, ...]`.

### BACnetClient.read_range

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

### BACnetClient.read_log_page

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

### BACnetClient.atomic_read_file

```python
def atomic_read_file(
    self,
    address: str,
    file_identifier: ObjectIdentifier,
    access_method: str,
    start_position: int = 0,
    requested_octet_count: int = 0,
    start_record: int = 0,
    requested_record_count: int = 0,
) -> Awaitable[AtomicReadFileResult]
```

Read from a file object. `access_method` is `"stream"` or `"record"`.
Returns the decoded acknowledgment: `access_method`, `end_of_file`,
then `start_position` and `data` for stream access, or
`start_record`, `record_count` and `records` for record access.
An acknowledgment in the other access method, or with more octets or
records than requested, raises BacnetError.

### BACnetClient.atomic_write_file

```python
def atomic_write_file(
    self,
    address: str,
    file_identifier: ObjectIdentifier,
    access_method: str,
    start_position: int = 0,
    file_data: bytes = b"",
    start_record: int = 0,
    record_count: int = 0,
    file_record_data: Optional[list[bytes]] = None,
) -> Awaitable[AtomicWriteFileResult]
```

Write to a file object. `access_method` is `"stream"` or `"record"`.
Returns the decoded acknowledgment: `access_method`, then
`start_position` for stream access or `start_record` for record
access, where the device says the write took place. An
acknowledgment in the other access method raises BacnetError.

### BACnetClient.add_list_element

```python
def add_list_element(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    list_of_elements: bytes,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Add elements to a list property (AddListElement service).
Requires nonempty, completely framed element bytes and a nonzero optional
index. Invalid framing/index zero raises ValueError synchronously before
client access; indexes outside u32 raise OverflowError. Empty-valued,
constructed/context/vendor elements are retained without remote datatype
validation. Shared limits: 1 MiB per tag, 32 context levels including the
service wrapper; application Boolean has no payload bytes. A device
error raises BacnetProtocolError with first\_failed\_element\_number set
when the device answers with a ChangeList-Error.

### BACnetClient.remove_list_element

```python
def remove_list_element(
    self,
    address: str,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    list_of_elements: bytes,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Remove elements from a list property (RemoveListElement service).
Requires nonempty, completely framed element bytes and a nonzero optional
index. Invalid framing/index zero raises ValueError synchronously before
client access; indexes outside u32 raise OverflowError. Empty-valued,
constructed/context/vendor elements are retained without remote datatype
validation. Shared limits: 1 MiB per tag, 32 context levels including the
service wrapper; application Boolean has no payload bytes. A device
error raises BacnetProtocolError with first\_failed\_element\_number set
when the device answers with a ChangeList-Error.

### BACnetClient.confirmed_private_transfer

```python
def confirmed_private_transfer(
    self,
    address: str,
    vendor_id: int,
    service_number: int,
    service_parameters: Optional[bytes] = None,
) -> Awaitable[dict[str, Any]]
```

Send a ConfirmedPrivateTransfer request.

Returns `{"vendor_id": int, "service_number": int, "result_block": bytes | None}`.
A device error raises BacnetProtocolError with vendor\_id,
service\_number and error\_parameters set from its
ConfirmedPrivateTransfer-Error. An ACK that is malformed, cut short
or has octets after its last member raises BacnetError.

### BACnetClient.unconfirmed_private_transfer

```python
def unconfirmed_private_transfer(
    self,
    address: str,
    vendor_id: int,
    service_number: int,
    service_parameters: Optional[bytes] = None,
) -> Awaitable[None]
```

Send an UnconfirmedPrivateTransfer request.

### BACnetClient.confirmed_text_message

```python
def confirmed_text_message(
    self,
    address: str,
    source_device: ObjectIdentifier,
    message_priority: MessagePriority,
    message: str,
    message_class_type: Optional[str] = None,
    message_class_value: Optional[Any] = None,
) -> Awaitable[None]
```

Send a ConfirmedTextMessage.

`message_class_type` is `"numeric"` or `"text"` (or `None` for no class).

### BACnetClient.unconfirmed_text_message

```python
def unconfirmed_text_message(
    self,
    address: str,
    source_device: ObjectIdentifier,
    message_priority: MessagePriority,
    message: str,
    message_class_type: Optional[str] = None,
    message_class_value: Optional[Any] = None,
) -> Awaitable[None]
```

Send an UnconfirmedTextMessage.

### BACnetClient.life_safety_operation

```python
def life_safety_operation(
    self,
    address: str,
    requesting_process_identifier: int,
    requesting_source: str,
    operation: LifeSafetyOperation,
    object_identifier: Optional[ObjectIdentifier] = None,
) -> Awaitable[None]
```

Send a LifeSafetyOperation request.

### BACnetClient.write_group

```python
def write_group(
    self,
    address: Optional[str],
    group_number: int,
    write_priority: int,
    change_list: list[tuple[int, Optional[int], Union[PropertyValue, bytes, bytearray]]],
    inhibit_delay: Optional[bool] = None,
    *,
    network: Optional[int] = None,
) -> Awaitable[None]
```

Send a WriteGroup request (unconfirmed).

With an `address` the request goes to that one device. With
`address=None` it is broadcast: on the local network, or with
`network` on that remote network (1-65534), or on every network when
`network` is 65535. Give `network` only with `address=None`.

`group_number` is 1-4294967295 (group 0 is reserved) and
`write_priority` is 1-16. `change_list` is a non-empty list of
`(channel, override_priority_or_none, value)` tuples, where
`channel` is a channel number 0-65535, the override priority is 1-16 or
`None`, and `value` is a `PropertyValue`, which is encoded for you,
or a `bytes` or `bytearray` holding one encoded BACnetChannelValue.
Either way the value must be one BACnetChannelValue: a single
application-tagged primitive (`PropertyValue.real(72.0)`), or a
context-0 lighting command, context-1 xy colour or context-2 colour
command (as `bytes`, or the same octets in
`PropertyValue.application_data`), with no wrapper tag. Raises
`ValueError` for anything else, a list for one, `TypeError` for a
value of another type (a list of ints included), and `OverflowError`
for integers that don't fit.

### BACnetClient.vt_open

```python
def vt_open(
    self,
    address: str,
    vt_class: VTClass,
    local_vt_session_identifier: int,
) -> Awaitable[int]
```

Open a virtual terminal session. Returns the remote session identifier.

`local_vt_session_identifier` (0-255) is the caller's own number for
the session, which the peer quotes when it sends data back.

### BACnetClient.vt_close

```python
def vt_close(self, address: str, session_ids: list[int]) -> Awaitable[None]
```

Close one or more virtual terminal sessions.

`session_ids` must not be empty and each identifier is 0-255. Raises
`ValueError` for an empty list, or `OverflowError` for an integer that
doesn't fit, before anything is sent. A device error that lists the
sessions it could not close raises BacnetProtocolError with
vt\_session\_identifiers set.

### BACnetClient.vt_data

```python
def vt_data(
    self,
    address: str,
    session_id: int,
    data: bytes,
    data_flag: bool,
) -> Awaitable[dict[str, Any]]
```

Send data over a virtual terminal session.

`data_flag` is the sequence flag that alternates on each new request
for a session; it is sent as an Unsigned 0 or 1.

Returns `{"all_new_data_accepted": bool, "accepted_octet_count": int | None}`;
the count is an `int` only when `all_new_data_accepted` is `False`.

### BACnetClient.confirmed_audit_notification_typed

```python
def confirmed_audit_notification_typed(
    self,
    address: str,
    request: AuditNotificationRequestInput,
) -> Awaitable[None]
```

Send a validated mapping through the native confirmed Audit helper.

### BACnetClient.unconfirmed_audit_notification_typed

```python
def unconfirmed_audit_notification_typed(
    self,
    address: str,
    request: AuditNotificationRequestInput,
) -> Awaitable[None]
```

Send a validated mapping through the native unconfirmed Audit helper.

### BACnetClient.audit_log_query_typed

```python
def audit_log_query_typed(
    self,
    address: str,
    request: AuditLogQueryRequestInput,
) -> Awaitable[AuditLogQueryAck]
```

Send a typed Audit Log query and return a canonical decoded mapping.

### BACnetClient.confirmed_audit_notification

```python
def confirmed_audit_notification(
    self,
    address: str,
    service_data: bytes,
) -> Awaitable[None]
```

Send a pre-encoded Clause 21 ConfirmedAuditNotification payload.

This is a raw escape hatch; the bundled server does not execute the
service.

### BACnetClient.unconfirmed_audit_notification

```python
def unconfirmed_audit_notification(
    self,
    address: str,
    service_data: bytes,
) -> Awaitable[None]
```

Send a pre-encoded Clause 21 UnconfirmedAuditNotification payload.

This is a raw escape hatch; the bundled server does not execute the
service.

### BACnetClient.audit_log_query

```python
def audit_log_query(self, address: str, service_data: bytes) -> Awaitable[bytes]
```

Send a pre-encoded Clause 21 AuditLogQuery payload.

This raw escape hatch returns the peer's response payload. The bundled
server does not execute the service.

### BACnetClient.sc_npdu_drop_counts

```python
def sc_npdu_drop_counts(self) -> Awaitable[ScNpduDropCounts]
```

Copy SC ingress counters; requires a started SC owner.

Available while disconnected. Each call returns an independent dictionary;
a new transport starts fresh counters. No network I/O is performed.

### BACnetClient.sc_peer_info

```python
def sc_peer_info(self) -> Awaitable[ScPeerInfo | None]
```

Observe the accepted active SC Hub without I/O; None while disconnected.
RuntimeError for non-SC, stopped or closing clients. Historical results
own data only. Observation time is not certificate validation time.

### BACnetClient.read_bdt

```python
def read_bdt(self, address: str) -> Awaitable[list[tuple[str, str]]]
```

Read a BBMD's Broadcast Distribution Table as `("ip:port", "mask")` rows.

### BACnetClient.write_bdt

```python
def write_bdt(
    self,
    address: str,
    entries: Sequence[str | tuple[str, str]],
) -> Awaitable[None]
```

Ask a BBMD to take `entries` (rows as `Bbmd(bdt=)` takes them)
as its BDT. A BBMD that follows the 2020 standard refuses with 0x0010.

### BACnetClient.read_fdt

```python
def read_fdt(self, address: str) -> Awaitable[list[FdtEntry]]
```

Read a BBMD's Foreign Device Table.

### BACnetClient.delete_fdt_entry

```python
def delete_fdt_entry(self, address: str, entry: str) -> Awaitable[None]
```

Ask a BBMD to delete the foreign device at `entry`
(`"ip:port"`) from its FDT; a refusal is 0x0050.

### BACnetClient.register_foreign_device_bvlc

```python
def register_foreign_device_bvlc(self, address: str, ttl: int) -> Awaitable[None]
```

Register once as a foreign device for `ttl` seconds (0 to
65535); a refusal is 0x0030. Nothing is renewed and broadcasts don't
go through the BBMD: `foreign_device=` does that.

### BACnetClient.connection_events

```python
def connection_events(self) -> Awaitable[ScEventIterator]
```

Subscribe to the SC hub-connector events (#1829). The first call
after a start gets the events since the client was built, starting
with the first `connected` (#1908); if that first call comes long
after the start, its first events may be old ones from the build. A
later call is a late subscriber and gets the events from that call
on. A slow reader never holds up the connector: see
`ScEventIterator.missed`.
RuntimeError for a stopped or non-SC client.

### BACnetClient.connection_status

```python
def connection_status(self) -> Awaitable[ScLinkStatus]
```

Snapshot the SC hub link without I/O (#1829). RuntimeError for a
stopped or non-SC client.

### BACnetClient.foreign_registration

```python
def foreign_registration(self) -> Awaitable[ForeignRegistrationStatus]
```

How the foreign-device registration is going (#938), a refusal
included. Sends nothing. Outside `foreign_device=` only manual
registrations show. RuntimeError before start and after stop; a
non-B/IP client raises BacnetError.

### BACnetClient.stop

```python
def stop(self) -> Awaitable[None]
```

Refuse new work, drain operations, and join native teardown.

Cancelling this Future leaves shutdown running; call stop() again to
join it. Drain duration depends on existing requests. Errors retain
closing state for another stop attempt.

## ScReconnect

Frozen SC client retry settings, copied when passed to BACnetClient.

Delays must be nonzero, initial \<= maximum, and maximum \<= 86,400,000 ms.
Invalid settings raise ValueError; unsigned integer overflow raises
OverflowError. Zero retries allows the initial connection and disables
active-hub retries after loss. Validation is not deployment qualification.

Properties:

- `initial_delay_ms: int`
- `max_delay_ms: int`
- `max_retries: int`

### ScReconnect.__init__

```python
def __init__(
    self,
    *,
    initial_delay_ms: int = 10000,
    max_delay_ms: int = 600000,
    max_retries: int = 10,
) -> None
```

## ScCertificateMetadata

Frozen informational metadata; ASCII-escaped fields are at most 1024 bytes.

Properties:

- `subject: str`
- `issuer: str`
- `not_before_unix: int`
- `not_after_unix: int`
- `public_key_algorithm_oid: str`
- `signature_algorithm_oid: str`
- `text_truncated: bool`
- `subject_alt_names: tuple[ScSubjectAltName, ...]`: The DNS, URI, email and IP Subject Alternative Names, in certificate order (#2022); other name forms are left out.

## ScSubjectAltName

One Subject Alternative Name exactly as the certificate wrote it:
no case folding, wildcards such as `*.example.com` as written. Compare
`value` for equality, never by suffix. Frozen; its repr shows the
kind only.

Properties:

- `kind: Literal["dns", "uri", "email", "ip"]`
- `value: str | ipaddress.IPv4Address | ipaddress.IPv6Address`: A `str`, or an `ipaddress` address for an `ip` name.

## ScInspectionFinding

Frozen local interpretation finding; never a TLS acceptance decision.

Properties:

- `area: Literal["certificate", "san", "key_usage", "extended_key_usage"]`
- `kind: Literal["malformed", "duplicate", "unsupported", "truncated"]`

## ScExtensionInspection

Frozen bounded display. Read certificate findings alongside these values.

Properties:

- `assessed: bool`
- `present: bool`
- `values: tuple[str, ...]`

## ScInspectedCertificate

Frozen exact DER fingerprint and optional interpretation, without retained DER.

Properties:

- `role: Literal["identity", "peer_trust"]`
- `index: int`
- `sha256: bytes`
- `metadata: ScCertificateMetadata | None`
- `validity: Literal["unavailable", "invalid_interval", "not_yet_valid", "expired", "within_interval"]`
- `expiry: Literal["not_assessed", "none", "warning", "alarm"]`
- `san: ScExtensionInspection`
- `key_usage: ScExtensionInspection`
- `extended_key_usage: ScExtensionInspection`
- `findings: tuple[ScInspectionFinding, ...]`

## ScKeyFilePermissions

Frozen Unix read-mode observations from the opened key file; no ACL audit.

Properties:

- `group_readable: bool`
- `world_readable: bool`

## ScPemInspection

Frozen owned offline data. No credential bytes, client or TLS policy is retained.

Properties:

- `inspected_at_unix: int`
- `peer_trust_supplied: bool`
- `certificates: tuple[ScInspectedCertificate, ...]`
- `key_consistency: Literal["not_assessed", "match", "mismatch", "unusable", "unknown"]`
- `key_file_permissions: ScKeyFilePermissions | None`
- `direct_issuer: Literal["not_assessed", "direct", "not_direct", "unknown"]`: Whether a `peer_trust` certificate signed the identity leaf directly (its subject is the leaf's issuer and the leaf's signature verifies with its key). `not_assessed` without peer trust; `unknown` when the leaf can't be parsed or a candidate's signature algorithm is unsupported. Informational: it changes no acceptance.
- `direct_issuer_index: int | None`: Zero-based `peer_trust` index of the CA that signed the leaf when `direct_issuer` is `direct`; else None.

## ScPeerCertificate

Frozen original DER and optional display interpretation.

Properties:

- `der: bytes`
- `metadata: ScCertificateMetadata | None`
- `inspection_error: Literal["certificate_parse_failed"] | None`

## ScUntrustedPeerInfo

Frozen, owned untrusted certificates from one failed outbound TLS attempt.

No accepted connection identity or verification result is implied. Missing
evidence does not prove that the peer sent no certificates. Metadata is
informational; retaining this value owns no network resources.

Properties:

- `peer_address: str`
- `server_name: str`
- `certificates: tuple[ScPeerCertificate, ...]`

## ScPeerInfo

Frozen historical observation; does not retain a client or socket.

Properties:

- `connection_id: int`
- `observed_at_unix: float`
- `peer_address: str`
- `server_name: str`
- `handshake_kind: Literal["full", "resumed", "unknown"]`
- `protocol_version: str | None`
- `cipher_suite: str | None`
- `leaf_sha256: bytes`
- `certificates: tuple[ScPeerCertificate, ...]`

## ScEvent

One SC hub-connector event (#1829), frozen. Fields that don't apply
to the event's kind are None.

A failover reads `disconnected`, then `reconnect_attempt` and
`connect_failed` for each failed primary attempt, then
`failover_engaged` and `connected` on the failover hub. A primary
restore reads `primary_restored`, then `connected` on the primary;
each failed restore probe, at most one per restore interval, reads
`primary_restore_failed` (#1828).

Properties:

- `kind: Literal['connected', 'disconnected', 'connect_failed', 'reconnect_attempt', 'failover_engaged', 'primary_restored', 'primary_restore_failed', 'reconnect_gave_up', 'other']`
- `hub: Literal["primary", "failover"] | None`: The hub connected to, lost, failed or being dialled; `primary` for `primary_restore_failed`.
- `hub_vmac: bytes | None`: The hub's VMAC from its Connect-Accept, on `connected`.
- `cause: Literal['local_close', 'peer_close', 'heartbeat_timeout', 'websocket', 'tls', 'nak', 'dial_timeout', 'connect_accept_timeout', 'upgrade_refused', 'malformed_result', 'unexpected_connect_reply', 'rejection_expired', 'other'] | None`: Why a `disconnected` link ended, or a `connect_failed` attempt or `primary_restore_failed` probe failed. `dial_timeout`: the dial (host lookup, TCP, TLS, WebSocket upgrade) ran out of time; `connect_accept_timeout`: the hub never answered the Connect-Request; `upgrade_refused`: the server answered the upgrade with `http_status` (#1839).
- `dial_stage: Literal['host_lookup', 'tcp_connect', 'tls', 'websocket_upgrade', 'other'] | None`: Where a `dial_timeout` dial stalled; None when it did not say.
- `http_status: int | None`: The HTTP status of an `upgrade_refused`, such as a proxy's 502.
- `cause_detail: str | None`: Diagnostic text for a `websocket`, `tls` or `other` cause; not stable.
- `error_class: ErrorClass | None`: The NAK's error class, for a `nak` cause.
- `error_code: ErrorCode | None`: The NAK's error code, for a `nak` cause.
- `attempt: int | None`: A `reconnect_attempt`'s number, from 1; the attempts made, for `reconnect_gave_up`.
- `backoff: float | None`: Seconds a `reconnect_attempt` waited first.

## ScLinkStatus

A frozen snapshot of the SC hub link (#1829).

Properties:

- `state: Literal["disconnected", "connecting", "connected", "disconnecting"]`
- `active_hub: Literal["primary", "failover"] | None`: The hub in use or, while reconnecting, being dialled; None before the first attempt.
- `hub_vmac: bytes | None`: The connected hub's VMAC; None while not connected.
- `heartbeat_age: float | None`: Seconds since the last message from the hub; None while not connected.
- `heartbeat_rtt: float | None`: Round-trip seconds of the latest answered Heartbeat-Request on this connection; None until one is answered.

## ScEventIterator

Async iterator over `ScEvent` from one `connection_events()` call.

The connector never waits for it: an iterator more than 64 events behind
loses the oldest and adds them to `missed`. A cancel that lands while an
event is being delivered, as a timed-out `asyncio.wait_for` does, puts
the event back: the next `__anext__` returns it first. Concurrent
`__anext__` calls complete one at a time. Read it from one running event
loop (`asyncio.run` is fine): an item on its way to a read whose loop
stops for good holds up later reads, which raise one `RuntimeWarning`
after 5 s and keep waiting. Iteration ends when the client stops.

Properties:

- `missed: int`: Events lost so far: because this iterator fell behind the connector, or dropped on the way to a read, as when its event loop closed first.

### ScEventIterator.__aiter__

```python
def __aiter__(self) -> ScEventIterator
```

### ScEventIterator.__anext__

```python
def __anext__(self) -> Awaitable[ScEvent]
```

## _DeviceReadBatchResult

`class _DeviceReadBatchResult(TypedDict)`

Not importable: the stub defines it only to type what a method returns.

Keys:

- `request_index: int`
- `device_instance: int`
- `value: PropertyValue | None`
- `error: BacnetError | None`

## _DeviceRpmBatchResult

`class _DeviceRpmBatchResult(TypedDict)`

Not importable: the stub defines it only to type what a method returns.

Keys:

- `request_index: int`
- `device_instance: int`
- `results: list[ReadAccessResult] | None`
- `error: BacnetError | None`

## _DeviceWriteBatchResult

`class _DeviceWriteBatchResult(TypedDict)`

Not importable: the stub defines it only to type what a method returns.

Keys:

- `request_index: int`
- `device_instance: int`
- `error: BacnetError | None`
<!-- generated:end -->
