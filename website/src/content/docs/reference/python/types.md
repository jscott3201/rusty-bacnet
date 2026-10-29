---
title: "Python types and values"
description: "Object identifiers, property values, structured values and B/IP modes, generated from the type stub."
---

:::note[On `dev` after the release]
This page is generated from the type stub on the `dev` branch and tracks the next release, so it can list classes, methods and keywords merged after v0.15.0 that the released wheel doesn't have. The [v0.15.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) and the [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d) say what changed.
:::

Identifiers, property values, structured values and B/IP modes. The [Python reference](/rusty-bacnet/reference/python/) lists every group. Each page opens with its main classes, then the other classes, the dictionaries (`TypedDict`) and the type aliases, in stub order.

<!-- generated:start by website/scripts/reference-python.py; do not edit this page -->

## ObjectIdentifier

BACnet Object Identifier (10-bit type + 22-bit instance number).

For u32-representable values, types above 1023 or instances above 4,194,303
raise ValueError. Integers outside u32 raise OverflowError.
Valid proprietary types and the wire wildcard instance are accepted.

`copy.copy`, `copy.deepcopy` and `pickle` (every protocol) rebuild
an equal identifier through this constructor.

Properties:

- `object_type: ObjectType`
- `instance: int`

### ObjectIdentifier.__init__

```python
def __init__(self, object_type: ObjectType, instance: int) -> None
```

## BACnetTimeStamp

Lossless BACnetTimeStamp CHOICE.

Time components accept their normal BACnet ranges or 255 for unspecified.
Date accepts full years 1900..2154 or 255 for unspecified, months 1..14,
days 1..34, and days-of-week 1..7; each non-year date field also accepts
255 for unspecified. Supplied values are never normalized.

`copy.copy`, `copy.deepcopy` and `pickle` (every protocol) rebuild
an equal timestamp from its encoded CHOICE, so one read from a peer with
a field outside those ranges copies too.

Properties:

- `kind: str`: Selected CHOICE: `sequence_number`, `time`, or `date_time`.
- `value: int | tuple[int, int, int, int] | tuple[tuple[int, int, int, int], tuple[int, int, int, int]]`: Exact selected value, using a full year for the Date tuple (255 when unspecified), as `PropertyValue.date` reads.

### BACnetTimeStamp.sequence_number

```python
@staticmethod
def sequence_number(value: int) -> BACnetTimeStamp
```

Construct the Sequence Number CHOICE with a value in 0..65535;
another integer raises OverflowError.

### BACnetTimeStamp.time

```python
@staticmethod
def time(hour: int, minute: int, second: int, hundredths: int) -> BACnetTimeStamp
```

Construct the Time CHOICE.

### BACnetTimeStamp.date_time

```python
@staticmethod
def date_time(
    date: tuple[int, int, int, int],
    time: tuple[int, int, int, int],
) -> BACnetTimeStamp
```

Construct DateTime from `(full_year, month, day, day_of_week)` and Time tuples.

## BitString

A BACnet bit string (#2080): the wire form plus its bits and names.

`bits` has one bool per bit, bit 0 first. `names` lists the set bits
by their constant names (`"IN_ALARM"`, `"TO_OFFNORMAL"`) when the bit
string's type is known from where it was read, or is None when it isn't.
Two bit strings are equal when their wire forms are; `names` doesn't
take part.

Properties:

- `unused_bits: int`: How many trailing bits of the final octet of `data` are padding.
- `data: bytes`: The octets as carried on the wire.
- `bits: list[bool]`: One bool per bit, bit 0 first.
- `names: list[str] | None`: The set bits by name when the type is known, else None. A set bit without a name (a vendor or reserved position) is left out.

### BitString.__init__

```python
def __init__(
    self,
    unused_bits: int,
    data: bytes,
    property: Optional[PropertyIdentifier] = None,
) -> None
```

A bit string from its wire form. With `property`, the bits are
named as that property's bit-string type names them (Status\_Flags,
Event\_Enable, Acked\_Transitions, Limit\_Enable,
Protocol\_Services\_Supported, Protocol\_Object\_Types\_Supported).
`unused_bits` is 0 to 7, and 0 when `data` is empty; anything
else raises ValueError.

### BitString.__len__

```python
def __len__(self) -> int
```

### BitString.__iter__

```python
def __iter__(self) -> Iterator[bool]
```

The bits as bools, bit 0 first.

### BitString.__getitem__

```python
def __getitem__(self, index: int) -> bool
```

Bit `index` (negative counts from the last bit); IndexError
past the end.

## PropertyValue

A decoded BACnet property value with tag and Python-native value.

Create values using the static constructors:

```python
PropertyValue.null()
PropertyValue.boolean(True)
PropertyValue.unsigned(42)
PropertyValue.real(3.14)
PropertyValue.character_string("hello")
PropertyValue.object_identifier(oid)
```

A read result (`read_property`, `read_property_multiple`, a
`CovNotification` value and `BACnetServer.read_property`) keeps every
element of the value; only broken framing raises:

- A constructed property with a typed form reads typed. A whole read of
a collection is a `list` of its elements, an indexed read one
element, and a read of a single value that element, tagged with its
production. Where the binding also takes the property as a typed
value, the form is that value's. The properties and forms (see
docs/python-api.md, "Typed constructed values"): Recipient\_List
(`"destination"`: a `Destination` with every key), Port\_Filter
(`"port_permission"`: `(port_id, enabled)`), a Group's
List\_Of\_Group\_Members (`"read_access_specification"`:
`(object_id, [(property_id, array_index), ...])`) and Present\_Value
(`"read_access_result"`: a `ReadAccessResult`), a Command's Action
(`"action_list"`: a list of `ActionCommand` with every key), the
object reference lists and Accompaniment
(`"device_object_reference"`: an `ObjectIdentifier`, or
`(device, object)`), Supported\_Formats
(`"authentication_factor_format"`), Authentication\_Policy\_List
(`"authentication_policy"`: an `AuthenticationPolicy`), Stages
(`"stage_limit_value"`: `(limit, values, deadband)`), Access Rights
rules (`"access_rule"`:
an `AccessRule` with every key), property reference lists and the
single Trend Log Log\_DeviceObjectProperty or Averaging and Event
Enrollment Object\_Property\_Reference
(`"device_object_property_reference"`: a
`DeviceObjectPropertyReference` with every key),
Event\_Algorithm\_Inhibit\_Ref (`"object_property_reference"`: the same
mapping without `device_identifier`), a Global Group's
Present\_Value (`"property_access_result"`),
Audit\_Notification\_Recipient (`"recipient"`: an
`AuditRecipientInput`), Weekly\_Schedule (`"daily_schedule"`),
Exception\_Schedule (`"special_event"`), Effective\_Period
(`"date_range"`), Date\_List (`"calendar_entry"`), timestamps
(`"timestamp"`: a `BACnetTimeStamp`), Active\_COV\_Subscriptions
(`"cov_subscription"`), Device\_Address\_Binding
(`"address_binding"`: `device_identifier`, `network_number` and
`mac_address`), Value\_Source and Value\_Source\_Array
(`"value_source"`), an Accumulator's Scale (`"scale"`: a
`float` or an `int`) and Prescale (`"prescale"`: `(multiplier, modulo_divide)`), Load Control's Requested\_Shed\_Level,
Expected\_Shed\_Level and Actual\_Shed\_Level (`"shed_level"`: a mapping
of `kind`, `percent`, `level` or `amount`, and `value`), an
Access Point's Access\_Event\_Authentication\_Factor and a Credential
Data Input's Present\_Value (`"authentication_factor"`:
`(format_type, format_class, value)`, what
`report_access_event_local` and `report_credential_read_local`
take), and an Event Enrollment's Event\_Parameters
(`"event_parameter"`: a mapping whose `kind` names the algorithm;
see docs/python-api.md). Access\_Event\_Time and Last\_Restore\_Time read
as `"timestamp"`, and Subordinate\_List as a list of
`"device_object_reference"`. Tags elements are `"name_value"`
mappings with `"name": str` and `"value": None | PropertyValue`. Semantic tags
have `None`; valued NULL is `PropertyValue.null()`. Date and Time
are separate primitive values, not a combined pair. Whole Tags reads
are lists, indexed reads one element, and index zero an unsigned count.
Each element keeps its octets, so the value
writes back unchanged. A value that isn't those elements, to the last
octet, follows the rules below.
- Other context-tagged content (a Load Control's shed levels, an Event
Enrollment's Event\_Parameters), or content this type has no form for
(a UCS-4, DBCS or JIS string, bad UTF-8, an ENUMERATED past 32 bits),
is `application_data` holding the octets exactly as served.
- A whole read (no `array_index`) of a property the stack's
classification table marks as an array or list on that object type
(every BACnetARRAY and BACnetLIST of the 2020 object tables) is a
`list` at every length, 0 and 1 included. A DateTime Value or
DateTime Pattern Value's Priority\_Array is such a list of 16 slots: a
set slot, the date-time choice under context tag 1, is
`application_data` whose `.value` is `[date, time]` (#2098).
- Any other read is the bare value when it holds one element, and a
`list` in wire order when it holds none or several (a date-time is
a date and then a time). An indexed read is one element under these
rules: `Port_Filter[2]` is a port\_permission, `Event_Time_Stamps[1]`
a timestamp.

Two values are equal when they have the same tag and value: numbers
compare as numbers (`real(0.0) == real(-0.0)`, and a NaN equals
nothing), a list item by item, and a typed read by its octets and
element production. Equal values hash alike.

`copy.copy`, `copy.deepcopy` and `pickle` (every protocol) rebuild
an equal value through the constructor its `tag` names, a list from
its items as PropertyValues, and a typed constructed element from the
octets it was read from.

Properties:

- `tag: str`: Type tag: 'null', 'boolean', 'unsigned', 'signed', 'real', 'double', 'octet\_string', 'character\_string', 'bit\_string', 'enumerated', 'date', 'time', 'object\_identifier', 'list', 'application\_data', or one typed constructed element: 'destination', 'port\_permission', 'read\_access\_specification', 'read\_access\_result', 'action\_list', 'device\_object\_reference', 'authentication\_factor\_format', 'stage\_limit\_value', 'access\_rule', 'device\_object\_property\_reference', 'object\_property\_reference', 'property\_access\_result', 'recipient', 'daily\_schedule', 'special\_event', 'calendar\_entry', 'date\_range', 'timestamp', 'cov\_subscription', 'value\_source', 'scale', 'prescale', 'address\_binding', 'shed\_level', 'authentication\_factor', 'event\_parameter'.
- `value: Any`: The Python-native value (int, float, str, bytes, bool, dict, tuple, ObjectIdentifier, BitString, list, or None); `application_data` is `bytes` (except a DateTime Value's set Priority\_Array slot, `[date, time]`), and a typed constructed element the form its typed write takes, or the form docs/python-api.md gives it. A date is `(year, month, day, day_of_week)` with the full year, or 255 for an unspecified year, as in every other date the binding reads. A bit string is a `BitString`, named when the value was read from a property of known bit-string type.
- `name: str | None`: The name of an enumerated value read from a property whose enumeration is known (`"HIGH_LIMIT"` for an Event\_State of 3), else None. `value` stays the number. Values built with the constructors carry no property, so their name is None; copies and pickles of a read value keep it.
- `octets: bytes`: The value's octets as a write of it carries them: a typed constructed element's are the octets it was read from, a whole typed collection's its elements' back to back, and any other value its application-tagged encoding. Raises ValueError for a value that doesn't encode.

### PropertyValue.null

```python
@staticmethod
def null() -> PropertyValue
```

### PropertyValue.boolean

```python
@staticmethod
def boolean(value: bool) -> PropertyValue
```

### PropertyValue.unsigned

```python
@staticmethod
def unsigned(value: int) -> PropertyValue
```

### PropertyValue.signed

```python
@staticmethod
def signed(value: int) -> PropertyValue
```

### PropertyValue.real

```python
@staticmethod
def real(value: float) -> PropertyValue
```

### PropertyValue.double

```python
@staticmethod
def double(value: float) -> PropertyValue
```

### PropertyValue.character_string

```python
@staticmethod
def character_string(value: str) -> PropertyValue
```

### PropertyValue.octet_string

```python
@staticmethod
def octet_string(value: bytes) -> PropertyValue
```

### PropertyValue.enumerated

```python
@staticmethod
def enumerated(value: int) -> PropertyValue
```

### PropertyValue.object_identifier

```python
@staticmethod
def object_identifier(oid: ObjectIdentifier) -> PropertyValue
```

### PropertyValue.date

```python
@staticmethod
def date(year: int, month: int, day: int, day_of_week: int) -> PropertyValue
```

Create a Date value, in the form `.value` reads it back.

`year` is the full year, 1900..2154, or 255 for unspecified; any
other year raises ValueError. Use 255 for any other unspecified
field.

### PropertyValue.time

```python
@staticmethod
def time(hour: int, minute: int, second: int, hundredths: int) -> PropertyValue
```

Create a Time value. Use 255 for unspecified fields.

### PropertyValue.bit_string

```python
@staticmethod
def bit_string(unused_bits: int, data: bytes) -> PropertyValue
```

Create a BitString value.

### PropertyValue.list

```python
@staticmethod
def list(items: list[PropertyValue]) -> PropertyValue
```

Create a List (array) value from a list of PropertyValue items.
Items that are all elements of one typed constructed collection (from
indexed reads) make that collection, equal to its whole read. Lists
nest at most 32 deep, the decoder's nesting limit; a deeper one
raises ValueError.

### PropertyValue.application_data

```python
@staticmethod
def application_data(bytes: bytes) -> PropertyValue
```

Create an ApplicationData value from pre-encoded application-layer bytes (e.g. a framed BACnetEventParameter).

## DiscoveredDevice

A device discovered via Who-Is / I-Am.

Properties:

- `object_identifier: ObjectIdentifier`
- `mac_address: bytes`: The MAC the I-Am came from: IPv4 and port on B/IP, the 3-octet VMAC on B/IPv6, the station on MS/TP.
- `max_apdu_length: int`
- `segmentation_supported: Segmentation`
- `vendor_id: int`
- `seconds_since_seen: float`
- `source_network: Optional[int]`
- `source_address: Optional[bytes]`

## CovNotification

A Change-of-Value notification received from a remote device.

Properties:

- `subscriber_process_identifier: int`
- `initiating_device_identifier: ObjectIdentifier`
- `monitored_object_identifier: ObjectIdentifier`
- `time_remaining: int`
- `delivery: str`
- `source_mac: bytes`
- `source_network: Optional[int]`
- `source_address: Optional[bytes]`
- `values: Any`: List of property value change entries: dicts with `property_id`, `array_index` and `value`. `value` is a `PropertyValue` shaped as a read result is, or `bytes` when the octets' framing is broken.

## CovNotificationIterator

Async iterator yielding `CovNotification` objects. One that falls behind
the client's COV channel loses the oldest and adds them to `missed`. A
cancel that lands while a notification is being delivered, as a timed-out
`asyncio.wait_for` does, puts it back: the next `__anext__` returns it
first. Concurrent `__anext__` calls complete one at a time. Read it from
one running event loop (`asyncio.run` is fine): an item on its way to a
read whose loop stops for good holds up later reads, which raise one
`RuntimeWarning` after 5 s and keep waiting. Iteration ends when the
client stops.

Properties:

- `missed: int`: Notifications lost so far: because this iterator fell behind, or dropped on the way to a read, as when its event loop closed first.

### CovNotificationIterator.__aiter__

```python
def __aiter__(self) -> CovNotificationIterator
```

### CovNotificationIterator.__anext__

```python
def __anext__(self) -> Awaitable[CovNotification]
```

## IHave

An I-Have this client received (#1850), frozen.

Properties:

- `device_identifier: ObjectIdentifier`
- `object_identifier: ObjectIdentifier`
- `object_name: str`
- `source_mac: bytes`: The link-layer peer it came from: the device, or the router that passed it on.
- `source_network: int | None`: SNET, when a router passed it on from another network.
- `source_address: bytes | None`: SADR, when a router passed it on from another network.

## IHaveIterator

Async iterator over every I-Have from one `i_have_events()` call. The
client never waits for it: one more than 256 behind loses the oldest and
adds them to `missed`. A cancel that lands while an item is being
delivered puts it back: the next `__anext__` returns it first.
Concurrent `__anext__` calls complete one at a time. Read it from one
running event loop (`asyncio.run` is fine): an item on its way to a read
whose loop stops for good holds up later reads, which raise one
`RuntimeWarning` after 5 s and keep waiting. Iteration ends when the
client stops.

Properties:

- `missed: int`: I-Haves lost so far: because this iterator fell behind, or dropped on the way to a read, as when its event loop closed first.

### IHaveIterator.__aiter__

```python
def __aiter__(self) -> IHaveIterator
```

### IHaveIterator.__anext__

```python
def __anext__(self) -> Awaitable[IHave]
```

## DiscoveredRouter

A router heard from during `discover_routers()` (#1850), frozen.

Properties:

- `mac: bytes`: The router's MAC on the local network: the next hop to `networks`.
- `networks: list[int]`: Every network it listed, ascending, each once.

## NetworkReport

A network-layer report this client received (#1850), frozen. Fields
that don't apply to its `kind` are None.

Properties:

- `kind: Literal['i_am_router_to_network', 'network_number_is', 'reject_message_to_network', 'other']`
- `source_mac: bytes`: The link-layer peer that sent it: for an I-Am-Router-To-Network, the router to send through.
- `source_network: int | None`: SNET, when a router passed it on from another network.
- `source_address: bytes | None`: SADR, when a router passed it on from another network.
- `networks: list[int] | None`: The networks an I-Am-Router-To-Network lists, in its order.
- `network: int | None`: A Network-Number-Is's network, or the one a refused message was for.
- `configured: bool | None`: Whether a Network-Number-Is sender had its number configured.
- `reason: int | None`: A Reject-Message-To-Network's reason octet.

## NetworkReportIterator

Async iterator over the network reports from one `network_reports()`
call. The client never waits for it: one more than 64 behind loses the
oldest and adds them to `missed`. A cancel that lands while an item is
being delivered puts it back: the next `__anext__` returns it first.
Concurrent `__anext__` calls complete one at a time. Read it from one
running event loop (`asyncio.run` is fine): an item on its way to a read
whose loop stops for good holds up later reads, which raise one
`RuntimeWarning` after 5 s and keep waiting. Iteration ends when the
client stops.

Properties:

- `missed: int`: Reports lost so far: because this iterator fell behind, or dropped on the way to a read, as when its event loop closed first.

### NetworkReportIterator.__aiter__

```python
def __aiter__(self) -> NetworkReportIterator
```

### NetworkReportIterator.__anext__

```python
def __anext__(self) -> Awaitable[NetworkReport]
```

## ForeignDevice

A foreign-device registration, for the `foreign_device=` keyword of
`BACnetClient`, `BACnetServer` and `BipEndpoint` (B/IP), and of
`BACnetClient` and `BACnetServer` with `transport="ipv6"` (#1941).

`bbmd` names a B/IP BBMD as `"ip"` or `"ip:port"`, or a B/IPv6 one
as `"ip"`, `"[ip]"` or `"[ip]:port"` (port 47808 by default; an
IPv6 port needs the brackets, since `"2001:db8::10:4780"` is read as a
bare address); the owner's transport must match, else its constructor
raises ValueError. A B/IPv6
BBMD must be a unicast host: an unspecified, multicast, link-local,
IPv4-mapped or IPv4-compatible address, a zone index or port 0 raises
ValueError saying which. `ttl` is how many seconds the BBMD keeps the
entry, 1 to 65535. `renewal` (B/IP only) is the seconds between
registrations, at least 0.1 and less than `ttl`; `None` renews at
half the TTL, which a B/IPv6 foreign device always does (but never more
often than every 30 s), so a B/IPv6 BBMD with `renewal` raises ValueError. Behind NAT,
renew often so the mapping the BBMD's replies come back on stays open,
while the TTL stays long. Invalid values raise ValueError, or
OverflowError for an integer out of range.

The owner registers at start and renews on its own. Its broadcasts go
through the BBMD; on B/IP the first few wait for the first registration
to be answered, and `foreign_registration()` reports how it goes (B/IP
only). A B/IPv6 client needs `ipv6_device_instance`: a B/IPv6 foreign
device uses its instance's VMAC.

Properties:

- `bbmd: str`
- `ttl: int`
- `renewal: Optional[float]`

### ForeignDevice.__init__

```python
def __init__(self, bbmd: str, ttl: int = 300, renewal: Optional[float] = None) -> None
```

## Bbmd

A BBMD set-up, for the `bbmd=` keyword of `BACnetClient`,
`BACnetServer` and `BipEndpoint` (B/IP only).

`bdt` lists the BBMDs of the other subnets, each `"ip:port"` (its
broadcasts forwarded by unicast) or `("ip:port", "mask")`; this BBMD's
own row is added when missing. `accept_foreign_devices` lets foreign
devices register (default False: every registration is refused).
`management_acl` lists the IPv4 addresses allowed to delete Foreign
Device Table entries (default: none). `bdt_persist_path` names a file
holding a BDT, 10 octets per row, to start with instead of `bdt` when
it loads; a BDT activated through the registered Network Port is saved
there. `alone_on_subnet` says this BBMD is the only device of its
B/IP network on its subnet, as behind a NAT router: forwarded and
distributed broadcasts then go only to its peers and foreign devices,
not to its own subnet (default False). `nat_global_address`,
`"ip"` or `"ip:port"`, runs it behind a NAT router that
port-forwards that public address to it (Annex J.7.5): the BDT must list
the address as this BBMD's own row, every row must be two-hop, and the
Forwarded-NPDUs it builds name that address as their origin.
`nat_private_ranges` lists up to four `"a.b.c.d/n"` ranges to count
as private besides RFC 1918: a foreign device registered from a private
address has its broadcasts sent on from the global address
(`"0.0.0.0/0"` makes that every foreign device). A BDT with rows that
doesn't list the global address, or has a one-hop row, raises
ValueError; an empty one serves foreign devices only. The
registered Network Port serves both as BACnet\_IP\_NAT\_Traversal and
BACnet\_IP\_Global\_Address. Write-BDT from the network is always refused.
Invalid values raise ValueError, or OverflowError for an integer out of
range.

Properties:

- `bdt: list[tuple[str, str]]`
- `accept_foreign_devices: bool`
- `management_acl: list[str]`
- `bdt_persist_path: Optional[pathlib.Path]`
- `alone_on_subnet: bool`
- `nat_global_address: Optional[str]`
- `nat_private_ranges: list[str]`

### Bbmd.__init__

```python
def __init__(
    self,
    bdt: Optional[Sequence[str | tuple[str, str]]] = None,
    *,
    accept_foreign_devices: bool = False,
    management_acl: Optional[Sequence[str]] = None,
    bdt_persist_path: Optional[str | os.PathLike[str]] = None,
    alone_on_subnet: bool = False,
    nat_global_address: Optional[str] = None,
    nat_private_ranges: Optional[Sequence[str]] = None,
) -> None
```

## Multicast

A B/IP multicast group (Annex J.8), for the `multicast=` keyword of
`BACnetClient`, `BACnetServer` and `BipEndpoint` (B/IP only).

`group` is a class D IPv4 address. The owner's broadcasts go to the
group, and it takes broadcasts only from the group, never a subnet
broadcast; unicast is unchanged. `ttl`, 1 to 255 (default 1), is the
multicast TTL: 1 keeps the group on the local network, so raise it for a
group that spans IP routers, which must forward multicast. `port` is
the group's UDP port, by default the owner's own; several owners on one
host at one address need their own ports and a shared group port.

`group` can't be in 224.0.0.0/24, the block reserved for local
network control.

Multicast needs an explicit `interface` (not `"0.0.0.0"`), a port
for the group (`port=0` needs `Multicast(port=...)`), and can't be
combined with `share_port_by_address=True` or `foreign_device=`:
the owner's constructor raises ValueError. With `bbmd=` the owner is
the BBMD that serves the group (Annex J.8.3): it forwards the group's
broadcasts to its BDT and foreign devices and theirs to the group, and
its own BDT row must have the mask 255.255.255.255, or start raises. A
registered Network Port serves the group as BACnet\_IP\_Multicast\_Address
(0.0.0.0 when off), writable when the server answers ReinitializeDevice;
on a BBMD a written group is activated together with the BDT written
beside it, both or neither (#1963). A group changed that way lasts until
the process exits, and isn't saved. Two `Multicast` values with the same settings
compare and hash equal. Invalid values raise ValueError, or
OverflowError for an integer out of range. Windows multicast is
untested.

Properties:

- `group: str`
- `ttl: int`
- `port: Optional[int]`

### Multicast.__init__

```python
def __init__(self, group: str, *, ttl: int = 1, port: Optional[int] = None) -> None
```

## UNSPECIFIED

```python
UNSPECIFIED: Literal[255]
```

What an unspecified date or time field holds, the year included: dates
are `(year, month, day, day_of_week)` with the full year, and times
`(hour, minute, second, hundredths)`. `datetime.date(255, ...)` is the
year 255 AD, not a wildcard.

## ActionCommand

`class ActionCommand(TypedDict)`

One write in a Command object's action list (`BACnetActionCommand`).

Unknown keys raise ValueError and wrong types raise TypeError. A
`device_identifier` naming another Device sends that write there as a
confirmed WriteProperty when the server has a binding for the Device;
with none, the command fails when the list runs. A `device_identifier`
that isn't a Device raises ValueError. A read of Action gives each
command in this form, with every key present.

Keys:

- `object_identifier: ObjectIdentifier`
- `property_identifier: PropertyIdentifier`
- `property_value: PropertyValue`
- `property_array_index: NotRequired[int | None]`
- `priority: NotRequired[int | None]`: 1..=16; other values raise BacnetProtocolError (VALUE\_OUT\_OF\_RANGE), and values outside 0..=255 OverflowError.
- `post_delay: NotRequired[int | None]`: Seconds to wait after this write, before the next one or the end.
- `quit_on_failure: NotRequired[bool]`: A failed write with this set stops the rest of the list. Default False.
- `device_identifier: NotRequired[ObjectIdentifier | None]`
- `write_successful: NotRequired[bool]`: A read of Action carries the flag a run sets. add\_command accepts it so a read mapping can be given back, but ignores it: a command starts False.

## DeviceObjectPropertyReference

`class DeviceObjectPropertyReference(TypedDict)`

A property (`BACnetDeviceObjectPropertyReference`), in this device or
the one `device_identifier` names: a Channel or Trend Log Multiple
member, or the property whose value decides when an access rule applies,
such as a Schedule's Present\_Value. A member list also takes an
`(object, property)` or `(object, property, array_index)` tuple for a
property in this device, and stores a member naming the server's own
Device in its local form.

Typed reads include all four keys; absent optional device and array
index fields are `None`. Single reference reads from Trend Log,
Averaging and Event Enrollment use this same mapping. Reserved instance
4194303 remains an explicit ObjectIdentifier, including unset references.

Unknown or missing keys and a `device_identifier` that isn't a Device
raise ValueError, a `property_array_index` outside unsigned32
OverflowError, and wrong types TypeError. The server reads only its own objects, so a Trend Log
Multiple member naming another Device logs a failure instead of a value.
A read of a Channel's, Schedule's or Trend Log Multiple's member list, or
of a Global Group's Group\_Members, gives each reference in this form,
with every key present.

Keys:

- `object_identifier: ObjectIdentifier`
- `property_identifier: PropertyIdentifier`
- `property_array_index: NotRequired[int | None]`: 0..=4294967295; one element of an array property.
- `device_identifier: NotRequired[ObjectIdentifier | None]`

## AccessRule

`class AccessRule(TypedDict)`

One element of an Access Rights rule array (`BACnetAccessRule`).

A missing or `None` `time_range` means the rule applies at any time
(ALWAYS), and a missing or `None` `location` that it covers every
access point (ALL). A `location` is an Access Point or Access Zone, as
an `ObjectIdentifier` in this device or a `(device, object)` pair.
A read of Positive\_Access\_Rules or Negative\_Access\_Rules gives each
rule in this form, with every key present.

Keys:

- `enable: bool`
- `time_range: NotRequired[DeviceObjectPropertyReference | None]`
- `location: NotRequired[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier] | None]`

## AssignedAccessRights

`class AssignedAccessRights(TypedDict)`

One credential assignment; both keys are required.

The reference names Access Rights locally or as a (Device, object) pair.
Instance 4194303 retains the native unused-marker semantics.

Keys:

- `assigned_access_rights: ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]`
- `enable: bool`

## AccessRulePosition

`class AccessRulePosition(TypedDict)`

A rule's original one-based position within its positive or negative array.

Keys:

- `access_rights: ObjectIdentifier`
- `kind: Literal["positive", "negative"]`
- `index: int`

## UnresolvedAccessRights

`class UnresolvedAccessRights(TypedDict)`

An unresolved assignment, retaining its reference and original one-based index.

Keys:

- `index: int`
- `reference: ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]`
- `reason: Literal["remote", "wildcard_device", "not_access_rights", "missing", "unreadable"]`

## AccessRightsEvaluation

`class AccessRightsEvaluation(TypedDict)`

Plain mapping of the native rights stage; every key is always present.

A grant may coexist with unresolved assignments. It is not a final
authorization result. Nullable rule and denial-event evidence use None.
These TypedDicts describe values; no result classes are exported at runtime.

Keys:

- `decision: Literal["granted", "denied", "exempt"]`
- `access_event: int | None`
- `rule: AccessRulePosition | None`
- `unresolved: list[UnresolvedAccessRights]`

## NameValue

`class NameValue(TypedDict)`

One Tags element (`BACnetNameValue`): a tag name, which may not
hold a semicolon, and a primitive value, or `None` (or no `value`
key) for a semantic tag. The `tags` keyword of the object builders
takes a list of these, the form a typed Tags read gives.

Keys:

- `name: str`
- `value: NotRequired[PropertyValue | None]`

## CalendarDate

`class CalendarDate(TypedDict)`

Keys:

- `kind: Literal["date"]`
- `date: BACnetDate`

## CalendarDateRange

`class CalendarDateRange(TypedDict)`

Keys:

- `kind: Literal["date_range"]`
- `start_date: BACnetDate`
- `end_date: BACnetDate`

## CalendarWeekNDay

`class CalendarWeekNDay(TypedDict)`

Keys:

- `kind: Literal["week_n_day"]`
- `month: int`: Each 255 for any.
- `week_of_month: int`
- `day_of_week: int`

## SpecialEvent

`class SpecialEvent(TypedDict)`

One Exception\_Schedule event (`BACnetSpecialEvent`): its period, a
calendar entry or a Calendar's `ObjectIdentifier`, its time-values and
its priority, 1 (highest) to 16.

Keys:

- `period: CalendarEntry | ObjectIdentifier`
- `time_values: list[TimeValue]`
- `priority: int`

## AuditReporterConfiguration

`class AuditReporterConfiguration(TypedDict)`

Owned pre-start target Reporter settings; no Python callbacks.

Keys:

- `instance: int`
- `audit_level: Literal["none", "audit_config", "audit_all"]`
- `auditable_operations: int`
- `issue_confirmed_notifications: bool`
- `monitored_objects: NotRequired[list[ObjectIdentifier | ObjectType | None] | None]`
- `audit_priority_filter: NotRequired[int | None]`
- `maximum_send_delay: NotRequired[int | None]`: Paired Maximum\_Send\_Delay/Send\_Now: None absent; 0..3600 whole seconds.

## AuditRecipientDevice

`class AuditRecipientDevice(TypedDict)`

Audit recipient selected by Device object identifier.

Keys:

- `kind: Literal["device"]`
- `object_identifier: ObjectIdentifier`

## AuditRecipientAddress

`class AuditRecipientAddress(TypedDict)`

Audit recipient selected by BACnet network and MAC address.

Keys:

- `kind: Literal["address"]`
- `network_number: int`
- `mac_address: bytes`

## Destination

`class Destination(TypedDict)`

One Recipient\_List destination (`BACnetDestination`) for
`add_notification_forwarder(recipients=...)`.

Unknown keys, values outside BACnet's range and malformed time tuples
raise ValueError, an integer outside its field's type (unsigned32 for
`process_identifier`, unsigned8 for the bit masks and time fields)
OverflowError, and other wrong types TypeError. A key left out gives a
destination active every day, all day, for every transition, with
unconfirmed notifications. A read of Recipient\_List gives each
destination in this form, with every key present.

Keys:

- `recipient: AuditRecipientInput`
- `process_identifier: int`
- `valid_days: NotRequired[int]`: Bit n is BACnetDaysOfWeek bit n: 1 \<\< 0 is Monday, 1 \<\< 6 Sunday. Default 0x7F.
- `from_time: NotRequired[tuple[int, int, int, int]]`: (hour, minute, second, hundredths); defaults (0, 0, 0, 0) and (23, 59, 59, 99).
- `to_time: NotRequired[tuple[int, int, int, int]]`
- `issue_confirmed_notifications: NotRequired[bool]`
- `transitions: NotRequired[int]`: Bit 0 to-offnormal, bit 1 to-fault, bit 2 to-normal. Default 0x07.

## AuditPropertyReference

`class AuditPropertyReference(TypedDict)`

Keys:

- `property_identifier: PropertyIdentifier`
- `property_array_index: NotRequired[int | None]`

## AuditNotificationInput

`class AuditNotificationInput(TypedDict)`

Keys:

- `source_device: AuditRecipientInput`
- `operation: AuditOperation`
- `target_device: AuditRecipientInput`
- `source_timestamp: NotRequired[BACnetTimeStamp | None]`
- `target_timestamp: NotRequired[BACnetTimeStamp | None]`
- `source_object: NotRequired[ObjectIdentifier | None]`
- `source_comment: NotRequired[str | None]`
- `target_comment: NotRequired[str | None]`
- `invoke_id: NotRequired[int | None]`
- `source_user_id: NotRequired[int | None]`
- `source_user_role: NotRequired[int | None]`
- `target_object: NotRequired[ObjectIdentifier | None]`
- `target_property: NotRequired[AuditPropertyReference | None]`
- `target_priority: NotRequired[int | None]`
- `target_value: NotRequired[bytes | None]`
- `current_value: NotRequired[bytes | None]`
- `result: NotRequired[tuple[ErrorClass, ErrorCode] | None]`

## AuditNotificationRequestInput

`class AuditNotificationRequestInput(TypedDict)`

Keys:

- `notifications: list[AuditNotificationInput]`

## AuditLogQueryByTargetInput

`class AuditLogQueryByTargetInput(TypedDict)`

Keys:

- `kind: Literal["by_target"]`
- `target_device_identifier: ObjectIdentifier`
- `successful_actions_only: Literal[0, 1, 2]`: Corrected BACnetSuccessFilter (RB-02/RB-20): 0 = all, 1 = successes-only, 2 = failures-only. The pre-RB-02 Boolean is rejected with TypeError.
- `target_device_address: NotRequired[AuditRecipientAddress | None]`
- `target_object_identifier: NotRequired[ObjectIdentifier | None]`
- `target_property_identifier: NotRequired[PropertyIdentifier | None]`
- `target_array_index: NotRequired[int | None]`
- `target_priority: NotRequired[int | None]`
- `operations: NotRequired[int | None]`

## AuditLogQueryBySourceInput

`class AuditLogQueryBySourceInput(TypedDict)`

Keys:

- `kind: Literal["by_source"]`
- `source_device_identifier: ObjectIdentifier`
- `successful_actions_only: Literal[0, 1, 2]`: Corrected BACnetSuccessFilter (RB-02/RB-20): 0 = all, 1 = successes-only, 2 = failures-only. The pre-RB-02 Boolean is rejected with TypeError.
- `source_device_address: NotRequired[AuditRecipientAddress | None]`
- `source_object_identifier: NotRequired[ObjectIdentifier | None]`
- `operations: NotRequired[int | None]`

## AuditLogQueryRequestInput

`class AuditLogQueryRequestInput(TypedDict)`

Keys:

- `audit_log: ObjectIdentifier`
- `query_parameters: AuditLogQueryParametersInput`
- `requested_count: int`: Unsigned16 requested count (0..=65535).
- `start_at_sequence_number: NotRequired[int | None]`: Optional corrected Unsigned64 cursor (0..=2\*\*64-1; RB-20).

## AuditPropertyReferenceResult

`class AuditPropertyReferenceResult(TypedDict)`

Keys:

- `property_identifier: PropertyIdentifier`
- `property_array_index: int | None`

## AuditNotification

`class AuditNotification(TypedDict)`

Canonical return mapping for a decoded Audit notification.

Keys:

- `source_timestamp: BACnetTimeStamp | None`
- `target_timestamp: BACnetTimeStamp | None`
- `source_device: AuditRecipientInput`
- `source_object: ObjectIdentifier | None`
- `operation: AuditOperation`
- `source_comment: str | None`
- `target_comment: str | None`
- `invoke_id: int | None`
- `source_user_id: int | None`
- `source_user_role: int | None`
- `target_device: AuditRecipientInput`
- `target_object: ObjectIdentifier | None`
- `target_property: AuditPropertyReferenceResult | None`
- `target_priority: int | None`
- `target_value: bytes | None`
- `current_value: bytes | None`
- `result: tuple[ErrorClass, ErrorCode] | None`

## AtomicReadFileStreamResult

`class AtomicReadFileStreamResult(TypedDict)`

What `atomic_read_file` returns for stream access.

Keys:

- `access_method: Literal["stream"]`
- `end_of_file: bool`
- `start_position: int`
- `data: bytes`

## AtomicReadFileRecordResult

`class AtomicReadFileRecordResult(TypedDict)`

What `atomic_read_file` returns for record access.

Keys:

- `access_method: Literal["record"]`
- `end_of_file: bool`
- `start_record: int`
- `record_count: int`
- `records: list[bytes]`

## AtomicWriteFileStreamResult

`class AtomicWriteFileStreamResult(TypedDict)`

What `atomic_write_file` returns for stream access.

Keys:

- `access_method: Literal["stream"]`
- `start_position: int`

## AtomicWriteFileRecordResult

`class AtomicWriteFileRecordResult(TypedDict)`

What `atomic_write_file` returns for record access.

Keys:

- `access_method: Literal["record"]`
- `start_record: int`

## AuditLogStatusDatum

`class AuditLogStatusDatum(TypedDict)`

Keys:

- `kind: Literal["log_status"]`
- `log_status: BitString`: BACnetLogStatus, named LOG\_DISABLED, BUFFER\_PURGED, LOG\_INTERRUPTED.

## AuditNotificationDatum

`class AuditNotificationDatum(TypedDict)`

Keys:

- `kind: Literal["audit_notification"]`
- `audit_notification: AuditNotification`

## AuditTimeChangeDatum

`class AuditTimeChangeDatum(TypedDict)`

Keys:

- `kind: Literal["time_change"]`
- `time_change: float`

## AuditLogRecord

`class AuditLogRecord(TypedDict)`

Keys:

- `timestamp: BACnetDateTime`
- `datum: AuditLogDatum`

## AuditLogRecordResult

`class AuditLogRecordResult(TypedDict)`

Keys:

- `sequence_number: int`
- `record: AuditLogRecord`

## AuditLogQueryAck

`class AuditLogQueryAck(TypedDict)`

Keys:

- `audit_log: ObjectIdentifier`
- `records: list[AuditLogRecordResult]`
- `no_more_items: bool`

## ForeignRegistrationStatus

`class ForeignRegistrationStatus(TypedDict)`

A foreign device's registration, as `foreign_registration()`
reports it. `registered` is whether a lease is live; the lease starts
at the earliest send a success could answer, so it never outlasts the
BBMD's. `lease_remaining` and `next_attempt_in` are seconds, or None.
`last_bbmd`, `last_result` (the BVLC result code, 0x0030 for a
refusal) and `last_outcome` (`"pending"`, `"accepted"`,
`"refused"`, `"timed_out"`, `"local_error"` or `"cancelled"`)
describe the latest attempt and clear when the owner stops. Those fields
are separate from the live lease: `registered` does not imply the
latest attempt was accepted, and can remain true during a pending,
refused or timed-out renewal. The totals
survive it: `lapses` counts leases that ran out with no newer success,
and `renewal_hints` the BBMD's Distribute-Broadcast-To-Network NAKs,
each of which brings the next attempt forward.

Keys:

- `registered: bool`
- `lease_remaining: Optional[float]`
- `next_attempt_in: Optional[float]`
- `last_bbmd: Optional[str]`
- `last_result: Optional[int]`
- `last_outcome: Optional[str]`
- `attempts: int`
- `accepted: int`
- `refused: int`
- `lapses: int`
- `renewal_hints: int`

## FdtEntry

`class FdtEntry(TypedDict)`

A Foreign Device Table entry, as `read_fdt()` returns it.
`seconds_remaining` is what the BBMD reports, its grace period
included.

Keys:

- `address: str`
- `ttl: int`
- `seconds_remaining: int`

## AuthenticationPolicy

```python
AuthenticationPolicy = tuple[
    list[tuple[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier], int]],
    bool,
    int,
]
```

One Access Point Authentication\_Policy\_List element (`BACnetAuthenticationPolicy`): the `(credential_data_input, index)` entries, each Credential Data Input an `ObjectIdentifier` in this device or a `(device, object)` pair, then whether the order is enforced and the timeout in seconds (0 for none). A read of the list gives each element in this form.

## BACnetDate

```python
BACnetDate = tuple[int, int, int, int]
```

A date `(year, month, day, day_of_week)` with the full year, and a time `(hour, minute, second, hundredths)`; 255 marks an unspecified field.

## BACnetTime

```python
BACnetTime = tuple[int, int, int, int]
```

## TimeValue

```python
TimeValue = tuple[BACnetTime, PropertyValue]
```

One Weekly\_Schedule or exception entry: the time and the value from then.

## CalendarEntry

```python
CalendarEntry = CalendarDate | CalendarDateRange | CalendarWeekNDay
```

One Date\_List entry, or an exception's own period (`BACnetCalendarEntry`).

## AuditRecipientInput

```python
AuditRecipientInput = AuditRecipientDevice | AuditRecipientAddress
```

A read of the Device's Audit\_Notification\_Recipient gives this form.

## AuditLogQueryParametersInput

```python
AuditLogQueryParametersInput = AuditLogQueryByTargetInput | AuditLogQueryBySourceInput
```

## BACnetDateTime

```python
BACnetDateTime = tuple[
    tuple[int, int, int, int],
    tuple[int, int, int, int],
]
```

## AtomicReadFileResult

```python
AtomicReadFileResult = Union[AtomicReadFileStreamResult, AtomicReadFileRecordResult]
```

## AtomicWriteFileResult

```python
AtomicWriteFileResult = Union[AtomicWriteFileStreamResult, AtomicWriteFileRecordResult]
```

## AuditLogDatum

```python
AuditLogDatum = AuditLogStatusDatum | AuditNotificationDatum | AuditTimeChangeDatum
```
<!-- generated:end -->
