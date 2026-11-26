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
Async entry loads the explicit site CA and operational cert/key, each file
read up to 1 MiB; invalid TLS configuration, a larger file included, raises
RuntimeError before dialing. No system-root or
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

`await endpoint.client()` on a `BipEndpoint`, `ScEndpoint` or
`MstpEndpoint` gives a client of this class on the endpoint's link
(#2617), with every call, on the endpoint's one address. Its endpoint
starts it: `async with` only enters it, and once stopped it stays
stopped (ask the endpoint for another). Its `stop()` ends the client
role alone.

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

For B/IP, `interface` accepts an IPv4 literal or the exact OS interface
name (the adapter friendly name on Windows), resolved once, at
construction, to one usable IPv4 address; `broadcast_address` is not
derived from it. See "Selecting an interface" in docs/python-api.md.

Properties:

- `write_policy: WritePolicy`: The priorities `Device.write` and `relinquish` may use, as `write_policy=` set them.
- `resolve_scope: Literal["local", "global"] | int`: Where a `device()` handle made from a bare instance asks for it, as `resolve_scope=` set it.
- `watch_lifetime: int`: The COV lifetime of a `Device.watch` that names none, in seconds, as `watch_lifetime=` set it.
- `watch_poll_interval: float`: The poll interval of a `Device.watch` that names none, in seconds, as `watch_poll_interval=` set it.
- `watch_subscribe: bool`: Whether a `Device.watch` that doesn't say subscribes its points for COV (False: polls them), as `watch_subscribe=` set it.

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
    share_wildcard_port: bool = False,
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
    write_policy: Optional[WritePolicy] = None,
    resolve_scope: Literal["local", "global"] | int | None = None,
    accept_complex_ack_for_simple_services: bool = False,
    watch_lifetime: int = 300,
    watch_poll_interval: float = 10.0,
    watch_subscribe: bool = True,
) -> None
```

`watch_lifetime` (default 300), `watch_poll_interval` (default
10.0) and `watch_subscribe` (default True): what a
`Device.watch` takes unless it names its own `lifetime`,
`poll_interval` or `subscribe` (#2603). Watches with equal
settings share their subscriptions. A lifetime is 1 to 28800 s
(eight hours, the longest every device must take), a poll interval
1 to 3600 s, `watch_subscribe=False` polls every point; another
value raises ValueError, another type TypeError.

`write_policy` (default None, which is `WritePolicy()`): the
priorities `Device.write` and `relinquish` may use. The default
refuses 1, 2, 5 and 6 with `BacnetWriteDeniedError`, sending
nothing; `WritePolicy().allow_priorities([...])` allows some of
them too (#2561). `write_property` and the other typed calls
don't look at it. Anything but a `WritePolicy` raises TypeError.

`resolve_scope` (default None, which is `"local"`): where a
`device()` handle made from a bare instance (`1234`) sends its
one Who-Is when no address claims the instance (#2561): this
network alone, `"global"` for every network, or a network number
from 1 to 65534. A target that names its network (`"5:1234"`)
asks there whatever this says. On `transport="router"`, which is
on each of the router's networks, `"local"` asks every one of
them (#2601). Another name or number raises ValueError, another
type TypeError.

`accept_complex_ack_for_simple_services` (default False): a device
that acknowledges a request whose service takes a Simple-ACK, such
as `write_property`, with a Complex-ACK fails the request with
`BacnetError` (#2721), since whether it carried the request out is
unknown. Set it True only for a device found in the field that
answers so, whose Complex-ACK this client then takes as the success
it means. A Simple-ACK where an answer must carry data, such as a
`read_property`'s, stays an error either way.

`transport="router"` with `router` (a `BACnetRouter` built
with `application="client"` or `"client_and_server"`): run the
client on the router's own application entity (#2071), with the
router's address on each network as its own. Entry takes the client's
share from the running router, once per router start: before the
router starts, or when a client already took it, entry raises
RuntimeError. Requests go out through the router's table, so name
each device by its network: discover it, then use the
`_from_device` / `_to_device` methods. The local scope is each
of the router's networks (#2601). A send to an `ip:port` names no
one of them and raises BacnetError. Once the router closes,
each request fails at once with BacnetStateError (state
`"closed"`), with nothing sent; stop the client as usual. With
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

`share_wildcard_port` (B/IP and B/IPv6 only, default False): share
an explicitly requested port with other sockets on this host that
bind it the same way. By default the client claims its port, so a
second bind of it fails, and `start()` raises when another socket
holds it. With sharing, broadcasts reach every socket on the port,
but each unicast reaches only one of them; prefer
`share_port_by_address` (B/IP) or separate ports. macOS and the
BSDs never let two sockets share one address and port this way, so
there the keyword only lets a stack bound to `0.0.0.0` or `::`
start beside one bound to a specific address (a B/IPv6 foreign
device), or the reverse. Another transport raises ValueError.

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
    address: str | RoutedTarget,
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
`address` is a `str` on the local link, or a `RoutedTarget` for
a station behind a router (#2617), as `RoutedTarget` describes.

### BACnetClient.write_property

```python
def write_property(
    self,
    address: str | RoutedTarget,
    object_id: ObjectIdentifier,
    property_id: PropertyIdentifier,
    value: PropertyValue,
    priority: Optional[int] = None,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Write a single property on a remote device.

Priority is None or 1..16. Invalid u8 priorities raise ValueError
synchronously; integers outside u8 raise OverflowError. The client's
`write_policy` doesn't apply: this is the low-level write.
`address` is a `str` on the local link, or a `RoutedTarget` for
a station behind a router (#2617), as `RoutedTarget` describes.

### BACnetClient.device

```python
def device(
    self,
    target: str | int | Discovered | DiscoveredDevice | tuple[int, bytes],
) -> Awaitable[Device]
```

A handle on one device, whose methods are the high-level calls:
`read`, `read_many`, `read_object`, `write`,
`preview_write`, `relinquish`, `priority_array`, `objects`
and `summary` (#2561).

`target` names the device in the target grammar, not in the
`address` forms the typed calls take. Most addresses read the
same in both: `"ip:port"`, six hex octets, the `bip6:` forms and
`"mstp:N"`. Two don't. \*\*A bare `"5"` is device instance 5 here,
not MS/TP station 5: write `"mstp:5"`.\*\* And colon-separated hex
of another length than six octets, which `address` takes as a
MAC, is no address here: it is refused (ValueError), or, for two
decimal parts such as `"10:20"`, read as instance 20 on network
10. The forms:

- an address pins the handle there and sends nothing for a station
on this network: `"192.168.1.10"` or `"192.168.1.10:47808"`,
`"mstp:5"`, `"bip6:0x0004D2"`, an SC VMAC as six hex octets,
a routed station as `"2:0x05"` (network 2, MAC 05; a MAC that
is one station though it looks like a broadcast, ARCNET's
station 255 or a gateway's two-octet station, takes the suffix
`":one-station"`, as `"5:0xFF:one-station"`), or a
`(network, mac)` pair such as a lookup error's candidate. A
routed station with no learned router is asked directly
first;
- an instance, `1234` (an int or text) or `"2:1234"` on network
2, gives a handle that follows the instance: the one address
claiming it, or the one that answers a Who-Is for it, sent where
`resolve_scope` says for a bare instance (this network alone by
default) and to the network named otherwise, waiting up to 3
seconds;
- a `discover()` row or a `DiscoveredDevice` pins the handle to
its address, knowing its instance.

A wrong type raises TypeError and text the grammar refuses
ValueError, when the call is made. On await: an address the
client's link can't reach raises ValueError, as does a local
address on `transport="router"`; an instance no address claims
and none answered for, or a routed address with no router known,
`BacnetLookupError`; an instance two or more addresses claim
`BacnetAmbiguousDeviceError`, whose `candidates` are each a
target.

### BACnetClient.read_property_multiple

```python
def read_property_multiple(
    self,
    address: str | RoutedTarget,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]],
) -> Awaitable[list[ReadAccessResult]]
```

Read multiple properties from a remote device (ReadPropertyMultiple).

`specs` is a list of `(object_id, [(property_id, array_index), ...])` tuples.
Returns ordered object/result dictionaries. Empty request/property lists
are rejected; this standalone profile still permits property selectors.
`address` is a `str` on the local link, or a `RoutedTarget`
(#2617).

### BACnetClient.write_property_multiple

```python
def write_property_multiple(
    self,
    address: str | RoutedTarget,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]]],
) -> Awaitable[None]
```

Write multiple properties on a remote device (WritePropertyMultiple).

Pass a local address string or a routed station, with an explicit or learned next hop.
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
    *,
    low: Optional[int] = None,
    high: Optional[int] = None,
    scope: Literal["local", "global"] | int | str | RoutedTarget = "local",
) -> Awaitable[None]
```

Send a Who-Is request. Responses are collected asynchronously;
use `discovered_devices()` to retrieve them.

`scope` says where the broadcast goes (#2560): `"local"`, the
default, is this network alone, which no router passes on;
`"global"` is every network, which on a large routed site every
device answers; a number from 1 to 65534 is that remote network,
through its router; and any other text is one station's address in
the forms `device()` takes (`"192.168.1.10"`, `"mstp:5"`),
asked by unicast (#2561). A `RoutedTarget` selects one station by
its network and MAC. Its explicit or learned next hop receives the
request by unicast. When the next hop is unknown, the local link
carries the station-addressed request by broadcast. Another
number, or text that names a device instance or a routed target,
raises `ValueError`, and another type `TypeError`, before
anything is sent; a station the client's link can't reach raises
`ValueError` on await. A client on `transport="router"` is on
each of the router's networks, so `"local"` sends one Who-Is to
each, which the router delivers there and passes no further, and
the devices answer as routed devices on their networks (#2601); a
station's address names no one of those networks and raises
`ValueError`, where a `RoutedTarget` names its network.

Give `low` and `high` together or not at all, each from
0 to 4194303: one alone, `low` above `high`, or a limit
past 4194303 raises `ValueError` before anything is sent.

### BACnetClient.discover

```python
def discover(
    self,
    *,
    scope: Literal["local", "global"] | int | str | RoutedTarget = "local",
    low: Optional[int] = None,
    high: Optional[int] = None,
    wait: float = 3.0,
) -> Awaitable[list[Discovered]]
```

Find devices: one Who-Is, then the devices that answered it
within `wait` seconds, one `Discovered` an address (#2561).
Only this call's answers: a device the client heard earlier that
stays quiet now isn't listed (`discovered_devices()` is the whole
table). Each row is a `device()` target, and its `duplicate`
says when another address claims the same instance.

The Who-Is goes where `scope` says, as for `who_is()`: this
network alone unless it names every network (`"global"`), one
remote network by its number, or one station by its local address
or `RoutedTarget`. A directed result matches that station's address.

Give `low` and `high` together or not at all, each from 0 to
4194303, to ask only those instances: one alone, `low` above
`high`, or a limit past 4194303 raises `ValueError` before
anything is sent, as does a `wait` outside 0 to 3600 seconds.

### BACnetClient.who_has_by_id

```python
def who_has_by_id(
    self,
    object_id: ObjectIdentifier,
    *,
    low: Optional[int] = None,
    high: Optional[int] = None,
    scope: Literal["local", "global"] | int | str = "local",
) -> Awaitable[None]
```

Send a Who-Has by object identifier. Answers come through
`i_have_events()`.

`scope` says where it goes, as for `who_is()` (#2596):
`"local"`, the default, is this network alone, which no router
passes on; `"global"` is every network, where every device that
hears it searches its objects; a number from 1 to 65534 is that
remote network, through its router; any other text is one station's
address, asked by unicast (#2561). Another number, or text that
names a device instance or a routed target, raises `ValueError`,
and another type `TypeError`, before anything is sent. A client on
`transport="router"` is on each of the router's networks:
`"local"` asks every one of them (#2601), and a station's address,
which names no one of them, raises `ValueError`.

Give `low` and `high` together or not at all, each from
0 to 4194303: one alone, `low` above `high`, or a limit
past 4194303 raises `ValueError` before anything is sent.

### BACnetClient.who_has_by_name

```python
def who_has_by_name(
    self,
    name: str,
    *,
    low: Optional[int] = None,
    high: Optional[int] = None,
    scope: Literal["local", "global"] | int | str = "local",
) -> Awaitable[None]
```

Send a Who-Has by object name, compared exactly. The limits and
`scope` follow `who_has_by_id()`'s rules: this network alone
unless `scope` names every network (`"global"`) or one remote
network by its number.

### BACnetClient.who_has_replies

```python
def who_has_replies(
    self,
    object: ObjectIdentifier | str,
    *,
    low: Optional[int] = None,
    high: Optional[int] = None,
    wait: float = 3.0,
    scope: Literal["local", "global"] | int | str = "local",
) -> Awaitable[list[IHave]]
```

Send a Who-Has for `object` (an identifier, or a name) and
collect the I-Haves that answer it from that scope for
`wait` seconds: one per device, object and station (a station's copy
through a router on this client's own network is its local answer),
in arrival order; an empty list when none answer. An I-Have heard
from elsewhere during the wait is not an answer, and collection stops
at 4096 answers, so a list of exactly that many may have been cut
short. The limits and `scope`
follow `who_has_by_id()`'s rules (#1850, #2596): with the default
scope an empty list says only that nothing on this network has the
object. `wait` is 0 to 3600 seconds (`ValueError` otherwise). It
holds the started client for the whole wait, so `stop()` waits up
to `wait` seconds for it.

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
    *,
    router: str | bytes | None = None,
    wait: float = 3.0,
) -> Awaitable[list[DiscoveredRouter]]
```

Broadcast a Who-Is-Router-To-Network for `network` (or every
network) and collect the routers that answer for `wait` seconds, one
per router MAC in the order first heard; for a `network`, only the
routers listing it. An empty list when none answer.

With `router`, the router's address on the local link (`bytes`
such as a `DiscoveredRouter`'s `mac`, or address text as for
`routing_table`), the question goes to that router alone and only
its own answers count. A router asked about a network it does not
know may ask further before it answers. An IPv6 socket address can
need VMAC resolution before the wait. The router answers with a
local broadcast, so a B/IP router on another IP subnet is heard only
when a BBMD forwards its answer here, as a peer or by foreign-device
registration: with no BBMD, an empty list is the expected result. A
`router` that names no one
station (empty, a broadcast or group address) raises ValueError, as
`routing_table` does, and sends nothing.

`network` 0 or 65535 raises BacnetError (#1850). `wait` is 0 to
3600 seconds (`ValueError` otherwise). It holds the started client
through resolution, sending and collection. `stop()` waits for the
call to finish or be cancelled.

### BACnetClient.routing_table

```python
def routing_table(
    self,
    router: str | bytes | RoutedTarget,
    *,
    wait: float = 3.0,
    quiet: float = 0.5,
    total: float = 10.0,
    max_entries: int = 4096,
) -> Awaitable[RoutingTable]
```

Read the routing table of a local or routed station (#2612). The call sends
one Initialize-Routing-Table with a port count of zero, which only
asks; no argument can make it an update.

For a local router, pass its MAC as `bytes` or a string in the
binding's address forms (`"ip:port"`, `"bip6:N"`, hex octets, an
MS/TP station). For a remote router, pass a RoutedTarget. An empty,
broadcast or group link address raises ValueError.

A long table comes as several acknowledgements with nothing to mark
the last, so the call returns once `quiet` seconds pass after the
last acknowledgement from the router (other traffic does not restart
it), at `total` seconds from the send, or at `max_entries`
entries (at least 1); the last two set `truncated` on the result,
which covers those bounds only. A quiet end does not prove the table
whole: on MS/TP, acknowledgements after the first wait for the
router's token, so use a larger `quiet` on a slow trunk. A late
acknowledgement of an earlier query to the same router can be
counted by the next one (nothing pairs answers with questions), so
leave a gap between repeated queries; entries are reported as
received, not deduplicated.

An empty table, or an older router's acknowledgement with no data,
is a `RoutingTable` with no entries. A router that sends nothing
within `wait` seconds raises BacnetTimeoutError: one that ignores
the message looks the same as one that is absent (on B/IP, so does
one that answers from another address or port), and neither is an
empty table. A Reject-Message-To-Network from the router before any
acknowledgement raises BacnetNetworkRejectError; rejects with
reasons 1, 2 and 4 are ignored for local queries. Each bound is 0 to
3600 seconds and `wait` may not exceed `total` (ValueError
otherwise). A client stopped mid-call raises RuntimeError. A link
that can't send the query for its own state or kind, such as a
BACnet/SC link not connected to its hub, or a router-application
client's link, which carries APDUs only, raises what that send's
error raises (BacnetStateError with state `"not_connected"` for
the first, BacnetError for the second), not ValueError. A
RoutedTarget selects the remote router, using its explicit or cached
next hop. Without either, one router-discovery window of up to
`wait` seconds must identify a single local next hop for its network.
Zero or multiple next hops, or a discovery overrun, raise BacnetError.
Replies must carry the remote station's source network and address and
arrive through the selected hop. An older router that answers a table
query only on its own link never answers this client: the call raises
BacnetTimeoutError, whose text for a routed query says so. A router
that refuses table queries from other networks (a hardened hosted
router with no authorizer, or a host that refuses the query class)
drops a routed query silently, and the call raises BacnetTimeoutError
too.
A reject for the requested DNET that comes through the next hop
before the first ack ends a routed query, whether the hop sent it or
relayed it from a router further on (reason 4, message too long, is
ignored: the query is too short to draw it). A simultaneous request through that path can
produce the same reject because network messages have no invoke ID.
One query runs at a time per client; a second call waits for
the first. It holds the started client for the whole wait, so
`stop()` waits for it.

### BACnetClient.configure_routed_path_max_npdu

```python
def configure_routed_path_max_npdu(
    self,
    router: str | bytes,
    network: int,
    max_npdu: int,
) -> Awaitable[None]
```

Set the size limit, in NPDU octets, of the routed path through
`router` to `network`, replacing any limit set or learned before
(#2667). Requests to stations on that network through that router are
sized to it, and segmented or refused as any request too large for
its path is.

`router` is the router's address on the local link, as
`routing_table` takes it: `bytes` such as a `DiscoveredRouter`'s
`mac`, or a string in the binding's address forms (a B/IPv6 socket
address is first asked its VMAC). `network` is 1 to 65534. A limit
under 228 is taken, for an older router that carries less than the
standard asks; one that could carry no request at all (under 66 on
B/IP, 61 on MS/TP) raises ValueError, as do a bad router or network.
An `int` outside 0 to 65535 raises OverflowError, and a `bool`
for `network` or `max_npdu` TypeError.

The change applies at once (#2683): it waits neither for a request on
the path nor out the hold a request with no answer leaves (the APDU
timeout times the attempts). A request waiting out the hold still
waits it out, then goes sized to the new limit; one already sent
keeps its size. Nothing goes through the path. RuntimeError before
start and after stop.

### BACnetClient.clear_routed_path_limit

```python
def clear_routed_path_limit(self, router: str | bytes, network: int) -> Awaitable[None]
```

Put the routed path through `router` to `network` back on the
default, 228 octets, dropping the limit set or learned for it
(#2667). Takes `router` and `network` as
`configure_routed_path_max_npdu` does and applies at once too.
Nothing goes through the path. RuntimeError before start and after
stop.

### BACnetClient.routed_path_limits

```python
def routed_path_limits(self) -> Awaitable[list[RoutedPathLimit]]
```

Every routed path that holds more than the default, by router and
then network (#2667): a limit set by hand, one learned from a
router's too-long reject, or a dropped below-minimum reject
(`below_minimum_reject_seen`). A path held only by that mark may be
dropped when the client's table of 256 paths is full. Sends nothing.
RuntimeError before start and after stop.

### BACnetClient.too_long_reject_drops

```python
def too_long_reject_drops(self) -> Awaitable[int]
```

How many too-long rejects (Reject-Message-To-Network, reason 4) the
client dropped since start, on every path, because they could answer
a larger frame of an earlier request (#2651). These clear by
themselves. RuntimeError before start and after stop.

### BACnetClient.below_minimum_reject_drops

```python
def below_minimum_reject_drops(self) -> Awaitable[int]
```

How many too-long rejects the client dropped since start, on every
path, because the request they landed on sent nothing above 228 NPDU
octets (#2665). A rise can mean a router that carries less than that:
its requests end by timeout until `configure_routed_path_max_npdu`
sets the path's limit; `routed_path_limits()` marks the paths.
RuntimeError before start and after stop.

### BACnetClient.off_path_answer_drops

```python
def off_path_answer_drops(self) -> Awaitable[int]
```

How many answers for a device's open request the client dropped
since start because they came by another path than the request took
(#2831): from another station than a routed request's router, or
relayed with this network's own number as SNET for a request sent
straight to the device. A rise means a station is answering for
other devices. RuntimeError before start and after stop.

### BACnetClient.network_reports

```python
def network_reports(self) -> Awaitable[NetworkReportIterator]
```

Follow the I-Am-Router-To-Network, Network-Number-Is,
Reject-Message-To-Network and Initialize-Routing-Table-Ack (query
answers, #2612) messages the client receives from now on (#1850).
RuntimeError before start and after stop.

### BACnetClient.frames

```python
def frames(self, capacity: int = 1024) -> Awaitable[FrameIterator]
```

Observe the raw frames this client's own transport sends and
receives from now on (B/IP, B/IPv6, SC or MS/TP), `capacity`
frames queued at most (1 to 65536, else ValueError). RuntimeError
before start and after stop; a router application client
(`transport="router"`) has no link of its own and raises
ValueError. Stop ends the iterator; a restart needs a new one.

### BACnetClient.i_am_arrivals

```python
def i_am_arrivals(self, capacity: int = 1024) -> Awaitable[IAmArrivalIterator]
```

Follow every I-Am this client's own transport receives from now
on (B/IP, B/IPv6, SC or MS/TP), one `IAmArrival` each (#2612),
from a tap queue of `capacity` frames of every kind (1 to 65536,
else ValueError). A standalone client sends no I-Am, so nothing is
left out: an I-Am from its own address (another node using it, or
one a NAT site reuses) is a record. A client an endpoint's
`client()` returns follows the endpoint's rule, and is how
`ScEndpoint` and `MstpEndpoint` give arrivals. RuntimeError before start and after stop;
a router application client raises ValueError. Stop ends the
iterator.

### BACnetClient.discovered_devices

```python
def discovered_devices(self) -> Awaitable[list[DiscoveredDevice]]
```

Return all devices discovered so far, one per address: a device
instance two addresses claim is listed for each (#2560).

### BACnetClient.get_device

```python
def get_device(self, instance: int) -> Awaitable[Optional[DiscoveredDevice]]
```

Look up a discovered device by instance number: None when no
address claims it, `BacnetAmbiguousDeviceError` when two or more
do.

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

Like every `_from_device` and `_to_device` method, it raises
`BacnetAmbiguousDeviceError` before sending anything when two or
more addresses claim the instance.

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

### BACnetClient.export_223_experimental

```python
def export_223_experimental(
    self,
    device_instance: int,
    *,
    types: Optional[list[ObjectType]] = None,
) -> Awaitable[Inventory223Experimental]
```

Read a device's objects for the experimental Standard 223 export
(#2436): its summary, its Object\_List, then the objects of the
selected types a few a request. `to_turtle()` on the result writes
the Turtle.

The device is looked up in the discovery table, as the
`_from_device` methods look it up. `types` narrows the objects read
(analog, binary and multi-state inputs, outputs and values, and
integer values; all ten by default). Another type or an empty list
raises ValueError before anything is sent.

A device that doesn't answer the summary read, or a failed read of the
Object\_List, raises the usual error; so does a device that can't send
the list whole, is read element by element, and fails an element; a
note on that exception names the element. A device that goes silent
while its objects are read gives an inventory with `stopped` set and
the objects read before it. `stop()`, and leaving `async with`,
wait for a running export to finish.

### BACnetClient.add_device

```python
def add_device(self, device_instance: int, address: str) -> Awaitable[None]
```

Add a device to the discovery table manually (useful when address is known without WhoIs).

The table holds one device per address: adding an instance at an
address that holds another replaces it, so `get_device()` no longer
finds the instance that was there (#2560).

### BACnetClient.subscribe_cov

```python
def subscribe_cov(
    self,
    address: str | RoutedTarget,
    subscriber_process_identifier: int,
    monitored_object_identifier: ObjectIdentifier,
    confirmed: bool,
    lifetime: Optional[int] = None,
) -> Awaitable[None]
```

Subscribe to Change-of-Value notifications for an object.

lifetime=None or 0 is indefinite; positive values are seconds.
Use unsubscribe\_cov for cancellation. confirmed always supplies the mode.
`address` is a `str` on the local link, or a `RoutedTarget` for
a station behind a router (#2588), as `RoutedTarget` describes. A
routed device notifies back through its router, so its notifications
have `source_network` and `source_address` set to the station's
network and MAC.

### BACnetClient.unsubscribe_cov

```python
def unsubscribe_cov(
    self,
    address: str | RoutedTarget,
    subscriber_process_identifier: int,
    monitored_object_identifier: ObjectIdentifier,
) -> Awaitable[None]
```

Cancel a COV subscription.

`address` is a `str` or a `RoutedTarget`, as for
`subscribe_cov`. The device finds the subscription by the address
the request came from and the process and object, so name the target
the subscription named: the same `RoutedTarget`, or the same
`str`. The same device reached another way, such as by its own
`ip:port` after a routed subscription, answers with success and
keeps the subscription.

### BACnetClient.subscribe_cov_property

```python
def subscribe_cov_property(
    self,
    address: str | RoutedTarget,
    subscriber_process_identifier: int,
    monitored_object_identifier: ObjectIdentifier,
    property_identifier: PropertyIdentifier,
    confirmed: bool,
    lifetime: int,
    array_index: Optional[int] = None,
    cov_increment: Optional[float] = None,
) -> Awaitable[None]
```

Subscribe to Change-of-Value notifications for one property of an
object (SubscribeCOVProperty, #2588).

`address` is a `str` or a `RoutedTarget`, as for
`subscribe_cov`. `lifetime` is required, 1 or more seconds: such
a subscription can't be indefinite, so 0 raises ValueError before
anything is sent; renew by subscribing again before it runs out.
`array_index` names one element of an array property;
`cov_increment` overrides the object's own increment for a numeric
property. Notifications arrive on `cov_notifications()`. Give each
subscription a process identifier of its own: an object subscription
and a property subscription on one object with one identifier send
notifications that can't be told apart.

### BACnetClient.unsubscribe_cov_property

```python
def unsubscribe_cov_property(
    self,
    address: str | RoutedTarget,
    subscriber_process_identifier: int,
    monitored_object_identifier: ObjectIdentifier,
    property_identifier: PropertyIdentifier,
    array_index: Optional[int] = None,
) -> Awaitable[None]
```

Cancel a single-property COV subscription.

Name the target, process, object, property and array index the
subscription named: the device finds it by those and the address
the request came from, as for `unsubscribe_cov`.

### BACnetClient.subscribe_cov_property_multiple

```python
def subscribe_cov_property_multiple(
    self,
    address: str | RoutedTarget,
    subscriber_process_identifier: int,
    specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int], Optional[float], bool]]]],
    issue_confirmed_notifications: bool,
    max_notification_delay: Optional[int] = None,
    lifetime: Optional[int] = None,
) -> Awaitable[None]
```

Subscribe to COV notifications for multiple properties on multiple objects.

`address` is a `str` or a `RoutedTarget`, as for
`subscribe_cov` (#2588). The notifications such a subscription
brings arrive on `cov_notifications()` as `CovNotificationMultiple`
objects.
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

Get an async iterator for incoming COV notifications of both kinds:
`CovNotification` (one object) and `CovNotificationMultiple`
(several objects, from `subscribe_cov_property_multiple`). The client
acknowledges a confirmed one of either kind whether or not it knows the
subscription; match one by its `subscriber_process_identifier` and
source.
An unsubscribed sender's broadcast can bring a
`CovNotificationMultiple` too, so check `kind` (or use
`isinstance`) before reading a `CovNotification`'s fields. A
confirmed notification that comes in segments is reassembled, then
acknowledged and yielded once (#2646), and a confirmed notification
sent again, its answer lost, is published again.

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

### BACnetClient.acknowledge_alarm

```python
def acknowledge_alarm(
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
    address: str | RoutedTarget,
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
`address` is a `str` on the local link, or a `RoutedTarget` for
a station behind a router (#2617), as `RoutedTarget` describes.

### BACnetClient.read_log_page

```python
def read_log_page(
    self,
    address: str | RoutedTarget,
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
outside 1..=32767 raises ValueError before I/O. `address` is a
`str` on the local link, or a `RoutedTarget` (#2617); every
request of a page, the count reads included, goes there.

### BACnetClient.atomic_read_file

```python
def atomic_read_file(
    self,
    address: str | RoutedTarget,
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
records than requested, raises BacnetError. `address` is a `str`
on the local link, or a `RoutedTarget` for a station behind a
router (#2613), as `RoutedTarget` describes: the answer, segmented
or not, and a reject, error or abort come back as from a local read.

### BACnetClient.atomic_write_file

```python
def atomic_write_file(
    self,
    address: str | RoutedTarget,
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
`address` is a `str` or a `RoutedTarget`, as for
`atomic_read_file` (#2613).

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
as its BDT: the BVLC Write-Broadcast-Distribution-Table, the path
for an older BBMD. A BBMD that follows the 2020 standard refuses with
0x0010; write its Network Port's BBMD\_Broadcast\_Distribution\_Table
with `PropertyValue.bdt(rows)` instead (#2710).

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
def foreign_registration(
    self,
) -> Awaitable[Union[ForeignRegistrationStatus, Bip6ForeignRegistrationStatus]]
```

How the foreign-device registration is going (#938), a refusal
included. Sends nothing. Outside `foreign_device=` only manual
registrations show. A `transport="ipv6"` client returns a
`Bip6ForeignRegistrationStatus` (#2365). RuntimeError before start
and after stop; a client on another transport raises BacnetError.

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

Delays must be nonzero, initial \<= maximum, and maximum \<= 600,000 ms
(ten minutes, the most Annex AB.6.1 lets the reconnect wait grow to).
Invalid settings raise ValueError; unsigned integer overflow raises
OverflowError. Validation is not deployment qualification.

After a lost connection each round waits its backoff, then tries the
primary hub and, when that fails, the failover hub. max\_retries bounds
the rounds; 0 turns recovery off (the initial connection and primary
restoration still run). None keeps going while a hub can be dialled and
needs initial\_delay\_ms \>= 1000, else ValueError.

Properties:

- `initial_delay_ms: int`
- `max_delay_ms: int`
- `max_retries: int | None`

### ScReconnect.__init__

```python
def __init__(
    self,
    *,
    initial_delay_ms: int = 10000,
    max_delay_ms: int = 600000,
    max_retries: int | None = 10,
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

Recovery runs in rounds that try the primary, then the failover hub.
Losing the primary reads `disconnected`, then per round
`reconnect_attempt` and `connect_failed` for the primary, and
`reconnect_attempt`, `failover_engaged` and `connected` (or
`connect_failed`) for the failover hub. Both attempts of a round share
its number and its one backoff. `failover_engaged` comes once the
failover dial succeeds, before its handshake, and can repeat once per
round. A primary
restore reads `primary_restored`, then `connected` on the primary;
each failed restore probe, at most one per restore interval, reads
`primary_restore_failed` (#1828).

Properties:

- `kind: Literal['connected', 'disconnected', 'connect_failed', 'reconnect_attempt', 'failover_engaged', 'primary_restored', 'primary_restore_failed', 'reconnect_gave_up', 'other']`
- `hub: Literal["primary", "failover"] | None`: The hub connected to, lost, failed or being dialled; `primary` for `primary_restore_failed`.
- `hub_vmac: bytes | None`: The hub's VMAC from its Connect-Accept, on `connected`.
- `cause: Literal['local_close', 'peer_close', 'heartbeat_timeout', 'websocket', 'websocket_closed', 'tls', 'nak', 'dial_timeout', 'connect_accept_timeout', 'upgrade_refused', 'malformed_result', 'unexpected_connect_reply', 'rejection_expired', 'other'] | None`: Why a `disconnected` link ended, or a `connect_failed` attempt or `primary_restore_failed` probe failed. `peer_close`: a Disconnect-Request from the hub; `websocket_closed`: the hub closed the WebSocket with `close_status` (#2438); `dial_timeout`: the dial (host lookup, TCP, TLS, WebSocket upgrade) ran out of time; `connect_accept_timeout`: the hub never answered the Connect-Request; `upgrade_refused`: the server answered the upgrade with `http_status` (#1839). `cause_code` names it.
- `cause_code: ErrorCode | None`: The BACnet error code that names the cause (Annex AB.7.5): a NAK's own code, or the BACnet/SC connection code for a DNS, TCP, TLS, HTTP or WebSocket failure, such as `TCP_CONNECTION_REFUSED` or `WEBSOCKET_ENDPOINT_LEAVES`. None when no code applies, as for a local close or a heartbeat timeout (#2438).
- `close_status: int | None`: The status in the hub's WebSocket close frame, for a `websocket_closed` cause, such as 1001 for a hub going away.
- `dial_stage: Literal['host_lookup', 'tcp_connect', 'tls', 'websocket_upgrade', 'other'] | None`: Where a `dial_timeout` dial stalled; None when it did not say.
- `http_status: int | None`: The HTTP status of an `upgrade_refused`, such as a proxy's 502.
- `cause_detail: str | None`: Diagnostic text for a `websocket`, `tls` or `other` cause; not stable.
- `error_class: ErrorClass | None`: The NAK's error class, for a `nak` cause.
- `error_code: ErrorCode | None`: The NAK's error code, for a `nak` cause.
- `attempt: int | None`: The reconnect round, starting at 1 and saturating at 2\*\*32 - 1. For `reconnect_gave_up`, the last round dialled, or zero.
- `backoff: float | None`: Seconds waited before the round containing this `reconnect_attempt`.

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

## Frame

One frame the owner's transport sent or received, frozen at capture.

The octets are raw; `decode_frame(frame.octets, frame.link)` gives the
decoded header summary. `local_address` and `peer` use one form per
link (see "Raw frame observation" in docs/python-api.md).

Properties:

- `monotonic: float`: Capture time in seconds on the clock and origin of `time.monotonic()`, so `time.monotonic() - frame.monotonic` is the frame's age. It never decreases within a subscription. The first `frames()` call in the process pairs the capture clock with `time.monotonic()` once, and each capture converts by its offset from that pair. Both read the same OS clock on Linux (`CLOCK_MONOTONIC`), on macOS (`mach_absolute_time`) and on Windows from Python 3.13 (`QueryPerformanceCounter`); neither counts system sleep on Linux or macOS, both count it on Windows. On Windows with Python 3.11 or 3.12, `time.monotonic()` reads `GetTickCount64` instead: it also counts sleep, but moves in 10-16 ms steps, so a value can sit up to one step away from it, and the two counters are not guaranteed to stay in step over a long run.
- `timestamp: float`: Capture time as POSIX seconds from the wall clock, as `time.time()`: comparable with other machines' clocks, but a clock adjustment can move it either way, so order frames by `monotonic`.
- `direction: Literal["inbound", "outbound", "other"]`: `"inbound"` or `"outbound"` (`"other"` only for a direction a later release adds).
- `link: Literal["bip", "ipv6", "sc", "mstp", "other"]`: The link, as the `transport=` argument spells it; `"other"` for a link no Python owner runs.
- `local_address: str`: This node's address on the link when the subscription began, in the form `peer` uses for that link: `"ip:port"` on B/IP, `"[ip]:port"` on B/IPv6 (the UDP socket, not the VMAC), the VMAC in colon hex on SC, the station in decimal on MS/TP.
- `peer: str`: The other end on the link: the datagram's source or destination on B/IP (`"ip:port"`) and B/IPv6 (`"[ip]:port"`, the form `resolve_vmac` takes), the hub connection on SC (`"primary"` or `"failover"`, as `ScEvent.hub`), the station in decimal on MS/TP (`"255"` for a broadcast). The next hop, not a forwarded frame's original source.
- `octets: bytes`: The frame as the link carries it, malformed received datagrams included.
- `missed_before: int`: Frames this subscription missed just before this one: dropped by its full queue (which keeps the oldest frames and drops the newest), or lost on the way to Python together with the gap such a frame carried. A frame redelivered after a cancelled read keeps its value. Summed over the frames received, plus any lost after the last one, it is `FrameIterator.missed`.

## FrameSummary

Frozen offline header summary. Missing fields are None.

parse\_error reports header decoding failures, while problem also reports
valid protocol refusals. Service parameters are not exhaustively validated.
original\_source is the originating address a BBMD wrote into a B/IP
Forwarded-NPDU, or a B/IPv6 Forwarded-NPDU or Forwarded-Address-Resolution;
Frame.peer is the actual transport peer. Neither field proves the sender's
identity or its path. Summaries compare and hash by value. A summary holds
its fields only, never the frame's octets: copy and pickle carry those
fields and nothing else.

Properties:

- `link_message: str`: BVLC function or MS/TP frame type, such as original-unicast-npdu. Not the data link; that is Frame.link.
- `broadcast: bool`
- `link_source: str | None`
- `link_destination: str | None`
- `original_source: str | None`
- `snet: int | None`
- `dnet: int | None`
- `routed_source: str | None`
- `routed_destination: str | None`
- `network_message: str | None`
- `pdu: str | None`
- `invoke_id: int | None`
- `service: str | None`
- `problem: str | None`
- `parse_error: str | None`
- `device: int | None`

## FieldNode

One immutable decoded field, with its source span.

Two nodes are equal, and hash alike, when their labels, spans,
device-text flags and children all are. `copy` and `pickle` rebuild
a node from those; nothing is decoded again.

Properties:

- `label: str`
- `span: tuple[int, int]`: Start and exclusive end offsets in the supplied frame.
- `children: tuple[FieldNode, ...]`
- `device_text: bool`: Whether the label quotes a character string supplied by the device.

## FrameIterator

Bounded raw-frame subscription. A full queue drops new arrivals.

Cancelling a read preserves the next frame and the subscription. Explicit
close or owner shutdown discards queued frames and wakes pending reads.
A native delivery admitted before close may still complete its asyncio Future.
Each subscription is independent and never keeps the owner's socket alive.
Read from one running event loop; concurrent reads keep arrival order.

Properties:

- `missed: int`: Frames lost so far, live: dropped by the full queue or lost on the way to Python, including any lost after the last frame read, which no `missed_before` has reported yet. Readable after close.

### FrameIterator.__aiter__

```python
def __aiter__(self) -> FrameIterator
```

### FrameIterator.__anext__

```python
def __anext__(self) -> Awaitable[Frame]
```

### FrameIterator.aclose

```python
def aclose(self) -> Awaitable[None]
```

Idempotently stop this subscription, without stopping its owner.

## IAmArrival

One I-Am the owner's link handed it, frozen at arrival (#2612).

Every I-Am frame received is a record, in arrival order: repeats, and
two devices claiming one instance at different addresses, included.
`link_sender` is who put the frame on this node's link, in
`Frame.peer`'s form; `link_source` is the source MAC the stack takes,
in the address form the calls take on that link (`"ip:port"`,
`"bip6:0x0004D2"`, an SC VMAC in colon hex, `"mstp:5"`). See "I-Am
arrivals" in docs/python-api.md for who is who on each link. The
parameters are kept as sent, odd ones included. A record compares,
hashes, copies and pickles by its fields.

The tap shows frames before the stack filters them, so a record does
not prove the discovery table took the I-Am.

Properties:

- `monotonic: float`: Arrival time on `time.monotonic()`'s clock and origin, as `Frame.monotonic` (with its Windows note).
- `timestamp: float`: Arrival time as POSIX seconds from the wall clock, as `Frame.timestamp`.
- `link: Literal["bip", "ipv6", "sc", "mstp", "other"]`: The link, as `Frame.link`.
- `object_identifier: ObjectIdentifier`: The I-Am's device identifier, of the type it was sent with, as `DiscoveredDevice.object_identifier`; `.instance` is the device instance.
- `max_apdu_length: int`: Max APDU length accepted, as sent (below 50 included).
- `segmentation_supported: Segmentation`: Segmentation supported, as sent (a value outside the enumeration included).
- `vendor_id: int`: Vendor identifier, as sent.
- `link_sender: str`: Who put the frame on this node's link, as `Frame.peer` writes it: the UDP source on B/IP and B/IPv6 (for a Forwarded-NPDU the BBMD that forwarded it), `"primary"` or `"failover"` on SC, the source station in decimal on MS/TP.
- `link_source: str`: The source MAC the stack takes for the frame: the origin a Forwarded-NPDU carries, else the sender, on B/IP; the source VMAC on B/IPv6 (the original source's when forwarded); the originating VMAC on SC; the source station on MS/TP. The router when `source_network` is set: the next hop back, which a node may take as that network's router (Clauses 6.5.3, 6.5.4).
- `original_source: Optional[str]`: The originating node's `"ip:port"` or `"[ip]:port"` a BBMD wrote into a B/IP or B/IPv6 Forwarded-NPDU; None otherwise.
- `forwarded: bool`: True when the frame carried an origin other than its sender: a BBMD forwarded it. Never on SC or MS/TP.
- `source_network: Optional[int]`: SNET, when a router relayed the I-Am from another network.
- `source_address: Optional[bytes]`: SADR, the device's MAC on `source_network`.
- `destination_network: Optional[int]`: DNET carried by the received NPDU.
- `destination_address: Optional[bytes]`: DADR, empty for a network broadcast and absent without a DNET.
- `broadcast: bool`: A link broadcast or a broadcast on a network. False for an MS/TP I-Am one station unicast to another, which the bus lets this station hear.
- `device_address: Optional[str]`: The device's address as `BACnetClient.device()` takes it: `link_source` for a device on this link, `"5:0x07"` for one a router relayed. None when those name no one station. The calls that take an `address` don't take `"5:0x07"`: for a routed record, `read_property`, `read_property_multiple`, `read_range`, `read_log_page`, `write_property`, `atomic_read_file` and `atomic_write_file` take `RoutedTarget(rec.source_network, rec.source_address, router=rec.next_hop_router)`.
- `next_hop_router: Optional[str]`: The router that relayed it, when `source_network` is set: its `link_source`.
- `forwarding_bbmd: Optional[str]`: The BBMD that forwarded it, when `forwarded`: its `link_sender`.
- `missed_before: int`: Frames of any kind this subscription lost just before this record, as `Frame.missed_before`: the tap cannot tell which of them held an I-Am. Summed over the records received, plus any lost after the last one, it is `IAmArrivalIterator.missed`. An own echo left out is no loss and adds nothing.

## IAmArrivalIterator

A bounded subscription to the I-Ams the owner's link hands it,
with `FrameIterator`'s semantics: the queue holds frames of every kind
(`capacity`), a full queue drops new frames, a cancelled read keeps
the next record, and close or owner shutdown wakes pending reads. It
never keeps the owner's socket alive. With a subscriber the transport
copies every frame once, as for `frames()`; the decode runs where the
records are read, never on the transport's path.

Properties:

- `missed: int`: Frames of any kind lost so far, live, as `FrameIterator.missed`. Readable after close.
- `malformed: int`: Inbound I-Am frames read so far that made no record: parameters that don't decode, or an SC message naming no originating VMAC. `frames()` and `decode_frame` show who sent one and why it failed. A frame whose headers don't decode is not counted: it is not known to hold an I-Am. Readable after close.
- `own_echoes: int`: The owner's own I-Ams come back to it, read so far and left out: on a `BipEndpoint` and on the client its `client()` returns, those from the endpoint's own address naming its own device instance, whether its broadcast returned or a BBMD sent it back. Always 0 on a standalone `BACnetClient`, which sends no I-Am and leaves nothing out. Readable after close.

### IAmArrivalIterator.__aiter__

```python
def __aiter__(self) -> IAmArrivalIterator
```

### IAmArrivalIterator.__anext__

```python
def __anext__(self) -> Awaitable[IAmArrival]
```

### IAmArrivalIterator.aclose

```python
def aclose(self) -> Awaitable[None]
```

Idempotently stop this subscription, without stopping its owner.

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
