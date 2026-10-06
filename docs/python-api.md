# Python API Reference

`rusty_bacnet` provides Python bindings for the Rust BACnet protocol stack via PyO3. All I/O operations are async (`asyncio`-based).

This reference follows the `dev` branch. At the `v0.12.0` tag it describes the
[published 0.12.0 package](https://pypi.org/project/rusty-bacnet/0.12.0/), which
the site's tutorials also target; see the [installation guidance](../README.md#install).
Changes merged after the release wait in [`changelog.d/`](../changelog.d/); to
use them, follow [Build from source](../README.md#build-from-source).

**Requirements:** Python >= 3.11

Native asynchronous methods return an `asyncio.Future` immediately when called
inside a running event loop. Their installed signatures use ordinary `def` with
`Awaitable[T]`, where `T` is the value produced by `await`. Argument validation
that occurs before Future creation still raises synchronously; I/O and operation
failures are delivered by awaiting the Future. Successful `Awaitable[None]`
operations resolve to Python `None`, including client commands, server local
writes and lifecycle, hub lifecycle, and endpoint broadcasts. Meaningful data,
tuple, string and context-entry results retain their declared types.

Use `await native_call()` or `asyncio.ensure_future(native_call())`.
`asyncio.create_task` requires a coroutine object and does not accept these native
Futures directly. It remains appropriate for user-written `async def` functions,
including listener tasks in the examples below. Await or
cancel and drain every Future you create; cancelling does not retract work
already sent or replace a resource owner's documented `stop`/`close` lifecycle.

Native async context methods remain usable with `async with`. For COV,
`iterator = await client.cov_notifications()` first resolves the Future to a
`CovNotificationIterator`; then use `async for notification in iterator`.
Its synchronous `__aiter__` returns the iterator, and `__anext__` returns a Future
that yields a notification or raises `StopAsyncIteration` when the channel closes.

## Installation

Install the published 0.12.0 package with:

```bash
pip install "rusty-bacnet==0.12.0"
```

Wheels cover CPython 3.11 to 3.14 on Linux (glibc 2.17 or newer; x86_64,
aarch64), macOS (x86_64 on 10.12 or later, arm64 on 11.0 or later) and Windows
(x64), each with BACnet/IPv6, BACnet/SC and MS/TP.

The package includes a `.pyi` type stub file for IDE autocompletion and type checking. Most editors (VS Code, PyCharm) will pick it up automatically from the installed package.

---

## Quick Start

```python
import asyncio
from rusty_bacnet import (
    BACnetClient, BACnetServer,
    ObjectType, ObjectIdentifier, PropertyIdentifier, PropertyValue,
)

async def main():
    oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

    # Read a property
    async with BACnetClient() as client:
        value = await client.read_property(
            "192.168.1.100:47808", oid, PropertyIdentifier.PRESENT_VALUE
        )
        print(f"{value.tag}: {value.value}")  # "real: 72.5"

        # Write a property
        await client.write_property(
            "192.168.1.100:47808", oid, PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(75.0), priority=8
        )

asyncio.run(main())
```

---

## Enums

All enums have class-level named constants, plus `from_raw(int)` and `to_raw()` for raw access. They support `==`, `hash()`, and `repr()`, and `copy.copy`, `copy.deepcopy` and `pickle`, which rebuild a value with `from_raw(value.to_raw())` (#1456).

### ObjectType

BACnet object types (u32). Constants include `ANALOG_INPUT`, `ANALOG_OUTPUT`, `ANALOG_VALUE`, `BINARY_INPUT`, `BINARY_OUTPUT`, `BINARY_VALUE`, `CALENDAR`, `DEVICE`, `LOOP`, `MULTI_STATE_INPUT`, `MULTI_STATE_OUTPUT`, `MULTI_STATE_VALUE`, `NOTIFICATION_CLASS`, `SCHEDULE`, `TREND_LOG`, `FILE`, `AUDIT_LOG`, `AUDIT_REPORTER`, `COMMAND`, `TIMER`, `LOAD_CONTROL`, `PROGRAM`, `LIGHTING_OUTPUT`, `BINARY_LIGHTING_OUTPUT`, `LIFE_SAFETY_POINT`, `LIFE_SAFETY_ZONE`, `GROUP`, `GLOBAL_GROUP`, `STRUCTURED_VIEW`, `NOTIFICATION_FORWARDER`, `ALERT_ENROLLMENT`, `ACCESS_DOOR`, `ACCESS_CREDENTIAL`, `ACCESS_POINT`, `ACCESS_RIGHTS`, `ACCESS_USER`, `ACCESS_ZONE`, `CREDENTIAL_DATA_INPUT`, `ELEVATOR_GROUP`, `ESCALATOR`, `LIFT`, `STAGING`, `ACCUMULATOR`, `PULSE_CONVERTER`, `EVENT_ENROLLMENT`, `CHANNEL`, `EVENT_LOG`, `TREND_LOG_MULTIPLE`, `NETWORK_PORT`, `INTEGER_VALUE`, `POSITIVE_INTEGER_VALUE`, `LARGE_ANALOG_VALUE`, `CHARACTER_STRING_VALUE`, `OCTET_STRING_VALUE`, `BIT_STRING_VALUE`, `DATE_VALUE`, `TIME_VALUE`, `DATE_TIME_VALUE`, `DATE_PATTERN_VALUE`, `TIME_PATTERN_VALUE`, `DATE_TIME_PATTERN_VALUE`, `AVERAGING`, etc.

```python
ot = ObjectType.ANALOG_INPUT
ot = ObjectType.from_raw(0)
raw = ot.to_raw()  # 0
```

### PropertyIdentifier

BACnet property identifiers (u32). Constants include `PRESENT_VALUE`, `OBJECT_NAME`, `OBJECT_TYPE`, `OBJECT_LIST`, `STATUS_FLAGS`, `EVENT_STATE`, `UNITS`, `PRIORITY_ARRAY`, `RELINQUISH_DEFAULT`, `COV_INCREMENT`, `LOG_BUFFER`, etc.

```python
pid = PropertyIdentifier.PRESENT_VALUE
```

### ErrorClass / ErrorCode

Error classification from BACnet error responses.

```python
ec = ErrorClass.PROPERTY
ev = ErrorCode.UNKNOWN_PROPERTY
```

### EnableDisable

For `device_communication_control`, and what `BACnetServer.comm_state()` returns. Constants: `ENABLE`, `DISABLE`, `DISABLE_INITIATION`. `DISABLE` is deprecated, and a rusty-bacnet server refuses it under every DCC policy.

```python
ed = EnableDisable.DISABLE_INITIATION
```

### ReinitializedState

For `reinitialize_device`. Constants: `COLDSTART`, `WARMSTART`, `START_BACKUP`, `END_BACKUP`, `START_RESTORE`, `END_RESTORE`, `ABORT_RESTORE`, `ACTIVATE_CHANGES`.

```python
state = ReinitializedState.WARMSTART
```

### Segmentation

Segmentation support levels. Constants: `BOTH`, `TRANSMIT`, `RECEIVE`, `NONE`.

### LifeSafetyOperation

For `life_safety_operation`. Constants: `NONE`, `SILENCE`, `SILENCE_AUDIBLE`, `SILENCE_VISUAL`, `RESET`, `RESET_ALARM`, `RESET_FAULT`, `UNSILENCE`, `UNSILENCE_AUDIBLE`, `UNSILENCE_VISUAL`.

```python
op = LifeSafetyOperation.SILENCE
```

### EventState

BACnet event states. Constants: `NORMAL`, `FAULT`, `OFFNORMAL`, `HIGH_LIMIT`, `LOW_LIMIT`, `LIFE_SAFETY_ALARM`.

```python
es = EventState.NORMAL
```

### EventType

BACnet event types. Constants: `CHANGE_OF_BITSTRING`, `CHANGE_OF_STATE`, `CHANGE_OF_VALUE`, `COMMAND_FAILURE`, `FLOATING_LIMIT`, `OUT_OF_RANGE`, `COMPLEX_EVENT_TYPE`, etc.

```python
et = EventType.CHANGE_OF_VALUE
```

### MessagePriority

For text message services. Constants: `NORMAL`, `URGENT`.

```python
mp = MessagePriority.URGENT
```

### VTClass

Terminal class for `vt_open`. Constants: `DEFAULT_TERMINAL`, `ANSI_X3_64`, `DEC_VT52`, `DEC_VT100`, `DEC_VT220`, `HP_700_94`, `IBM_3130`.

```python
vc = VTClass.DEFAULT_TERMINAL
```

---

## ObjectIdentifier

Immutable BACnet object identifier (type + instance).

```python
oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
oid.object_type   # ObjectType.ANALOG_INPUT
oid.instance       # 1
```

It copies and pickles like the other value classes; see
[Copying and pickling](#copying-and-pickling).

---

## PropertyValue

Immutable typed BACnet application-layer value. Create with static constructors, read with `.tag` and `.value`.

### Constructors

```python
PropertyValue.null()
PropertyValue.boolean(True)
PropertyValue.unsigned(42)
PropertyValue.signed(-10)
PropertyValue.real(72.5)            # 32-bit float
PropertyValue.double(72.5)          # 64-bit float
PropertyValue.character_string("hello")
PropertyValue.octet_string(b"\x01\x02")
PropertyValue.enumerated(1)
PropertyValue.object_identifier(oid)
PropertyValue.date(2026, 3, 21, 6)    # full year, month, day, day_of_week (1=Mon)
PropertyValue.time(14, 30, 0, 0)      # hour, minute, second, hundredths
PropertyValue.bit_string(0, b"\xff")  # unused_bits, data
PropertyValue.list([PropertyValue.unsigned(1), PropertyValue.unsigned(2)])
```

Lists nest at most 32 deep, the decoder's nesting limit: a deeper
`PropertyValue.list`, by hand or from a pickle, raises `ValueError` (#1506).
Two values are equal when they have the same tag and value, numbers compared
as numbers (`PropertyValue.real(0.0) == PropertyValue.real(-0.0)`, and a NaN
equals nothing), and equal values hash alike.

### Accessors

```python
v = PropertyValue.real(72.5)
v.tag     # "real"
v.value   # 72.5 (native Python float)
```

| Tag | Python `.value` type |
|-----|---------------------|
| `"null"` | `None` |
| `"boolean"` | `bool` |
| `"unsigned"` | `int` |
| `"signed"` | `int` |
| `"real"` | `float` |
| `"double"` | `float` |
| `"character_string"` | `str` |
| `"octet_string"` | `bytes` |
| `"enumerated"` | `int` |
| `"object_identifier"` | `ObjectIdentifier` |
| `"bit_string"` | `dict` with `"unused_bits"` and `"data"` |
| `"date"` | `tuple(year, month, day, day_of_week)`, the full year as [Dates](#dates) gives it |
| `"time"` | `tuple(hour, minute, second, hundredths)` |
| `"list"` | `list` of native Python values |
| `"application_data"` | `bytes`: the encoded value, octet for octet |
| `"destination"`, `"port_permission"` and the other element tags of [typed constructed values](#typed-constructed-values) | the element in its typed form |

### Dates

Every date the binding reads or takes is a `(year, month, day, day_of_week)`
tuple with the full year, 1900 to 2154, and 255 for an unspecified year, the
same 255 as any other unspecified date or time field (#1501). The module
exports it as `rusty_bacnet.UNSPECIFIED`. That holds for a `"date"` value, a
`BACnetTimeStamp` date-time, the dates in schedules, calendars and date
ranges, and the audit log's records. A year outside 1900 to 2154 that isn't
255 (the year octet 126, say) raises `ValueError`. The time synchronization
requests take only a specific date and time; see
[Time Synchronization](#time-synchronization).

```python
from rusty_bacnet import UNSPECIFIED
v = PropertyValue.date(2026, 3, 21, 6)
v.value                                       # (2026, 3, 21, 6)
PropertyValue.date(UNSPECIFIED, 12, 25, UNSPECIFIED).value  # (255, 12, 25, 255): every Christmas
BACnetTimeStamp.date_time((2026, 3, 21, 6), (8, 0, 0, 0)).value[0]  # (2026, 3, 21, 6)
```

A `datetime.date(255, 12, 25)` is Christmas in the year 255 AD, not a
wildcard: compare a date's year with `UNSPECIFIED` before building a
`datetime.date` from it.

### Integer arguments

An integer argument is read as the fixed-width type of the field it fills
(unsigned8, unsigned16, unsigned32, unsigned64 or INTEGER). One outside
that type, negative for an unsigned field or too wide, raises
`OverflowError`, whether it is a parameter, a tuple member or a value in a
mapping (#1360). A value that fits the type but that BACnet doesn't allow
in the field raises `ValueError` when the binding checks it while reading
the argument (an instance past 4194303, a month of 15, a `valid_days` of
128), and `BacnetProtocolError` with `VALUE_OUT_OF_RANGE` when the object's
own check refuses it (a priority of 17, units past 65535). So
`add_access_point(..., priority_for_writing=256)` raises `OverflowError`,
and `priority_for_writing=17` raises `BacnetProtocolError`.

### Read results

Every read that returns a `PropertyValue` decodes the value's octets by one
rule: `BACnetClient.read_property` and `read_property_multiple` (and their
`_from_device` and batch forms), `EndpointClient.read_property` and
`read_property_multiple`, `CovNotification.values`, and
`BACnetServer.read_property`. Every element of the value is kept, and only
broken framing (a length past the end, an unmatched opening or closing tag)
is an error.

- **A typed constructed value**, one of the constructed properties in
  [the table below](#typed-constructed-values), reads in its typed form
  (the form its typed write takes, where the binding has one): a whole read
  of a list or array is a `list` of the elements, an indexed read one
  element, and a read of a single value that element, each tagged with its
  production. A value that doesn't decode as those elements, to the last
  octet, falls through to the rules below.
- **Other context-tagged content** comes back as `application_data` whose
  `.value` holds the octets as served. Constructed values with no typed form,
  such as a Load Control's Requested_Shed_Level or an Event Enrollment's
  Event_Parameters, take this form, as local reads of them always have. Element
  boundaries of a constructed list aren't marked in the octets, so the
  binding doesn't split one: decode the octets with the datatype's layout,
  or read single array elements with `array_index`. Writing the value back
  sends the same octets.
- **Content `PropertyValue` can't hold**, such as a CharacterString in UCS-4,
  DBCS or JIS X 0208, UTF-8 that doesn't decode, an ENUMERATED wider than 32
  bits or a reserved application tag, makes the whole value `application_data`
  in the same way.
- **A whole array or list** (no `array_index`) of application-tagged elements
  is a `list` at every length, zero and one included. Whether a property is
  an array or a list on an object type comes from the stack's classification
  table, which holds every property the 2020 Clause 12 object tables type as
  a BACnetARRAY or BACnetLIST; a vendor-defined collection is shaped by the
  last rule. A whole Object_List, Priority_Array, one-state State_Text or a
  B/IP Network Port's IP_DNS_SERVER is a `list`.
- **Anything else** is the bare value when it holds one element, and a
  `list` in wire order when it holds none or several. An empty value is
  `PropertyValue.list([])`, whatever the property's element type. A
  BACnetDateTime such as a Load Control's Start_Time is a two-element list,
  the date then the time, and a list whose elements are several
  application-tagged fields comes back flat, in wire order.

`array_index=0` reads an array's size. Any other index reads one element,
shaped by the same rules: a single application value is bare
(`Object_List[2]`), an element of a typed collection is that element
(`Port_Filter[2]` is a `port_permission`), an element of several application
fields is a `list`, an element of a typed array is that element
(`Event_Time_Stamps[1]` is a `timestamp`), and any other context-tagged
element is `application_data`.

```python
objects = await client.read_property(address, device, PropertyIdentifier.OBJECT_LIST)
objects.tag    # "list", even for a device with one object
recipients = await client.read_property(address, nc, PropertyIdentifier.RECIPIENT_LIST)
recipients.tag    # "list"
recipients.value  # [{"recipient": {"kind": "device", ...}, "process_identifier": 1, ...}, ...]
port = await client.read_property(address, nf, PropertyIdentifier.PORT_FILTER, 2)
port.tag, port.value  # ("port_permission", (1, False))
```

#### Typed constructed values

These properties hold constructed values, or lists or arrays of them, that
read back typed (#1310, #1344, #1345). Where the binding also takes the
property as a typed value, the read comes back in that form, so a script can
usually hand it back to the write. The exceptions are a Target_References
`(device, object)` pair, which `add_staging` refuses because it takes local
targets only, and a value the object's own checks refuse, which raises as it
would if written by hand. Where it has no typed write, the form is the one
the table gives. A whole read of a list or array is a list of the elements,
and a read of a single value (Accompaniment, Audit_Notification_Recipient,
Effective_Period, Last_Command_Time, Value_Source, Scale, Prescale) the
element itself. An
empty collection is `PropertyValue.list([])`. Each element keeps the octets it
was read from, so writing the value back (`write_property`,
`write_property_local`) sends them unchanged. `PropertyValue.list` of
elements of one collection, from indexed reads, equals the whole read. Two
values are equal when they carry the same octets and the same element
production, so a typed read doesn't equal `PropertyValue.application_data` of
its octets.

| Object type | Property | Element tag | Element `.value` | Typed write |
|-------------|----------|-------------|------------------|-------------|
| Notification Class, Notification Forwarder | Recipient_List | `"destination"` | a `Destination` mapping with every key | `add_notification_forwarder(recipients=...)` |
| Notification Forwarder | Port_Filter | `"port_permission"` | `(port_id, enabled)` | `add_notification_forwarder(port_filter=...)` |
| Group | List_Of_Group_Members | `"read_access_specification"` | `(object_id, [(property_id, array_index), ...])` | `add_group(members=...)` |
| Group | Present_Value | `"read_access_result"` | a `read_property_multiple` result: `{"object_id": ..., "results": [...]}` | none: the members' results |
| Command | Action | `"action_list"` | a list of `ActionCommand` mappings with every key | `add_command(action=...)`, which ignores `write_successful` |
| Access Door, Access Point, Staging | Door_Members, Access_Doors, Target_References | `"device_object_reference"` | an `ObjectIdentifier`, or `(device, object)` when the reference names a device | `door_members=`, `access_doors=`; `target_references=` takes the `ObjectIdentifier` form only |
| Access Point | Authentication_Policy_List | `"authentication_policy"` | `([(credential_data_input, index), ...], order_enforced, timeout)`, each reference an `ObjectIdentifier` or `(device, object)` | `add_access_point(authentication_policies=...)`, paired with the Authentication_Policy_Names element |
| Credential Data Input | Supported_Formats | `"authentication_factor_format"` | the format type, or `(format_type, vendor_id, vendor_format)` when it has vendor members (a missing one is `None`, which the write also takes) | `add_credential_data_input(supported_formats=...)`, paired with Supported_Format_Classes |
| Staging | Stages | `"stage_limit_value"` | `(limit, values, deadband)`, `values` a `list[bool]` | `add_staging(stages=...)` |
| Access Rights | Positive_Access_Rules, Negative_Access_Rules | `"access_rule"` | an `AccessRule` mapping with every key; `None` stands for ALWAYS and ALL | `add_access_rights(positive_access_rules=..., negative_access_rules=...)` |
| Access Rights | Accompaniment (one value) | `"device_object_reference"` | an `ObjectIdentifier`, or `(device, object)` | `add_access_rights(accompaniment=...)` |
| Access Zone, Access User | Entry_Points, Exit_Points; Credentials, Members, Member_Of | `"device_object_reference"` | an `ObjectIdentifier`, or `(device, object)` | `add_access_zone(entry_points=..., exit_points=...)`, `add_access_user(credentials=..., members=..., member_of=...)` |
| Life Safety Point, Life Safety Zone | Member_Of; Zone_Members | `"device_object_reference"` | an `ObjectIdentifier`, or `(device, object)` | none |
| Global Group, Schedule, Channel, Trend Log Multiple | Group_Members; List_Of_Object_Property_References; Log_DeviceObjectProperty | `"device_object_property_reference"` | a `DeviceObjectPropertyReference` mapping with every key | `add_channel(members=...)`, `add_trend_log_multiple(members=...)` |
| Global Group | Present_Value | `"property_access_result"` | the member's `DeviceObjectPropertyReference` keys, then `"value"` (shaped as a read of the member) and `"error"` (`(ErrorClass, ErrorCode)`), one of them `None` | none |
| Device | Audit_Notification_Recipient (one value) | `"recipient"` | an `AuditRecipientInput` mapping | `configure_audit_recipient(...)` |
| Device | Active_COV_Subscriptions | `"cov_subscription"` | `{"recipient", "process_identifier", "object_identifier", "property_identifier", "property_array_index", "issue_confirmed_notifications", "time_remaining", "cov_increment"}`; `recipient` an `AuditRecipientInput` mapping, `cov_increment` a `float` or `None` | none |
| Device | Device_Address_Binding | `"address_binding"` | `{"device_identifier", "network_number", "mac_address"}`; `network_number` 0 for a device on this network, `mac_address` its own MAC, not a router's | none |
| Schedule | Weekly_Schedule | `"daily_schedule"` | one day: `[(time, value), ...]`, `time` an `(hour, minute, second, hundredths)` tuple and `value` a `PropertyValue` | none |
| Schedule | Exception_Schedule | `"special_event"` | `{"period", "time_values", "priority"}`; `period` a calendar entry mapping or a Calendar's `ObjectIdentifier`, `time_values` as for a day | none |
| Schedule | Effective_Period (one value) | `"date_range"` | `(start_date, end_date)` | none |
| Calendar | Date_List | `"calendar_entry"` | `{"kind": "date", "date"}`, `{"kind": "date_range", "start_date", "end_date"}` or `{"kind": "week_n_day", "month", "week_of_month", "day_of_week"}` | none |
| any | Event_Time_Stamps, Command_Time_Array; Last_Command_Time (one value) | `"timestamp"` | a `BACnetTimeStamp` | none |
| any | Value_Source_Array; Value_Source (one value) | `"value_source"` | `None` (none), an `ObjectIdentifier` or `(device, object)` (an object), or an `AuditRecipientAddress` mapping, `{"kind": "address", "network_number", "mac_address"}` (an address) | none |
| Accumulator | Scale (one value) | `"scale"` | a `float` for a float scale, an `int` for a power-of-ten scale | `add_accumulator(scale=...)` |
| Accumulator | Prescale (one value) | `"prescale"` | `(multiplier, modulo_divide)` | `add_accumulator(prescale=...)` |

A date in these forms is a `(year, month, day, day_of_week)` tuple as
[Dates](#dates) gives it: the full year, and 255 in any field left
unspecified. An Access Rights rule whose specifiers disagree with the
references it carries, which no typed write makes, has no `AccessRule` form,
so its array reads as `application_data`.

A Group's Present_Value results, and an `ActionCommand`'s `property_value`,
are themselves read results: each value is shaped as a read of the property it
names would be.

---

## Copying and pickling

`ObjectIdentifier`, `PropertyValue` and `BACnetTimeStamp`, like the
[enums](#enums), support `copy.copy`, `copy.deepcopy` and `pickle` at every
protocol, and the copy equals the original (#1500). Every class the module
exports reports `rusty_bacnet` as its `__module__`.

- An `ObjectIdentifier` rebuilds through its constructor.
- A `PropertyValue` rebuilds through the constructor its `tag` names, with
  the value as stored: `PropertyValue.date(2026, 3, 21, 6)` pickles as that
  call. A list rebuilds from its items as `PropertyValue`s, so a `real` item
  stays a `real` where `.value` would give a plain `float`. A typed
  constructed read rebuilds from the octets it was read from, with its
  element tag, so it still writes back exactly what was read.
- A `BACnetTimeStamp` rebuilds from its encoded CHOICE, so a timestamp a peer
  sent with a field outside the ranges `date_time` checks (a month of 0, say)
  copies too.

```python
import pickle
value = await client.read_property(address, schedule, PropertyIdentifier.EXCEPTION_SCHEDULE)
assert pickle.loads(pickle.dumps(value)) == value
```

A pickle is for the same rusty-bacnet version that made it: it names the
classes' private rebuild methods, which may change before 1.0. Don't keep
pickles across upgrades.

The other classes (`DiscoveredDevice`, `CovNotification`,
`ScHubCertificateBinding`, and the client, server, endpoint and hub classes)
raise `TypeError` when pickled; copy the values you need out of them.

---

## DiscoveredDevice

Read-only device information from WhoIs/IAm discovery (frozen).

| Property | Type | Description |
|----------|------|-------------|
| `.object_identifier` | `ObjectIdentifier` | Device object ID |
| `.mac_address` | `list[int]` | Raw MAC bytes |
| `.max_apdu_length` | `int` | Max APDU the device accepts |
| `.segmentation_supported` | `Segmentation` | Segmentation capability |
| `.vendor_id` | `int` | Vendor identifier |
| `.seconds_since_seen` | `float` | Seconds since last IAm |
| `.source_network` | `int \| None` | Remote network number (if routed) |
| `.source_address` | `bytes \| None` | Remote MAC address (if routed) |

---

## CovNotification

Read-only incoming COV notification (frozen).

| Property | Type | Description |
|----------|------|-------------|
| `.subscriber_process_identifier` | `int` | Subscriber process ID |
| `.initiating_device_identifier` | `ObjectIdentifier` | Source device |
| `.monitored_object_identifier` | `ObjectIdentifier` | Changed object |
| `.time_remaining` | `int` | Subscription seconds remaining |
| `.values` | `list[dict]` | Changed properties (see below) |

Each item in `.values` is a dict:
```python
{
    "property_id": PropertyIdentifier,
    "array_index": int | None,
    "value": PropertyValue | bytes,
}
```

`value` follows the [read result](#read-results) rule, using the monitored
object's type; octets whose framing is broken come back as `bytes`.

---

## CovNotificationIterator

Async iterator yielding `CovNotification` objects from the client's broadcast channel.

```python
notifications = await client.cov_notifications()
async for notification in notifications:
    print(notification.monitored_object_identifier)
    for v in notification.values:
        print(f"  {v['property_id']}: {v['value']}")
```

Automatically retries on lagged messages. Raises `StopAsyncIteration` when the client is stopped.

---

## BACnetClient

Async BACnet client for communicating with remote devices.

### Constructor

```python
client = BACnetClient(
    interface="0.0.0.0",        # Bind address
    port=47808,                  # UDP port
    broadcast_address="255.255.255.255",
    apdu_timeout_ms=6000,
    transport="bip",             # "bip", "ipv6", or "sc"
    share_port_by_address=False, # Keyword-only; B/IP, see "Sharing a port by address"
    # IPv6 options:
    ipv6_interface=None,         # IPv6 selected local address; None means ::
    # SC options:
    sc_hub=None,                 # WebSocket hub URL
    sc_vmac=None,                # 6-byte VMAC
    sc_device_uuid=None,         # Keyword-only; persistent 16-byte nonzero UUID required for SC
    sc_ca_cert=None,             # Site CA PEM path (required for SC)
    sc_client_cert=None,         # Operational certificate PEM path (required for SC)
    sc_client_key=None,          # Matching private key PEM path (required for SC)
    sc_heartbeat_interval_ms=None,  # 3000..=300000 ms when configured
    sc_heartbeat_timeout_ms=None,   # must be greater than interval
    min_request_interval_ms=0,   # Keyword-only; least ms between confirmed requests to one destination
)
```

`min_request_interval_ms` paces the confirmed requests the client sends to
each destination (#1535). Each waits until that long after the latest one
sent to the same destination finished, by a reply, an error or the caller
giving up, or until that long after it was sent while it is still
outstanding, checking again when it wakes. Requests waiting together go one
at a time, in no promised order. Paging a log or polling then leaves a slow
device room for its
other clients. Requests to different destinations don't wait on each other;
0, the default, sends at once, and more than 3,600,000 (an hour) raises
`ValueError`. Behind a router the pause after a reply holds for requests made
one after another only. `EndpointClient` has no pacing.

### Sharing a port by address

By default a B/IP client, server or `BipEndpoint` binds `0.0.0.0` on its port,
whatever `interface` says, so two devices on two addresses of one host can't
both use 47808. With the keyword-only `share_port_by_address=True`, each binds
its own `interface` address instead and gets only the unicast sent there, so
several devices on one host share the port (#1538). It needs an explicit
`interface` and a nonzero `port`; `start()` fails otherwise, and another
transport raises `ValueError`.

In this mode every send leaves from the interface address, and broadcasts
and unicast are received in **no fixed order**: on Linux and macOS
broadcasts arrive on separate receive-only sockets, so a unicast that depends
on a broadcast sent just before it can be handled first. Those sockets keep
only broadcasts that arrived on the interface's own link, as identified
when the device starts: restart it after its interface changes. The
`broadcast_address` must be the interface's subnet broadcast or
`255.255.255.255` (a loopback `interface` may also name itself), or
`start()` fails. On Windows the address is claimed
exclusively, though the bind still succeeds beside another program's socket
on `0.0.0.0` at that port. See
[Sharing a port by address](rust-api.md#sharing-a-port-by-address) for each
OS.

### MS/TP serial ports

`transport="mstp"` takes the keyword-only `serial_port` (required),
`mstp_baud` (9600, 19200, 38400, 57600, 76800 or 115200; default 38400),
`mstp_mac` (default 1), `mstp_max_master` (default 127) and
`mstp_max_info_frames` (default 1). `BACnetServer` takes the same options, and
`MstpEndpoint` its own `serial_port`.

`rusty_bacnet.list_serial_ports()` returns the names of the serial ports the
operating system reports, to pass as `serial_port`: macOS lists them through
IOKit, Windows through SetupAPI and the registry, and Linux from sysfs. A port
that another program has open is listed too, and an empty list means none. It
raises `OSError` (or the subclass for the failure's kind) if the operating
system can't be asked.

```python
import rusty_bacnet

print(rusty_bacnet.list_serial_ports())  # ['/dev/ttyUSB0'], ['/dev/cu.usbserial-1410'], ['COM3'], ...
```

### Lifecycle

```python
# Preferred: async context manager
async with BACnetClient() as client:
    ...  # client is started and ready

# Manual:
await client.stop()
```

### Address Format

All `address` parameters accept:
- IPv4: `"192.168.1.100:47808"` (4-byte IP + 2-byte port)
- IPv6: `"[::1]:47808"` (16-byte IP + 2-byte port)
- Hex MAC: `"01:02:03:04:05:06"` (raw bytes, for SC/Ethernet)

---

### Property Access

#### `read_property(address, object_id, property_id, array_index=None) -> PropertyValue`

Standalone and endpoint clients validate the ACK's object, property and array
index. Device or Network Port instance `4194303` requests accept a concrete
same-type identifier reported by the peer; unresolved wildcard and mismatched
ACKs raise `BacnetError`. The return value is the decoded property value,
with every element kept as [Read results](#read-results) describes.
The bundled server resolves the Device alias. Full-server RP/RPM and endpoint RP
also resolve the Network Port alias when that owner explicitly registers a port;
see [registered B/IP ports](#registered-bip-network-port).

```python
value = await client.read_property(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    PropertyIdentifier.PRESENT_VALUE,
)
print(value.value)  # 72.5
```

#### `write_property(address, object_id, property_id, value, priority=None, array_index=None)`

The built-in commandable objects expose `Priority_Array` as read-only (§19.2.1).
Set or relinquish a priority slot by writing a value or NULL to `Present_Value`
with the desired priority. Whole-array and indexed `Priority_Array` writes are
denied, including index 0 (the element count); indexed reads remain available.
A refused direct array write does not change the effective value or terminate
an active lighting operation. WPM preserves valid earlier writes when it reaches
such a denied element. This does not impose a write policy on custom objects.

For indexed WP/WPM received by the bundled server, a property absent from
nonempty effective object metadata returns `PROPERTY/UNKNOWN_PROPERTY` before
value decoding. Served scalars and BACnetLIST properties retain
`PROPERTY_IS_NOT_AN_ARRAY`; served arrays retain their object-specific write
rules. Absence-first is a local error-precedence policy. Custom objects with empty
metadata keep their existing classifier and writer delegation. WPM retains its
successful prefix and leaves the failing element and suffix unmodified. This
describes this library, not the error policy of a remote server.


All three single-property entrypoints (`write_property`, `write_property_to_device`,
and `write_property_to_devices`) accept omitted priority or 1–16. Values 0 or
17–255 raise synchronous `ValueError` before a future, device lookup, or traffic;
values outside the native u8 range raise `OverflowError`. A multi-device batch
validates every input before dispatch, so an invalid later priority cannot send a
valid prefix. Valid batches retain per-device outcomes and completion order.
Supplied valid priority is ignored by a noncommandable remote property; NULL
relinquishment semantics are unchanged.

```python
await client.write_property(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1),
    PropertyIdentifier.PRESENT_VALUE,
    PropertyValue.real(75.0),
    priority=8,
)
```

#### `read_property_multiple(address, specs) -> list[dict]`

Read multiple properties from multiple objects in one request.

```python
results = await client.read_property_multiple("192.168.1.100:47808", [
    (ObjectIdentifier(ObjectType.ANALOG_INPUT, 1), [
        (PropertyIdentifier.PRESENT_VALUE, None),
        (PropertyIdentifier.OBJECT_NAME, None),
    ]),
    (ObjectIdentifier(ObjectType.ANALOG_INPUT, 2), [
        (PropertyIdentifier.PRESENT_VALUE, None),
    ]),
])

for obj in results:
    print(f"Object: {obj['object_id']}")
    for prop in obj['results']:
        if prop['value'] is not None:
            print(f"  {prop['property_id']}: {prop['value'].value}")
        elif prop['error'] is not None:
            ec, ev = prop['error']
            print(f"  {prop['property_id']}: ERROR {ec} {ev}")
```

Return format: `list[dict]` where each dict has:
- `"object_id"`: `ObjectIdentifier`
- `"results"`: `list[dict]` with `"property_id"`, `"array_index"`, `"value"` (PropertyValue shaped as [Read results](#read-results) describes, raw `bytes` when the octets' framing is broken, or None on error), `"error"` (tuple of ErrorClass, ErrorCode or None)

#### `write_property_multiple(address, specs)`

Write multiple properties to multiple objects in one request. Both direct and
device-directed WPM validate the entire input synchronously before creating an
operation or starting discovery. Empty request/property lists, ALL/REQUIRED/OPTIONAL
targets, and priorities 0 or 17–255 raise `ValueError`; integers outside the native
u8 range raise `OverflowError`. Omitted priority, priorities 1–16, index zero,
proprietary properties, NULL and empty list **values** remain valid. This is local
request validation; remote property availability and commandability are not inferred.

```python
await client.write_property_multiple("192.168.1.100:47808", [
    (ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1), [
        (PropertyIdentifier.PRESENT_VALUE, PropertyValue.real(75.0), 8, None),
        # (property_id, value, priority, array_index)
    ]),
])
```

---

### COV Subscriptions

The full server's network Device reads use its actual execution profile:
service bits reflect the fixed dispatcher with clock-dependent time services,
and both COV list properties remain present after underlying Device declaration
changes. The selected Device receives live lists; other Devices receive empty
lists. Property_List and RPM selector/index behavior follow the same definitions.
WP and WPM reject assignment to Device service bits, both COV lists and
Property_List, including custom Device writers. Other custom properties retain
their existing write behavior. Changing a Device's declaration is not a runtime
service-disable switch.

The bundled server distinguishes ordinary, Single-property and Multiple-reference
subscriptions by their requested coordinates. All families match the original
client BACnet address independently of the immediate router. Ordinary and Single
use process/object coordinates, plus property/index for Single; their confirmed
mode remains mutable. Successfully admitted renewals select the current route,
including permitted ordinary indefinite renewals. Cancellation through either
router removes the same context; refused renewal preserves its route and terms.
Every ordinary/Single renewal advances its generation and follows the normal
observation reset/initial-notification path, fencing stale completion.

Multiple additionally keys on confirmed form. Its latest accepted finite request,
including an empty renewal of an existing context, retargets every retained
reference. Cancellation removes matching targets without retargeting survivors;
route changes fence old snapshots while preserving unreplaced observations. The
exception is a confirmed context whose report is outstanding, or failed and still
owed, at the move: the kept untimestamped references that report carried report
afresh (#923).
Different accepted array indexes stay distinct, and exact duplicate Multiple
references use final options once. Peer cleanup follows the current route for
all families. Claimed BACnet addresses do not establish authentication.
Live finite subscription notifications use positive ceiling seconds (bounded to
`u32::MAX`); ordinary indefinite subscriptions continue to report zero. Expired or
stale ownership at the final eligibility check cannot admit a new notification.
This does not retract bytes if cancellation races afterward. A Multiple context-only renewal
uses its current deadline, and each retained value must have its own live owner.
An already admitted confirmed notification continues its retry/ACK lifecycle.
Bundled server property subscriptions compare their selected value, using the same
validated sample for payload and baseline; failed reads never substitute
Present_Value. Numeric Present_Value alone inherits the object's increment;
other numeric properties without one report changes. Count coordinates and
supported structured/whole-array values ignore increments. Unclassified whole
arrays are refused regardless of increment presence. Retained samples have local
32-level / 1,024-node / 65,536-payload-byte caps, independent of traffic budgets;
admission overflow preserves existing subscriptions, and later overflow skips the
sample. See the [Rust selected-property profile](rust-api.md#cov-subscriptions)
for exact type, exceptional-number and shape policies.
Applicable Status_Flags changes also trigger reports independently of numeric
increments. Property notifications include declared-present flags once per object,
including non-Life-Safety objects. Selected values and flags advance one delivered
observation together. Read/encoding/size failure, or malformed declared-present
flags, skips the whole report without advancing either part; ordinary failed
Present_Value no longer produces a flags-only partial report. Declared absence
omits the companion. Disappearance alone does not trigger; a later present value
can trigger after a delivered absent observation. Multiple samples each context
separately under one DB/snapshot borrow and retains existing lifetime fences.
These server guarantees do not add a Python Single-property API, empty finite
Multiple contexts, delayed Multiple notifications or general numeric-array reduction.

#### `subscribe_cov(address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime=None)`

`confirmed` explicitly selects notification mode. A `None` or zero lifetime
creates an indefinite subscription; positive lifetimes are seconds. Use
`unsubscribe_cov` to cancel. The client always supplies the required mode,
including when the lifetime is omitted.

```python
await client.subscribe_cov(
    "192.168.1.100:47808",
    subscriber_process_identifier=1,
    monitored_object_identifier=ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    confirmed=True,
    lifetime=300,  # seconds, or None for indefinite
)
```

#### `unsubscribe_cov(address, subscriber_process_identifier, monitored_object_identifier)`

```python
await client.unsubscribe_cov(
    "192.168.1.100:47808",
    subscriber_process_identifier=1,
    monitored_object_identifier=ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
)
```

#### `cov_notifications() -> Awaitable[CovNotificationIterator]`

Returns an awaitable that yields an async iterator. Await the factory before
iterating; it can be called multiple times for independent consumers.

```python
notifications = await client.cov_notifications()
async for notif in notifications:
    print(f"Object {notif.monitored_object_identifier} changed:")
    for v in notif.values:
        print(f"  {v['property_id']}: {v['value']}")
```

---

### Discovery

#### `who_is(low_limit=None, high_limit=None)`

Broadcast a WhoIs request.

```python
await client.who_is()                    # all devices
await client.who_is(1000, 2000)          # instance range
```

The limits go together, each from 0 to 4194303 (Clauses 16.9 and 16.10).
Every discovery call that takes `low_limit` and `high_limit` raises
`ValueError` when only one is given, when `low_limit` is above `high_limit`,
or when a limit is past 4194303, before anything is sent (#1483); a single
limit used to go out as a request for every device.

A confirmed request goes to one device. A confirmed call such as
`read_property` or `write_property` to a broadcast or group address, such as
`"255.255.255.255:47808"`, the configured broadcast address at any port, or
an IPv4 multicast address, raises `BacnetError` before anything is sent
(#1479).

#### `who_has_by_id(object_id, low_limit=None, high_limit=None)`

Find a device hosting a specific object by identifier.

```python
await client.who_has_by_id(ObjectIdentifier(ObjectType.ANALOG_INPUT, 1))
```

#### `who_has_by_name(name, low_limit=None, high_limit=None)`

Find a device hosting a specific object by name.

```python
await client.who_has_by_name("Zone Temperature")
```

#### `discovered_devices() -> list[DiscoveredDevice]`

Get all discovered devices (populated by WhoIs/IAm).

```python
await client.who_is()
await asyncio.sleep(2)  # wait for responses
devices = await client.discovered_devices()
for dev in devices:
    print(f"Device {dev.object_identifier.instance} at {dev.mac_address}")
```

#### `get_device(instance) -> DiscoveredDevice | None`

Look up a specific device by instance number.

```python
dev = await client.get_device(1234)
if dev:
    print(f"Found: vendor={dev.vendor_id}, APDU={dev.max_apdu_length}")
```

#### `clear_devices()`

Reset the discovered devices table.

```python
await client.clear_devices()
```

#### `discover(timeout_ms=3000, low_limit=None, high_limit=None) -> list[DiscoveredDevice]`

Convenience method: send WhoIs, wait, return devices.

```python
devices = await client.discover(timeout_ms=5000)
for dev in devices:
    print(f"Device {dev.object_identifier.instance}")
```

#### `who_is_directed(address, low_limit=None, high_limit=None)`

Send a Who-Is to a specific device address (unicast).

```python
await client.who_is_directed("192.168.1.100:47808")
```

#### `add_device(device_instance, address)`

Manually add a device to the discovery table.

```python
await client.add_device(1234, "192.168.1.100:47808")
```

---

### Auto-Routing (by device instance)

These methods look up the device address from the discovery table. Useful when you've already discovered devices.

#### `read_property_from_device(device_instance, object_id, property_id, array_index=None) -> PropertyValue`

```python
value = await client.read_property_from_device(
    1234,  # device instance
    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    PropertyIdentifier.PRESENT_VALUE,
)
```

#### `read_property_multiple_from_device(device_instance, specs) -> list[dict]`

```python
results = await client.read_property_multiple_from_device(1234, [
    (ObjectIdentifier(ObjectType.ANALOG_INPUT, 1), [
        (PropertyIdentifier.PRESENT_VALUE, None),
    ]),
])
```

#### `write_property_to_device(device_instance, object_id, property_id, value, priority=None, array_index=None)`

```python
await client.write_property_to_device(
    1234,
    ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1),
    PropertyIdentifier.PRESENT_VALUE,
    PropertyValue.real(75.0),
    priority=8,
)
```

#### `write_property_multiple_to_device(device_instance, specs)`

```python
await client.write_property_multiple_to_device(1234, [
    (ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1), [
        (PropertyIdentifier.PRESENT_VALUE, PropertyValue.real(75.0), 8, None),
    ]),
])
```

---

### Time Synchronization

#### `time_synchronization(address, date, time)`

```python
await client.time_synchronization(
    "192.168.1.100:47808",
    date=(2026, 3, 21, 6),      # (year, month, day, day_of_week)
    time=(14, 30, 0, 0),        # (hour, minute, second, hundredths)
)
```

The request sets the peer's clock, so the date and time must be specific: a
real day with the full year (1900 to 2154), month 1 to 12, day 1 to 31 and
`day_of_week` that day's own weekday (1 = Monday), and every time field in
range. A field that is `UNSPECIFIED` (255) or a pattern value (month 13 for
odd months, day 32 for a month's end), or a weekday that doesn't match the
date, raises `ValueError` before anything is sent.

#### `utc_time_synchronization(address, date, time)`

Same arguments and checks as `time_synchronization`.

---

### Object Management

#### `create_object(address, object_specifier, initial_values=None) -> bytes`

Create an object on a remote device. `object_specifier` is either an `ObjectType` (server assigns instance) or an `ObjectIdentifier` (specific instance).

```python
# Create by type — server picks instance
raw = await client.create_object(
    "192.168.1.100:47808",
    ObjectType.ANALOG_INPUT,
    initial_values=[
        (PropertyIdentifier.OBJECT_NAME, PropertyValue.character_string("New AI"), None, None),
        (PropertyIdentifier.UNITS, PropertyValue.enumerated(62), None, None),
    ],
)

# Create with specific instance
raw = await client.create_object(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.ANALOG_INPUT, 100),
)
```

A rusty-bacnet server names an object created without an Object_Name after
its type and instance (`ANALOG_INPUT-2`), adding the first free ` (n)` when
another object holds that name. It also takes a few properties at creation
that WriteProperty refuses afterwards: Units on an Analog Input or Output,
and Number_Of_States (1 to 1024) on the multi-state types. A valid
Number_Of_States applies before the other initial values, and an
Alarm_Values entry past the count is refused. State_Text written whole, at
creation or by a later write, sets Number_Of_States to its number of labels;
with a Number_Of_States in the same request it has to match it, and a write
that would leave a state the object holds past the new count is refused
with VALUE_OUT_OF_RANGE. Writing a count to State_Text at `array_index=0`
resizes it the same way, adding `State n` labels when it grows.

#### `delete_object(address, object_id)`

```python
await client.delete_object(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.ANALOG_INPUT, 100),
)
```

---

### Device Management

#### `device_communication_control(address, enable_disable, time_duration=None, password=None)`

```python
await client.device_communication_control(
    "192.168.1.100:47808",
    EnableDisable.DISABLE_INITIATION,
    time_duration=60,       # minutes
    password="secret",
)
```

#### `reinitialize_device(address, reinitialized_state, password=None)`

```python
await client.reinitialize_device(
    "192.168.1.100:47808",
    ReinitializedState.WARMSTART,
    password="secret",
)
```

---

### Alarms & Events

#### `acknowledge_alarm_request(address, acknowledging_process_identifier, event_object_identifier, event_state_acknowledged, timestamp, acknowledgment_source, time_of_acknowledgment)`

Sends exactly the given fields. `timestamp` must echo the event notification's
timestamp, and `time_of_acknowledgment` is the acknowledging device's time.

```python
from rusty_bacnet import BACnetTimeStamp, EventState

await client.acknowledge_alarm_request(
    "192.168.1.100:47808",
    acknowledging_process_identifier=1,
    event_object_identifier=ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    event_state_acknowledged=EventState.HIGH_LIMIT,
    timestamp=notification_timestamp,  # from the event notification
    acknowledgment_source="operator",
    time_of_acknowledgment=BACnetTimeStamp.sequence_number(42),
)
```

#### `acknowledge_alarm(address, acknowledging_process_identifier, event_object_identifier, event_state_acknowledged, acknowledgment_source)` (deprecated)

Fills in both timestamps itself; prefer `acknowledge_alarm_request`.

```python
from rusty_bacnet import EventState

await client.acknowledge_alarm(
    "192.168.1.100:47808",
    acknowledging_process_identifier=1,
    event_object_identifier=ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    event_state_acknowledged=EventState.HIGH_LIMIT,
    acknowledgment_source="operator",
)
```

#### `get_event_information(address, last_received_object_identifier=None) -> bytes`

Returns raw encoded event information.

```python
raw = await client.get_event_information("192.168.1.100:47808")
```

---

### ReadRange

#### `read_range(address, object_id, property_id, array_index=None, range_type=None, reference_index=None, reference_seq=None, count=None, *, reference_time=None, validation="strict") -> dict`

Read a range of items from a list or log object. This method is shared by
`BACnetClient` and `EndpointClient`. Python supports all-items (`range_type=None`),
position, sequence and time forms. Invalid selectors,
array index zero and omitted or zero counts for a selected range raise
`ValueError` before address parsing or I/O. A count outside INTEGER16
(-32768 to 32767) raises `OverflowError` before address parsing or I/O (#1360). Omitted reference values default
to zero; zero position/sequence references are valid and may return no matches.
The typed `ReadRangeResult` dictionary preserves raw item bytes, the three-boolean
flags tuple and optional first sequence number. Endpoint responses must be
unsegmented; the standalone client's existing segmentation support is unchanged.

`range_type="time"` (#1533) reads the `count` items logged after
`reference_time`, or before it for a negative count. `reference_time` is a
`datetime.datetime` or a `((year, month, day, day_of_week), (hour, minute,
second, hundredths))` pair, and must name a specific instant. A device
timestamps its records in its own local time and the request carries no zone,
so a `datetime` must be naive and in the device's local time; an aware one
raises `ValueError` (convert it first with
`dt.astimezone(device_zone).replace(tzinfo=None)`). Hundredths come from the
microseconds, rounded down.

`validation="strict"` (the default) refuses an answer that breaks a ReadRange
rule with `BacnetReadRangeViolationError`, whose `rule` names it: an echo
mismatch, a first sequence number missing, zero or unexpected, MORE_ITEMS
with the flag for the end a ranged read moves toward (with no range, with
both FIRST_ITEM and LAST_ITEM), or more items than `count`. `validation="lenient"` (#1531)
keeps such an answer and lists the rules in `"violations"`, so a device that
numbers the record after its sequence wrap 0 doesn't cost the page.

```python
# Read by position
result = await client.read_range(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.TREND_LOG, 1),
    PropertyIdentifier.LOG_BUFFER,
    range_type="position",
    reference_index=1,
    count=10,
)
print(result["item_count"])     # number of items returned
print(result["result_flags"])   # (first_item, last_item, more_items)
print(result["item_data"])      # raw bytes
print(rusty_bacnet.decode_log_records(result))  # typed records

# Read by sequence number, keeping a page that breaks a rule
result = await client.read_range(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.TREND_LOG, 1),
    PropertyIdentifier.LOG_BUFFER,
    range_type="sequence",
    reference_seq=100,
    count=10,
    validation="lenient",
)
print(result["violations"])     # e.g. ["zero_first_sequence_number"]

# Read the records logged since a time, in the device's local time
result = await client.read_range(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.TREND_LOG, 1),
    PropertyIdentifier.LOG_BUFFER,
    range_type="time",
    reference_time=datetime.datetime(2026, 10, 5, 9, 0),
    count=50,
)

# Read all (no range)
result = await client.read_range(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.TREND_LOG, 1),
    PropertyIdentifier.LOG_BUFFER,
)
```

Return dict keys: `"object_id"`, `"property_id"`, `"array_index"`, `"result_flags"` (tuple of 3 bools), `"item_count"` (int), `"item_data"` (bytes), `"first_sequence_number"` (int or `None`), and `"violations"` (list of rule names; empty unless lenient).

#### `decode_log_records(result) -> list[dict]`

Decodes the records of a `read_range` result of a Trend Log, Event Log, Trend
Log Multiple or Audit Log's Log_Buffer (#1534). Each record is a dict with
`"timestamp"` (a `(date, time)` pair) and `"datum"`, whose `"kind"` names the
choice and whose key of the same name holds the value, for example
`{"kind": "real", "real": 72.5}`; a Trend Log record adds `"status_flags"`.
It raises `ValueError` for any other result, or when the item data doesn't
decode as `item_count` records, naming the first record that fails by index
and offset.

#### `read_log_page(address, object_id, cursor=None, page_size=100) -> dict`

Reads one page of a log's Log_Buffer (#1530), on `BACnetClient` and
`EndpointClient`. `cursor` is `None` or `"oldest"` (the oldest record, from
Record_Count and Total_Record_Count), `("sequence", n)`, `("position", n)` or
`("time", reference_time)`. The page is a dict: `"records"` (as
`decode_log_records` gives them), `"first_sequence_number"`, `"result_flags"`,
`"gap"` (`None`, or `{"expected", "first", "skipped"}` when the log no longer
holds the records asked for), `"violations"`, `"next"` (the cursor to read
from next), `"done"` and `"wrapped"` (the page reached the top of the
sequence range, so `next` wrapped to 1). Loop until `done`, then keep `next`
as the checkpoint; lists in place of the tuples work, so it survives
`json.dumps` and `json.loads`, a time cursor included. One request is
outstanding at a time, and sequence numbers wrap from the top of their range
to 1. A page whose records don't decode raises `BacnetError` naming the
first that fails, and the records before it are dropped; read that range with
`read_range` and `decode_log_records` to see them. An answer that breaks a rule
the reader can't tolerate, such as an echo that doesn't match the request,
raises `BacnetReadRangeViolationError`.

```python
cursor = None
while True:
    page = await client.read_log_page(address, trend_log, cursor, page_size=100)
    store(page["records"])
    cursor = page["next"]
    if page["done"]:
        break
save_checkpoint(cursor)
```

A device that answers with records from before the one asked for (bacnet-stack
1.6.1 does past its sequence wrap), or with none where its counts say it holds
records, raises `BacnetLogNotAdvancingError` instead of looping. Its sequence
numbers are inconsistent: read it from `("position", 1)`. A log that is merely
full doesn't need that; the reader starts over from the oldest record when
the log drops the one it asked for. A non-log object or a `page_size` outside
1..=32767 raises `ValueError` before I/O.

---

### File Services

#### `atomic_read_file(address, file_identifier, access_method, start_position=0, requested_octet_count=0, start_record=0, requested_record_count=0) -> bytes`

```python
# Stream access
data = await client.atomic_read_file(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.FILE, 1),
    access_method="stream",
    start_position=0,
    requested_octet_count=1024,
)

# Record access
data = await client.atomic_read_file(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.FILE, 1),
    access_method="record",
    start_record=0,
    requested_record_count=10,
)
```

#### `atomic_write_file(address, file_identifier, access_method, ...) -> bytes`

```python
# Stream write
result = await client.atomic_write_file(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.FILE, 1),
    access_method="stream",
    start_position=0,
    file_data=b"Hello, BACnet!",
)

# Record write
result = await client.atomic_write_file(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.FILE, 1),
    access_method="record",
    start_record=0,
    record_count=2,
    file_record_data=[b"record1", b"record2"],
)
```

---

### List Manipulation

Both methods require a nonempty sequence of complete encoded elements and a
nonzero optional `array_index`. Empty-valued elements (for example an empty
OctetString) are valid; an empty byte sequence is not. Invalid framing or index
zero raises `ValueError` synchronously before future creation, address parsing or
client access. Indexes outside native `u32` extraction raise `OverflowError`.
Framing uses the shared 1 MiB per-tag length and 32-level context limits, including
the service's outer `[3]`, and handles application Boolean without payload bytes.
Context, constructed and vendor encodings remain opaque: this does not validate
every application primitive or the remote property's datatype.

When the device refuses the request with a ChangeList-Error, the raised
`BacnetProtocolError` carries `first_failed_element_number`: the position, from
1, of the element that failed, or 0 when the request failed for another reason.

#### `add_list_element(address, object_id, property_id, list_of_elements, array_index=None)`

```python
await client.add_list_element(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 1),
    PropertyIdentifier.RECIPIENT_LIST,
    list_of_elements=encoded_bytes,
)
```

#### `remove_list_element(address, object_id, property_id, list_of_elements, array_index=None)`

```python
await client.remove_list_element(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 1),
    PropertyIdentifier.RECIPIENT_LIST,
    list_of_elements=encoded_bytes,
)
```

---

### Private Transfer

#### `confirmed_private_transfer(address, vendor_id, service_number, service_parameters=None) -> dict`

Send a vendor-specific confirmed service request. The result is a dict with
`vendor_id`, `service_number` and `result_block` (`bytes`, or `None` when the
ACK carries no block). A device error raises `BacnetProtocolError` with
`vendor_id`, `service_number` and `error_parameters` set; an ACK that is
malformed, cut short or has octets after its last member raises `BacnetError`.

```python
ack = await client.confirmed_private_transfer(
    "192.168.1.100:47808",
    vendor_id=999,
    service_number=1,
    service_parameters=b"\x01\x02\x03",
)
block = ack["result_block"]
```

#### `unconfirmed_private_transfer(address, vendor_id, service_number, service_parameters=None)`

Send a vendor-specific unconfirmed service request (fire-and-forget).

```python
await client.unconfirmed_private_transfer(
    "192.168.1.100:47808",
    vendor_id=999,
    service_number=1,
    service_parameters=b"\x01\x02\x03",
)
```

---

### Text Messages

#### `confirmed_text_message(address, source_device, message_priority, message, message_class_type=None, message_class_value=None)`

Send a confirmed text message to a device. It returns `None` once the device
acknowledges the message.

```python
from rusty_bacnet import MessagePriority

await client.confirmed_text_message(
    "192.168.1.100:47808",
    source_device=ObjectIdentifier(ObjectType.DEVICE, 1234),
    message_priority=MessagePriority.URGENT,
    message="Fire alarm on floor 3",
    message_class_type="numeric",     # "numeric" or "text"
    message_class_value=1,            # int for numeric, str for text
)
```

#### `unconfirmed_text_message(address, source_device, message_priority, message, message_class_type=None, message_class_value=None)`

Send an unconfirmed text message (fire-and-forget).

```python
await client.unconfirmed_text_message(
    "192.168.1.100:47808",
    source_device=ObjectIdentifier(ObjectType.DEVICE, 1234),
    message_priority=MessagePriority.NORMAL,
    message="Status update: all clear",
)
```

---

### Life Safety

#### `life_safety_operation(address, requesting_process_identifier, requesting_source, operation, object_identifier=None)`

Execute a life safety operation on a device.

```python
from rusty_bacnet import LifeSafetyOperation

await client.life_safety_operation(
    "192.168.1.100:47808",
    requesting_process_identifier=1,
    requesting_source="operator-console",
    operation=LifeSafetyOperation.SILENCE,
    object_identifier=ObjectIdentifier(ObjectType.LIFE_SAFETY_POINT, 1),
)
```

---

### Alarm Summaries

#### `get_alarm_summary(address) -> bytes`

Get a summary of all active alarms on a device.

```python
raw = await client.get_alarm_summary("192.168.1.100:47808")
```

#### `get_enrollment_summary(address, acknowledgment_filter=AcknowledgmentFilter.ALL, event_state_filter=None, event_type_filter=None, min_priority=None, max_priority=None, notification_class_filter=None) -> list[dict]`

Get enrollment summary with filters.

```python
from rusty_bacnet import AcknowledgmentFilter, EnrollmentSummaryEventStateFilter

summaries = await client.get_enrollment_summary(
    "192.168.1.100:47808",
    acknowledgment_filter=AcknowledgmentFilter.NOT_ACKED,
    event_state_filter=EnrollmentSummaryEventStateFilter.OFFNORMAL,
    min_priority=0,
    max_priority=255,
)
```

Each result dictionary contains `object_id`, `event_type`, `event_state`,
`priority`, and `notification_class`. The notification class is `None` when
the peer omits that optional ACK member; an explicit class zero remains `0`.

---

### COV Property Multiple

#### `subscribe_cov_property_multiple(address, subscriber_process_identifier, specs, issue_confirmed_notifications, max_notification_delay=None, lifetime=None)`

Subscribe to COV on multiple properties across multiple objects. `issue_confirmed_notifications` is now a required `bool`, including for cancellation requests. This is a breaking Python call-signature change; callers should pass it by keyword as shown below. For subscriptions and re-subscriptions, pass both `lifetime` and `max_notification_delay`. For whole-context cancellations, pass `specs=[]` and omit both timing fields.

Each supplied object specification must contain at least one property reference.
`PropertyIdentifier.ALL`, `PropertyIdentifier.OPTIONAL`, and
`PropertyIdentifier.REQUIRED` are not valid COV references. Complete request
validation, including timing and the cumulative 10,000-reference limit, raises
`ValueError` synchronously before an awaitable is returned, address parsing,
client-state access or I/O. An invalid later specification cannot dispatch a
valid prefix. Valid calls return an awaitable and retain normal remote protocol
errors. Empty outer lists remain encodable with omitted or valid finite timing;
this does not claim bundled-server materialization of a finite empty context.

```python
await client.subscribe_cov_property_multiple(
    "192.168.1.100:47808",
    subscriber_process_identifier=1,
    specs=[
        (ObjectIdentifier(ObjectType.ANALOG_INPUT, 1), [
            # (property_id, array_index, cov_increment, timestamped)
            (PropertyIdentifier.PRESENT_VALUE, None, 0.5, True),
            (PropertyIdentifier.STATUS_FLAGS, None, None, False),
        ]),
        (ObjectIdentifier(ObjectType.BINARY_INPUT, 1), [
            (PropertyIdentifier.PRESENT_VALUE, None, None, True),
        ]),
    ],
    issue_confirmed_notifications=True,
    max_notification_delay=10,
    lifetime=300,
)
```

---

### Write Group

#### `write_group(address, group_number, write_priority, change_list, inhibit_delay=None, *, network=None)`

Write values to the Channel objects of a control group (unconfirmed). Nothing
answers, so the call returns once the request is sent.

- `address`: one device's address, or `None` to broadcast.
- `network`: with `address=None`, broadcast on that remote network (1 to 65534),
  or on every network with 65535. Leave it `None` for the local network. An
  address and a network together raise `ValueError`.
- `group_number`: 1 to 4294967295; group 0 is reserved.
- `write_priority`: 1 to 16, used for entries that do not override it.
- `change_list`: a non-empty list of `(channel, override_priority, value)` tuples.
  - `channel` is a channel number (`int`, 0 to 65535) matching a Channel object's
    `Channel_Number`.
  - `override_priority` is 1 to 16, or `None` to use `write_priority`.
  - `value` is a `PropertyValue`, which the binding encodes, or a `bytes` or
    `bytearray` holding the encoded octets (#1359). Either way it must be one
    BACnetChannelValue with no wrapper tag: a single application-tagged
    primitive (`PropertyValue.real(72.0)`, `PropertyValue.null()`), or a
    constructed one given as `bytes` or as the same octets in
    `PropertyValue.application_data(...)`: a context-0 lighting command, or
    Addendum 135-2020ca's context-1 xy colour or context-2 colour command
    (#1474). Anything else, such as a
    `PropertyValue.list(...)`, raises `ValueError`, and a value of another
    type, a list of ints included, raises `TypeError`.
- `inhibit_delay`: optional Boolean. TRUE skips the execution delays of Channels whose
  `Allow_Group_Delay_Inhibit` is TRUE.

A value outside those rules raises `ValueError`, or `OverflowError` for integers that
don't fit, before anything is sent.

The Rust server and the Python `BACnetServer` execute WriteGroup on their
Channel objects, whose members may be in other devices; `add_channel` registers
one (see [Channels](#channels)).

```python
await client.write_group(
    "192.168.1.100:47808",
    group_number=1,
    write_priority=8,
    change_list=[
        # Channel 5 gets REAL 72.0; channel 6 gets NULL and writes at
        # priority 10 (NULL relinquishes, as with WriteProperty).
        (5, None, PropertyValue.real(72.0)),
        (6, 10, PropertyValue.null()),
    ],
    inhibit_delay=False,
)

# The same REAL for every device on network 5, already encoded
# (application tag 4).
await client.write_group(
    None, 1, 8, [(5, None, bytes([0x44, 0x42, 0x90, 0x00, 0x00]))], network=5
)
```

---

### Virtual Terminal

#### `vt_open(address, vt_class, local_vt_session_identifier) -> int`

Open a virtual terminal session. `vt_class` is a `VTClass`;
`local_vt_session_identifier` (0-255) is your own number for the session,
which the peer uses when it sends data back. Returns the remote session
identifier the peer assigned.

```python
remote_id = await client.vt_open(
    "192.168.1.100:47808",
    vt_class=VTClass.DEFAULT_TERMINAL,
    local_vt_session_identifier=5,
)
```

#### `vt_close(address, session_ids)`

Close one or more virtual terminal sessions. `session_ids` must contain at
least one identifier, each 0 to 255; an empty list raises `ValueError`, or
`OverflowError` for an integer that doesn't fit, before anything is sent.

```python
await client.vt_close("192.168.1.100:47808", session_ids=[1, 2])
```

#### `vt_data(address, session_id, data, data_flag) -> dict`

Send data on a virtual terminal session. `data_flag` is the sequence flag that
alternates between `False` and `True` with each new request on a session (it is
sent as an Unsigned 0 or 1). The result always has a boolean
`all_new_data_accepted`; `accepted_octet_count` is an `int` only when the peer
accepted part of the data, and `None` when it accepted all of it.

```python
ack = await client.vt_data(
    "192.168.1.100:47808",
    session_id=1,
    data=b"Hello VT",
    data_flag=False,
)
if not ack["all_new_data_accepted"]:
    sent = ack["accepted_octet_count"]
```

---

### Audit Services

The additive `_typed` methods accept dictionaries (or other Python mappings)
described by the installed `TypedDict` stubs. They convert into the native Rust
Audit request models before transport; the native client helpers remain the
only service encoder, transaction coordinator, and strict query-ACK decoder.
The mapping objects, wrapper values, and address string are not modified.

`AuditOperation` is the one Audit runtime wrapper. Its named constants are the
standard operations 0 through 15. `AuditOperation.from_raw()` remains lossless,
but request mappings accept only standard operations 0..15 or proprietary
operations 32..63; reserved 16..31 and values above 63 are rejected.

Recipients are discriminated mappings:

```python
device = {"kind": "device", "object_identifier": device_oid}
address = {
    "kind": "address",
    "network_number": 5,  # Unsigned16
    "mac_address": b"\x01\x02",
}
```

An `AuditNotificationInput` requires `source_device`, `operation`, and
`target_device`. Its optional keys are `source_timestamp`, `target_timestamp`,
`source_object`, `source_comment`, `target_comment`, `invoke_id`,
`source_user_id`, `source_user_role`, `target_object`, `target_property`,
`target_priority`, `target_value`, `current_value`, and `result`.
`target_property` uses `property_identifier` and optional
`property_array_index`; `result` is an `(ErrorClass, ErrorCode)` tuple.
`target_value` and `current_value` are `bytes | None` containing structurally
valid raw `ABSTRACT-SYNTAX.&Type` values, not `PropertyValue` objects.
`b""` is a present empty value (for example, an empty list); `None` omits
the optional field. Encoded NULL (`b"\x00"`) remains distinct. Typed
notification input and query projection preserve these distinctions. The
codec has no 32-octet limit; target Reporters include known complete values
through 32 encoded octets and omit larger values without truncating them.

#### `confirmed_audit_notification_typed(address, request) -> None`

Send one or more structured notifications and wait for the confirmed response.
The `notifications` list must contain 1..10,000 items.

```python
from rusty_bacnet import AuditOperation

await client.confirmed_audit_notification_typed(
    "192.168.1.100:47808",
    {
        "notifications": [
            {
                "source_device": device,
                "operation": AuditOperation.WRITE,
                "target_device": device,
                "target_property": {
                    "property_identifier": PropertyIdentifier.PRESENT_VALUE,
                },
                "target_priority": 8,
            }
        ]
    },
)
```

#### `unconfirmed_audit_notification_typed(address, request) -> None`

Send the same mapping contract without waiting for a response.

#### `audit_log_query_typed(address, request) -> AuditLogQueryAck`

The request requires `audit_log`, discriminated `query_parameters`, and an
Unsigned16 `requested_count` (0..=65535); `start_at_sequence_number` is an
optional corrected Unsigned64 cursor (0..=2**64-1). Query parameters use
`kind: "by_target"` with required `target_device_identifier`, or
`kind: "by_source"` with required `source_device_identifier`. Both require
`successful_actions_only` as the corrected `BACnetSuccessFilter` integer:
0 = all, 1 = successes-only, 2 = failures-only. The pre-RB-02 Boolean is
rejected with `TypeError` (use 1 for the old `True`, 0 for the old `False`);
3 to 255 raises `ValueError`, and an integer outside unsigned8 `OverflowError`. Optional fields follow the
installed `AuditLogQueryByTargetInput` and `AuditLogQueryBySourceInput`
definitions. `operations` is an integer bit mask:
bits 0..15 and 32..63 are permitted, while reserved bits 16..31, negative
values, and masks wider than 64 bits are rejected.

```python
ack = await client.audit_log_query_typed(
    "192.168.1.100:47808",
    {
        "audit_log": ObjectIdentifier(ObjectType.AUDIT_LOG, 1),
        "query_parameters": {
            "kind": "by_target",
            "target_device_identifier": ObjectIdentifier(ObjectType.DEVICE, 100),
            "operations": 1 << AuditOperation.WRITE.to_raw(),
            "successful_actions_only": 1,  # successes-only (was True pre-RB-02)
        },
        "requested_count": 100,
    },
)
```

The ACK always has exactly `audit_log`, `records`, and `no_more_items`. Each
record result has `sequence_number` and `record`; each record has `timestamp`
and `datum`. `timestamp` is `(date, time)`, where date is
`(full_year, month, day, day_of_week)` and time is
`(hour, minute, second, hundredths)`, matching the established
`BACnetTimeStamp.date_time(...).value` convention. Datum mappings use `kind`
values `"log_status"`, `"audit_notification"`, or `"time_change"`, with a
same-named payload key. `log_status` is an int of BACnetLogStatus flags: 1
log-disabled, 2 buffer-purged, 4 log-interrupted, decoded from the wire's
bit order (log-disabled in the top bit of the octet). Nested notifications use the canonical field names
listed above and include every optional key with either its decoded value or
`None`. ACK projection is all-or-error and never returns a partial mapping.

All mappings reject unknown keys. A non-mapping container, wrong field
container, or wrong wrapper/value type raises `TypeError`; missing required
keys, bad discriminators, reserved values, and integers that fit their field
but fall outside BACnet's range (a `target_priority` of 17) raise
`ValueError`. An integer outside its field's type raises `OverflowError`
(#1360): `invoke_id` or `source_user_role` past 255, `source_user_id`,
`requested_count` or a `network_number` past 65535, a negative or too wide
`start_at_sequence_number` or `operations` mask. Native validation, transport, and protocol failures use the
existing `BacnetError` hierarchy. Validation and native encoding complete
before an APDU can be sent.

This boundary follows the corrected 2020 baseline (ANSI/ASHRAE 135-2020 plus
Errata Summary 2024-04-29 items 7-8): Unsigned64 start sequence and the
three-state `BACnetSuccessFilter`, enforced end to end by retained-storage
filtering with a literal newest-first continuation cursor. It does not add
notification generation policy, authorization, persistence, or conformance
claims.

#### Raw Audit escape hatches

#### `confirmed_audit_notification(address, service_data)`

Send a confirmed audit notification. `service_data` is a raw escape hatch: the
caller must supply a complete AuditNotification-Request service payload encoded
to the Standard 135-2020 Clause 21 production. The method returns `None` after
the peer acknowledges the request.

```python
await client.confirmed_audit_notification(
    "192.168.1.100:47808",
    service_data=encoded_audit_bytes,
)
```

#### `unconfirmed_audit_notification(address, service_data)`

Send an unconfirmed audit notification (fire-and-forget). `service_data` is a
raw escape hatch and must contain the complete Clause 21
AuditNotification-Request service payload.

```python
await client.unconfirmed_audit_notification(
    "192.168.1.100:47808",
    service_data=encoded_audit_bytes,
)
```

#### `audit_log_query(address, service_data) -> bytes`

Send an audit log query and return the peer's raw response service payload.
`service_data` is a raw escape hatch and must contain the complete Clause 21
AuditLogQuery-Request service payload.

```python
raw = await client.audit_log_query(
    "192.168.1.100:47808",
    service_data=encoded_query_bytes,
)
```

These three methods remain signature- and byte-compatible generic outbound
paths. They do not validate the caller-provided payload, so corrected-contract
bytes pass through unchanged and no validation implication attaches to the raw
path. A bundled server with an
explicitly persisted Audit Log object can execute the raw AuditLogQuery payload
against its retained in-memory records and return a raw typed ACK payload. The
standalone Python server can receive confirmed and unconfirmed notifications
when an application [explicitly selects one sink and static admission policy](#inbound-audit-notification-sink).
The typed client boundary does not weaken that authorization or add query
authorization, producer behavior, forwarding, or new durable idempotency semantics.

---

### Additional Discovery

#### `who_am_i(vendor_id, model_name, serial_number)`

Broadcast a Who-Am-I request announcing this device's identity so that a
configuration tool can answer with You-Are. The three arguments are mandatory
and should match the Vendor_Identifier, Model_Name and Serial_Number properties
of the sending Device object. `vendor_id` is 0 to 65535 (`OverflowError` for an
integer that doesn't fit); a string that cannot be encoded raises `ValueError`.

```python
await client.who_am_i(260, "Controller-X", "SN-0001")
```

---

## BACnetServer

The full server constructs its owned
Device with the selected transport's stable local receive capacity: 1476 for
B/IP, B/IPv6 and SC, or 480 for MS/TP. Device
`Max_APDU_Length_Accepted` and I-Am therefore match the effective server
acceptance; negotiated SC egress does not change that declaration. This adds no
Python capacity argument and does not rewrite application-owned Rust Devices.
The MS/TP constructor correction has a non-hardware binding test; it is not
serial-hardware qualification. The [directional Rust contract](rust-api.md#local-receive-capacity-and-outgoing-limits)
explains raw declarations and Confirmed-Request header flooring. These changes
are new in 0.12.0.

Await `server.stop()` to join admitted work and release the transport. Cancelling
its Future retains the server's shutdown owner; a later `stop()` joins it, and a
cleanup error leaves the owner available for retry. Local mutation rejects once
shutdown begins. Successful stop clears the Python server's running instance.

Async BACnet server that hosts objects and responds to remote requests.

### Constructor

```python
server = BACnetServer(
    device_instance=1234,
    device_name="My BACnet Device",
    interface="0.0.0.0",
    port=47808,
    broadcast_address="255.255.255.255",
    transport="bip",             # "bip", "ipv6", or "sc"
    share_port_by_address=False, # keyword-only; B/IP, see "Sharing a port by address"
    # SC options same as BACnetClient
    mutation_policy="permissive", # keyword-only; "deny_all" denies covered network mutations
    dcc_password=None,           # password alone does not enable DCC
    dcc_policy="deny_all",       # keyword-only; explicit require_password or INSECURE legacy_permissive
    dcc_source_restriction=None, # optional list[(network_or_None, bytes)]; [] denies all; requires require_password
    dcc_disable_rate_limit=None, # optional (capacity, refill_interval_ms); (3, 20000) enables default rate
    reinit_password=None,        # password for ReinitializeDevice
    cov_policy=None,             # keyword-only dict of COV limits; see COV policy below
    time_sync_policy=None,       # keyword-only dict of clock limits; see Time synchronization policy below
)
```

`dcc_disable_rate_limit` is keyword-only and defaults OFF. When configured, one
global native-server bucket charges only authorized DISABLE_INITIATION requests;
authorized ENABLE never checks, charges or resets it. Capacity must be 1–65535
and refill interval 1–86400000 milliseconds (constructor validation). This does
not enable DCC or limit password guessing/ingress traffic. Admission charges survive
cancellation before commit; denial uses the existing policy-denied error/counter.
Each successful `start()` creates a new native server and full bucket, including
after `stop()` on the same Python object. An operator able to restart can reset
the budget. See [DCC policy](dcc-policy.md#optional-global-disable_initiation-rate-policy)
for exact timing, validation and exclusions.

### Adding Objects (before start)

All `add_*` methods must be called before `server.start()`. Objects cannot be added after the server is running.

#### Core I/O Objects

```python
# Analog objects (units: BACnet engineering units enum value, e.g. 62 = degrees-Fahrenheit)
server.add_analog_input(instance=1, name="Zone Temp", units=62, present_value=72.5)
server.add_analog_output(instance=1, name="Damper Cmd", units=62)
server.add_analog_value(instance=1, name="Setpoint", units=62)

# Binary objects
server.add_binary_input(instance=1, name="Occupancy")
server.add_binary_output(instance=1, name="Fan Enable")
server.add_binary_value(instance=1, name="Override")

# Multi-state objects
server.add_multistate_input(instance=1, name="Mode", number_of_states=4)
server.add_multistate_output(instance=1, name="Speed", number_of_states=3)
server.add_multistate_value(instance=1, name="Season", number_of_states=4)
```

#### Schedule & Notification

```python
from rusty_bacnet import EventType

server.add_calendar(instance=1, name="Holiday Calendar")
server.add_schedule(instance=1, name="Occupancy Schedule")
server.add_notification_class(
    instance=1,
    name="Critical Alarms",
    notification_class=1,
    storage_path="/application/state/class-1",  # optional
    recipients=[  # keyword-only; seeds Recipient_List in order
        {
            "recipient": {"kind": "device",
                          "object_identifier": ObjectIdentifier(ObjectType.DEVICE, 99)},
            "process_identifier": 1,
        },
    ],
)
server.add_notification_forwarder(
    instance=1,
    name="Forwarder",
    process_identifier_filter=None,  # None forwards every process identifier
    local_forwarding_only=False,
    storage_path="/application/state/forwarder-1",  # optional
)
server.add_notification_forwarder(
    instance=2,
    name="Seeded forwarder",
    recipients=[  # keyword-only; seeds Recipient_List in order
        {
            "recipient": {"kind": "device",
                          "object_identifier": ObjectIdentifier(ObjectType.DEVICE, 99)},
            "process_identifier": 1,
        },
        {
            "recipient": {"kind": "address", "network_number": 0,
                          "mac_address": bytes([192, 168, 1, 20, 0xBA, 0xC0])},
            "process_identifier": 7,
            "valid_days": 0b0011111,       # bit 0 Monday .. bit 6 Sunday
            "from_time": (8, 0, 0, 0),     # (hour, minute, second, hundredths)
            "to_time": (17, 30, 0, 0),
            "issue_confirmed_notifications": True,
            "transitions": 0b101,          # bit 0 to-offnormal, 1 to-fault, 2 to-normal
        },
    ],
    port_filter=[(0, True)],  # keyword-only; (port_id, enabled) per network port
)
server.add_alert_enrollment(
    instance=1,
    name="Alert",
    initial_source=ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
)
server.add_event_enrollment(
    instance=1,
    name="Event",
    event_type=EventType.OUT_OF_RANGE,  # default: EventType.CHANGE_OF_BITSTRING
)
```

A Notification Class with `storage_path` keeps the Recipient_List a client
writes in that file and serves it again after a restart; without it the list
lives in memory only. The file is replaced whole on each list write, and the
save runs on a thread of its own while the server goes on answering other
requests. A write whose list cannot be saved is refused with DEVICE /
OPERATIONAL_PROBLEM, and the class keeps its old list. `storage_path` takes a
`str` (a `pathlib.Path` raises `TypeError`, as for the forwarder). Give each
class its own file: the file records which class it belongs to, so two
classes sharing a path fail to register after a restart, and a file this
backend did not write, or a corrupt one, makes `add_notification_class` raise
`BacnetError`. `recipients` seeds the class's Recipient_List with
`Destination` mappings, with the forwarder's checks, errors and precedence
(see below): a written, saved list wins over the seed, which until then
applies at every start and is not saved (#1364).

The Notification Forwarder sends each event notification the server
receives, and each one its own objects address to its Device, on to the
destinations its Recipient_List and Subscribed_Recipients name. Clients
configure both lists over the network. `storage_path` keeps both lists in
one file, replaced whole when either list changes and at most once a minute
while the Subscribed_Recipients entries count down, so the lists and each
entry's remaining minutes survive a restart; without it the lists live in
memory only. Saves run on a thread of their own, and the server waits for a
list write's save without holding the object database, so a slow disk does not
hold up its other requests. A list write whose request fails before it lands
puts the saved lists back to the served ones at once. Every well-formed
ConfirmedEventNotification is acknowledged, whether or not a forwarder takes
it; a retransmission of one already received is acknowledged again but not
forwarded again. One notification goes to at most 64 destinations across all
the forwarders, and the server ignores a confirmed request sent by broadcast.

`recipients` seeds Recipient_List with `Destination` mappings, typed as the
`Destination` TypedDict in the stub. `recipient` takes the mapping the Audit
services use for a recipient, and `process_identifier` is required. A key
left out gives a destination that is active every day, all day, for every
transition, with unconfirmed notifications. The binding checks shapes and
Python types (unknown keys, values outside BACnet's range and malformed time
tuples raise `ValueError`, an integer outside its field's type
`OverflowError`, other wrong types `TypeError`), and the object's own
`add_destination` decides what the list holds, as it does for a client's
write: more than 32 destinations, or an address MAC longer than 18 octets,
raises `BacnetProtocolError`. `port_filter` serves Port_Filter as
`(port_id, enabled)` pairs, one per network port; the server receives through
Port_ID 0, and without `port_filter` the property is absent. Clients can still
change both lists over the network, within the limits Port_Filter's writes
allow. With `storage_path`, a Recipient_List a client wrote, once saved, wins
over `recipients`; until a write sets the list, `recipients` applies at every
start and is not saved. Both lists read back in these forms, each destination
with every key (see [typed constructed values](#typed-constructed-values)).
Save failures are counted by
[`forwarder_save_counters()`](#forwarder_save_counters---dictint-forwardersavecounters).

`initial_source` is required and becomes the Alert Enrollment object's
read-only `Present_Value`. This is an intentional breaking correction; there
is no sentinel/default source. The served Table 12-61 surface also removes the
former `Status_Flags`, `Out_Of_Service`, and `Reliability` compatibility
properties. This models source ownership only and does not add an Alert
evaluator or notification-generation flow.

#### Logging & Trending

```python
server.add_trend_log(instance=1, name="Temp Log", buffer_size=1000)
server.add_trend_log_multiple(
    instance=1,
    name="Multi Log",
    buffer_size=1000,
    members=[
        # A tuple for a property here, or a mapping, which can name a device.
        (ObjectIdentifier(ObjectType.ANALOG_INPUT, 1), PropertyIdentifier.PRESENT_VALUE),
        {"object_identifier": ObjectIdentifier(ObjectType.ANALOG_INPUT, 2),
         "property_identifier": PropertyIdentifier.PRESENT_VALUE},
    ],
    log_interval=6000,          # hundredths of a second: once a minute
    logging_type="polled",
    align_intervals=True,       # on each minute
)
server.add_event_log(instance=1, name="Event Log", buffer_size=500)
# Also records the event notifications the server receives.
server.add_event_log(2, "Received", 500, log_received_notifications=True)
server.add_audit_log(
    instance=1,
    name="Audit Trail",
    storage_path="/application/state/audit-trail",
    buffer_size=500,
)
server.add_audit_reporter(instance=1, name="Reporter")
```

The server's poller samples every `add_trend_log_multiple` member into one
record, one value per member in order; a client reads the records with
`read_range` on Log_Buffer. The keyword arguments are all optional:

- `members` is a list of at most 64 members, each an `(object, property)` or
  `(object, property, array_index)` tuple for a property in this device, or a
  `DeviceObjectPropertyReference` mapping (`object_identifier`,
  `property_identifier`, and optionally `property_array_index` and
  `device_identifier`), as `add_channel` takes them. A `device_identifier`
  that isn't a Device raises ValueError. A member naming the server's own
  Device is stored in its local form; one naming another Device logs a
  failure for its slot, since the server reads only its own objects.
- `log_interval` is in hundredths of a second. `logging_type` is `"polled"`
  or `"triggered"`. POLLED with no `log_interval` takes a one-minute interval;
  TRIGGERED sets Log_Interval to 0 and makes it read-only, so passing both
  raises `BacnetProtocolError` (WRITE_ACCESS_DENIED). A Trend Log Multiple
  never logs by COV: `"cov"` raises VALUE_OUT_OF_RANGE, as a client's write of
  COV to Logging_Type does. Without either argument Log_Interval stays 0 and
  nothing is polled.
- `start_time` and `stop_time` keep records only from the start up to, not
  including, the stop. Each is a `((full_year, month, day, day_of_week),
  (hour, minute, second, hundredths))` pair; every field 255 leaves that side
  open, 255 seconds or hundredths count as zero, and any other value that
  isn't an actual date and time raises VALUE_OUT_OF_RANGE. The log records
  LOG_DISABLED when the window closes and a clear status when it opens.
- `align_intervals=True` makes a polled log acquire on clock boundaries when
  Log_Interval divides a day, `interval_offset` hundredths (modulo the
  interval) after each one. The boundaries follow the device's clock, so
  setting that clock moves them too.

A triggered log logs one record each time Trigger is written TRUE, by a peer
or by the application through `write_property_local`; Trigger reads TRUE until
the poller has acquired the record:

```python
await server.write_property_local(
    ObjectIdentifier(ObjectType.TREND_LOG_MULTIPLE, 2),
    PropertyIdentifier.TRIGGER,
    PropertyValue.boolean(True),
    source_object=None,
)
```

Peers can write each of these properties; writing Trigger TRUE to a polled
log raises NOT_CONFIGURED_FOR_TRIGGERED_LOGGING.

All three logs take a keyword-only `total_record_count` (default 0) that
seeds Total_Record_Count, an Unsigned32 (#1537). The first record is numbered
one past it, so a log seeded at `2**32 - 2` numbers its records `2**32 - 1`,
then 1, 2 and on: a reader's handling of the wrap can be tested without
logging four billion records first. A value outside Unsigned32 raises
`OverflowError`. Restoring saved records is Rust-only, through
`restore_log_buffer`.

`add_trend_log` and `add_event_log` take none of the other keyword arguments,
but their objects serve the same rows over the network and through
`write_property_local`: a Trend Log has Logging_Type (POLLED or TRIGGERED),
Log_Interval, Start_Time, Stop_Time, Align_Intervals, Interval_Offset and
Trigger, and an Event Log has Start_Time and Stop_Time. The server can't log
by COV yet (#1480), so a Trend Log refuses a COV Logging_Type, and a polled
log's Log_Interval written from nonzero to zero (the older way to ask for
COV), with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.

A Trend Log's Log_DeviceObjectProperty reads as `application_data` holding the
context-tagged BACnetDeviceObjectPropertyReference. Without a reference it
reads as Analog Input 4194303's Present_Value (`0c 00 3f ff ff 19 55`), the
empty element a Trend Log Multiple grows by, and the log polls nothing.
Writing a reference whose object or Device is at instance 4194303 removes the
reference, and a null succeeds and changes nothing (#1417); before, it read
null while unset and a null write removed it.

Once the server runs with a valid Device clock, every event notification it
generates (an intrinsic or Event Enrollment transition, or an acknowledgment)
is recorded in each Event Log, stamped with that clock, even when no recipient
takes it. A client reads the records with `read_range` on `LOG_BUFFER`. Not
logged: an Event Log's BUFFER_READY reports, reports from an Event Enrollment
watching an Event Log, and transitions whose Notification Class is missing or
unreadable. The Rust API's Logging & Trending notes give the details.

An Event Log added with `log_received_notifications=True` also records the
Confirmed and UnconfirmedEventNotifications the server receives, unicast or
broadcast, each as it decoded, Process Identifier included (#1346). The option
is off by default. Each source, by network address, gets 5 records a second
across the collecting logs; past that a notification isn't logged and counts
in `event_notification_counters()["received_not_logged"]`. At most 33
sources' worth, 165 records a second, get in however many send, and up to
twice that within a second that straddles two windows. A received
BUFFER_READY report on an Event Log's buffer, and one claiming this device as
its source, are not logged.

Event Logs, Trend Logs and Trend Log Multiples report BUFFER_READY (#1347):
write Notification_Threshold (and Notification_Class) with
`write_property_local` or from a peer, and the log's Notification Class
recipients hear each time that many more records have been collected. Zero,
the default, reports nothing.

Every object that reports intrinsically also takes Event_Message_Texts_Config
(three strings, the Message Text of the TO_OFFNORMAL, TO_FAULT and TO_NORMAL
transitions in place of the server's own; an empty string leaves it) and
Event_Algorithm_Inhibit, which suspends the event algorithm but not fault
reporting (#1329). Write them with `write_property_local`; there are no
`add_*` keyword arguments for them. Event_Algorithm_Inhibit_Ref makes the
inhibit follow a local Boolean or BinaryPV property, read each time the
server evaluates the object.

An Audit Log's `storage_path` is application-owned and produces two sibling
snapshot files with `.slot0` and `.slot1` suffixes. Reuse the same path when
reopening that Audit Log; the server does not infer a global or
working-directory location.

An Audit Log's Buffer_Size takes a write, from a client or through
`write_property_local`, only while Log_Enable is FALSE; with logging on the
write raises `WRITE_ACCESS_DENIED`. A smaller size keeps the newest records
that fit, and a larger one keeps them all. The size written is stored with
the log, so a reopened log keeps it and `buffer_size` sizes only a log its
storage path does not hold yet; when the two differ the log keeps the stored
size and logs a warning. To give a stored log a new size, turn Log_Enable off,
write Buffer_Size and turn Log_Enable on again, or use a new storage path. Clients cannot purge an Audit Log, since its
Record_Count is read-only; the application calls
[`purge_audit_log`](#purge_audit_logobject_id).

#### Inbound Audit notification sink

```python
server.configure_audit_notification_sink(instance=1, policy="allow_all")
```

This synchronous, pre-`start()` method selects exactly one **already registered**
Audit Log. `policy` is a required keyword-only `Literal["deny_all", "allow_all"]`.
It replaces the previous selection only after validation succeeds; there is no
implicit first-log selection, even when only one Audit Log exists. Multiple logs
may be registered, but only the selected instance receives inbound notifications.

- **Default/unconfigured or `deny_all`:** fresh confirmed notifications receive
  `BacnetProtocolError` with `SERVICES/SERVICE_REQUEST_DENIED`; unconfirmed
  notifications are silently dropped. Denial creates no record, receipt, or forward.
- **Explicit `allow_all`:** both services use the existing Rust receiver, its
  64 KiB service-payload / 256-notification bounds, and atomic durable storage.
  Fresh confirmed success is acknowledged only after the record batch and exact
  request receipt commit together. Retained exact confirmed duplicates are silently
  discarded, including after file reopen, **before authorization** (so an already
  completed duplicate remains silent even after reopening with `deny_all`). Existing
  receipt retention/capacity limits still apply; this is not permanent deduplication.
  Unconfirmed receipt creates no request receipt and never sends a response, even
  on error. Successful unconfirmed client completion means *sent*, not *stored*.

The policy is **transport admission only**, not identity verification or query
authorization. `allow_all` admits any otherwise valid sender reaching this server;
deploy only on an appropriately isolated/trusted network. Payload `Source_Device`
and `Target_Device` remain **peer-reported content**, never verified origin. Static
Rust authorizers do not call Python or acquire the GIL; there are no callbacks or
allowlists. The existing runtime, database locks, error mapping and joined shutdown
remain in use.

Malformed policy or instance (including booleans, non-integers and values outside
`0..=4194303`) raises `ValueError`, except an instance outside unsigned32, which
raises `OverflowError`. Missing/wrong-type registrations and duplicate
registrations of the selected Audit Log instance also raise `ValueError`. An object
of another type with the same instance number is not an Audit Log. Selection is
revalidated at `start()` before transport preparation or registration transfer, so
a duplicate registered after selection cannot silently replace the sink. These
validation failures preserve pending registrations and the previous configuration.
Configuration while running raises `RuntimeError`. This does not add general
rollback for unrelated later startup failures.

**Migration:** existing constructors and `add_audit_log()` calls need no change and
continue to deny inbound notifications. To opt in, register the log, call the new
method with explicit `allow_all`, then start. Reopen with the same application-owned
storage path to query committed records. `add_audit_reporter()` alone remains inert
registration. The separately configured direct parent-forwarding and static target
Reporter subsets are described below. Full Reporter parity, shared-endpoint Audit
producers, full Audit/BIBB/BTL support and #345 closure are not claimed.

Local producer → durable logger → typed query example (loopback only, ten-record
buffer, one notification, one-record query; run inside an async function):

```python
import tempfile
from pathlib import Path
from rusty_bacnet import (
    AuditOperation, BACnetClient, BACnetServer, ObjectIdentifier, ObjectType,
)

with tempfile.TemporaryDirectory() as directory:
    server = BACnetServer(
        device_instance=503_512, interface="127.0.0.1", port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_audit_log(1, "Audit Sink", str(Path(directory) / "audit"), buffer_size=10)
    server.configure_audit_notification_sink(1, policy="allow_all")
    await server.start()
    try:
        address = await server.local_address()
        async with BACnetClient(
            interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            apdu_timeout_ms=2_000,
        ) as client:
            await client.confirmed_audit_notification_typed(address, {
                "notifications": [{
                    "source_device": {"kind": "device", "object_identifier":
                                      ObjectIdentifier(ObjectType.DEVICE, 1)},
                    "operation": AuditOperation.READ,
                    "target_device": {"kind": "device", "object_identifier":
                                      ObjectIdentifier(ObjectType.DEVICE, 2)},
                }],
            })
            page = await client.audit_log_query_typed(address, {
                "audit_log": ObjectIdentifier(ObjectType.AUDIT_LOG, 1),
                "query_parameters": {
                    "kind": "by_target",
                    "target_device_identifier": ObjectIdentifier(ObjectType.DEVICE, 2),
                    "successful_actions_only": 0,
                },
                "requested_count": 1,
            })
            assert len(page["records"]) == 1
    finally:
        await server.stop()
```

#### Direct Audit Log parent forwarding

Two synchronous methods connect the standalone Python receiver to the existing
[immediate Rust forwarding path](audit-log-forwarding.md), **for direct BACnet/IP
(IPv4) only**. Both servers in this example use `transport="bip"` (the default).
Configure their Audit Logs and explicit sink policies before their respective `start()` calls:

```python
# parent has Device instance 9 and registered Audit Log instance 7.
parent.configure_audit_notification_sink(7, policy="allow_all")
await parent.start()
try:
    # child has a different Device instance and registered Audit Log instance 1.
    child.configure_audit_notification_sink(1, policy="allow_all")
    child.add_device_binding(device_instance=9, address=await parent.local_address())
    child.configure_audit_log_parent(
        instance=1, parent_device_instance=9, parent_audit_log_instance=7,
    )
    await child.start()
    try:
        # Send confirmed_audit_notification_typed() to the child as in the example above.
        # A changed, durably accepted batch is eligible for one confirmed parent attempt.
        # Query Audit Log 7 on the parent to observe its independent commit.
        ...
    finally:
        await child.stop()
finally:
    await parent.stop()
```

- `add_device_binding(device_instance: int, address: str) -> None` registers a
  **direct configured B/IP** Device binding, only on a `transport="bip"` server.
  It reuses the client address parser but requires exactly six bytes: four IPv4
  octets followed by two big-endian UDP port octets. IPv4 `"127.0.0.1:47808"` and
  its equivalent colon-separated hex `"7f:00:00:01:ba:c0"` are accepted. IPv6
  (18 bytes), MS/TP (one byte), and other incompatible lengths raise `ValueError`
  before retaining a binding. Non-B/IP servers also raise `ValueError`, even for
  six-byte input. The shared parser's broader IPv6/hex/MS/TP grammar remains used
  by other APIs; it does not imply forwarding support here. Parsing is not
  reachability or authentication. Duplicate Device identifiers raise `ValueError`,
  even for the same address, without changing the first binding. There is no update/removal,
  routed-binding or discovery API. The Rust builder's 4096-binding capacity check
  runs at `start()` before registrations transfer (`ValueError`). The Rust
  build step then refuses a binding at a broadcast or other group address of
  the link, such as a multicast address, 255.255.255.255 or the configured
  broadcast IP at another port: `start()` raises `BacnetError` naming the
  device and the address (#1493). Bind each device at its unicast address.
- `configure_audit_log_parent(instance: int, *, parent_device_instance: int,
  parent_audit_log_instance: int) -> None` sets the registered local log's
  `Member_Of` reference. All identifiers must be integers in `0..=4194303`, not
  booleans. A local parent Device, invalid identifiers, missing/wrong-type local
  registrations or duplicate local Audit Log identities raise `ValueError`.
  A valid repeated call **replaces** that log's parent; an invalid call leaves it
  unchanged. Startup revalidates every parent-configured local log, including
  nonselected logs, before transfer so later duplicate registration cannot silently
  discard its configuration. The parent log is not remotely discovered or validated.
- Malformed address strings raise `ValueError`; non-string addresses raise
  `TypeError`. Instance and address syntax validation precede the frozen-state
  check. For parsed bindings, frozen-state rejection precedes the new transport
  and six-byte shape checks. Configuration calls with otherwise valid inputs after
  `start()` consumes registrations raise `RuntimeError`, including while startup
  is in flight and after stop.
  Validation/TLS/serial preparation failures before transfer do not freeze these
  settings. Later startup failures retain the existing lack of general rollback.
  Recreate the server, register the same storage paths and reapply settings on
  reopen: parent references and bindings are not durable snapshot data.

Only the **selected inbound sink** forwards accepted batches that change retained
record content. Parent configuration alone does not enable receipt or Reporter
production. `Member_Of` identifies the intended parent; AuditNotification has no
target-log parameter, so the receiving Device's sink selection decides which log
actually receives it. Configure that sink consistently and use an **acyclic hierarchy**.

With a parent configured, local/property reads expose `Member_Of` as the existing
context-tagged `PropertyValue.application_data` bytes, `Delete_On_Forward` as false,
`Issue_Confirmed_Notifications` as true, and `Reliability`; these properties remain
read-only over BACnet. An unresolved binding or local-address alias retains the
Rust `CONFIGURATION_ERROR` / no-forward behavior. Delivery failure never rolls back
the child commit or its ACK; a child ACK does **not** prove parent delivery. Query
the parent under an application deadline rather than assuming task ordering.

The RB-22 semantics are unchanged: at most one immediate confirmed attempt,
64 shared active Audit permits, one total three-second deadline, no waiting queue,
retry, deletion, bulk/backlog replay, durable outbox or restart-delivery guarantee.
Payload identities remain peer-reported; bindings do not authenticate a peer.
Installed-extension evidence covers loopback BACnet/IP child-to-parent receipt,
using IPv4:port and equivalent six-byte hex bindings, typed queries and file reopen.
IPv6/SC/MS/TP forwarding is not exposed by this Python slice; no VMAC discovery,
SC mapping, MS/TP hardware support or independent interop is claimed here.
This is not active Reporter configuration, full Audit/BIBB, BTL/certification
or issue #345 closure.

#### Target Audit Reporters

`BACnetServer.configure_audit_reporters(reporters: list[AuditReporterConfiguration]) -> None`
selects one through 64 pending Reporters through the shared Rust target owner. It
is synchronous, opt-in and pre-start only; `add_audit_reporter(instance, name)`
registers objects without selecting them. Every dict supplies the required fields
shown below; `monitored_objects`, `audit_priority_filter`, and `maximum_send_delay`
are optional. Omitted or `None` delay leaves both delay/control properties absent;
zero exposes them with immediate delivery. A valid call replaces the complete
selected set and settings. All validation precedes any object mutation.
See [election, overlap health and live Rust mutation](target-audit-reporters.md).

```python
# parent: running Device 9, file-backed Audit Log 7, explicit allow_all sink.
# child: a distinct, not-yet-started transport="bip" server (Device 8).
child.add_audit_reporter(1, "Target Reporter")
child.add_analog_value(1, "Writable target")
child.configure_audit_recipient({"kind": "device", "object_identifier": ObjectIdentifier(ObjectType.DEVICE, 9)})
child.configure_audit_reporters([{
    "instance": 1, "audit_level": "audit_all",
    "auditable_operations": 1 << AuditOperation.WRITE.to_raw(),
    "issue_confirmed_notifications": True,
    "monitored_objects": [ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)],
    "audit_priority_filter": 1 << 7,  # priority 8 only; omit for all priorities
    "maximum_send_delay": 0,  # paired controls, immediate; omit/None for absence
}])
child.add_device_binding(9, await parent.local_address())
# After child.start(), a public BACnetClient.write_property() to the child's
# target at priority=8 can produce a notification. Query the parent's Audit Log 7
# under a deadline to observe receipt; the write ACK alone is not that evidence.
```

- Each Reporter identifier must be a non-Boolean integer in `0..=4194302`.
  Empty/over-64 lists, duplicate identities and unknown fields are rejected.
  Provision the Device recipient independently with `configure_audit_recipient`,
  using a copied `AuditRecipientInput` Device or Address mapping. Device choices
  must be concrete, remote Device identifiers (instance 4194303 is reserved).
  Address choices use the supported direct unicast B/IP subset, on network zero
  or a network numbered 1 to 65534: an address naming the server's own network
  number is local once the server knows that number (a registered port's, or one
  learned from Network-Number-Is), and until then it starts unresolved with
  CONFIGURATION_ERROR (see [Device recipient writes](device-audit-recipient.md)). Missing provision
  fails startup before registration transfer, including at Audit_Level NONE.
  See [Device recipient writes](device-audit-recipient.md) for the complete bounded contract.
- `audit_level` is required and exactly `"none"`, `"audit_config"` or `"audit_all"`.
  Other strings raise `ValueError`; non-strings raise `TypeError`. DEFAULT and
  proprietary levels are not exposed. NONE suppresses ordinary reporting; AUDIT_CONFIG
  treats Present_Value writes as operational (suppressed), other property writes,
  implemented list/file writes and CREATE/DELETE as configuration operations.
- `auditable_operations` is a required non-Boolean integer mask, not a list or
  an operation ordinal: WRITE is `1 << AuditOperation.WRITE.to_raw()` (`2`).
  Bits 0–15 are standard operations; bits 32–63 are preserved proprietary positions.
  Reserved bits 16–31 raise `ValueError`, and negatives and u64 overflow
  `OverflowError` (#1360); wrong types, including bool, raise `TypeError`. Accepting a bit does not implement its source.
- `issue_confirmed_notifications` requires actual `True` or `False`; integers
  and truthy objects raise `TypeError`. The three optional dict fields are
  `monitored_objects`, `audit_priority_filter`, and `maximum_send_delay`.
  Validation finishes before object settings or selection change.
- `monitored_objects=None` (or omission) removes the optional `Monitored_Objects`
  property (`UNKNOWN_PROPERTY` on read) and selects all ordinary targets. An exact
  `list` selects exact `ObjectIdentifier`s or all instances of each `ObjectType`,
  including supported proprietary/extensible values from `ObjectType.from_raw()`.
  `None` entries are retained as ignored NULL selectors, not wildcards: `[]` and
  `[None]` select no ordinary targets. Duplicates/overlaps never duplicate records.
  Other containers (including list subclasses), raw integers/bools, strings,
  mappings or other element types raise `TypeError`, with an index for bad elements.
- `audit_priority_filter=None` (or omission) selects all priorities (`0xFFFF`).
  Otherwise it is a strict non-Boolean integer mask in `0..=65535`; `0x0000` is
  valid. Wrong types/bool raise `TypeError`, negatives/overflow raise `OverflowError`.
  Bit 0 selects priority 1 through bit 15 selecting priority 16. For example,
  `1 << 7` selects priority 8; `1 << 15` selects priority 16, also used by a write
  with omitted priority. Filtering applies to commandable-property writes, not
  non-commandable writes or non-write operations. Existing list/file/lifecycle
  behavior is unchanged; enabled Reporter-target writes retain their bypass.
- `maximum_send_delay=None` (or omission) leaves Maximum_Send_Delay and Send_Now
  absent. A non-Boolean integer in `0..=3600` exposes the pair: zero sends
  immediately, while positive values enable bounded ordinary target batching.
  Wrong types/bool raise `TypeError`; an integer past 3600 raises `ValueError`,
  and one outside unsigned32 `OverflowError`.
  See [delayed target controls and limits](delayed-target-audit.md).
- Every valid call replaces the complete selected set. Omitted options reset to
  catch-all selectors, all priorities, and absent delay/control properties.
  Failed calls preserve all settings and registrations.
  Input dictionaries, selector lists and values are copied. Enabled nominal overlaps
  expose CONFIGURATION_ERROR on every affected Reporter; only the lowest instance
  emits, before operation/value filters. Mandatory fallback does not add overlap.
- Binding and Reporter configuration may occur in either order. Device choices use
  existing configured direct B/IP IPv4 `add_device_binding()` routes. Address
  choices need no binding and use the direct unicast B/IP IPv4 subset described
  above. Neither choice adds discovery, routed, IPv6, SC or MS/TP destinations to
  this Python profile. An unresolved Device recipient does not fail configuration or
  startup: an enabled Reporter exposes `RELIABILITY=CONFIGURATION_ERROR` (`10`)
  and the fault bit in `STATUS_FLAGS`, without emitting or queuing ordinary records.
  Use existing `read_property()` on these properties; there is no separate status API.
- Startup revalidates every selected pending identity before draining registrations.
  Configuration freezes at ownership transfer: valid calls during startup, while
  running or after stop raise `RuntimeError`. Input validation precedes that check.
  Validation and TLS/serial preparation failures before transfer leave it retryable;
  later failures retain the existing lack of general rollback. Recreate and reapply
  static settings/bindings on a new server; they are not persisted.

By default `Monitored_Objects` remains absent (catch-all) and `Audit_Priority_Filter`
selects all priorities; `Audit_Source_Reporter` remains false. The Rust producer covers inbound WP/WPM elements, AddListElement/RemoveListElement,
AtomicWriteFile and CREATE/DELETE successes and authorized execution errors at
their existing operation boundaries, and each DeviceCommunicationControl change
the server carries out as DEVICE_DISABLE_COMM or DEVICE_ENABLE_COMM (see
[Audit records](dcc-policy.md#audit-records)). Normal operations require their operation
bit; enabled external Reporter property writes retain the core filter bypass.
Success omits Result; known execution errors include the response-mapped Error.
Each selected Reporter has its own optional bounded, memory-only AUDITING_FAILURE
resource-admission summary when its bit is enabled. No new ordinary producer source is introduced.

Delivery retains 64 shared immediate Audit permits, one total three-second deadline,
no retries or outbound segmentation, object-owned health and joined shutdown.
Optional `maximum_send_delay` (0–3600 seconds, `None` absent) provisions the paired
Maximum_Send_Delay/Send_Now properties on each selected target Reporter. Positive
delay uses bounded ordinary batching; zero exposes immediate delivery controls.
See [delayed target Audit reporting](delayed-target-audit.md) for limits, live wire
commands, historical-loss filtering and the three-second target stop drain. Delivery failure does not change the original operation result.
Unconfirmed send success proves only transport acceptance, not recipient storage.
No durable outbox, replay or restart-delivery guarantee is provided; the receiver's
file-backed storage is a separate contract. Installed-extension loopback tests in
`test_audit_api.py` prove queryable target WRITEs for both confirmation modes,
strict atomic validation, replacement, selector/priority filtering and Reporter-write
bypass, suppression, unresolved-recipient no growth and lifecycle freezing.
An inbound WriteGroup is audited as one WRITE per Channel it writes, from the
requester's address and with no invoke ID (`WriteGroupAuditTests`). Broader
source/bounds evidence remains the existing Rust Reporter suites, not
independent interoperability qualification.

Recipient changes through the active Device property also support local and
network writes; both of the change's notifications (to the old and new recipients,
or to the new one and by global broadcast when the old has no route) are admitted
atomically. Rust's supported
`write_local` operation also shares the target observer; raw database authoring
and physical Input sampling remain outside it. AV/BV policy rows are described
below. No Python live configuration/callbacks, payload-origin verification, standalone
source-side reporting, ordinary sample/event production,
source batching, durability, full Reporter/Audit/BIBB/BTL/certification,
independent interop or #345 closure is claimed.

#### Object-owned AV/BV Audit policy

`BACnetServer`, `BipEndpoint`, `ScEndpoint`, and `MstpEndpoint` accept three
creation-time keyword arguments on `add_analog_value` and `add_binary_value`:

- `audit_level`: `None` (absent), `"default"`, `"none"`, `"audit_config"`, or `"audit_all"`.
- `auditable_operations`: `None` (absent) or a u64 operation mask. Reserved bits 16–31 are rejected.
- `audit_priority_filter`: `None` (absent), `"inherit"` (present BACnet NULL), or a 16-bit mask. Bit 0 selects priority 1.

Validation occurs before registration, with no I/O. Invalid names and reserved
operation bits raise ValueError; a mask outside its integer type (negative, past
unsigned64 for `auditable_operations` or past 65535 for `audit_priority_filter`)
raises OverflowError (#1360); noninteger masks, including bool, raise TypeError. Endpoint pending
registrations retain the typed policy across their existing startup retry paths.
These options provision readable optional rows; they do not enable an endpoint
target Reporter or broaden its executing service set. The standalone target
Reporter uses the object overrides for READ/WRITE/CREATE/DELETE; its NONE setting
remains the master suppression boundary. Network writes can change provisioned
rows, but cannot add an absent row. Source reporting ignores remote object policy.

```python
server.add_analog_value(7, "Setpoint", audit_level="default",
                        auditable_operations=0b11, audit_priority_filter="inherit")
server.add_binary_value(8, "Enable", audit_level="audit_config")
```

Actual Audit_Level changes bypass the object's own NONE/WRITE suppression under
an enabled Reporter; actual Auditable_Operations changes bypass WRITE while the
effective level is enabled. These eligible actual changes reserve immediate
notification resources before mutation. A missing route or unavailable runtime,
capacity, confirmed lease or APDU fit returns SERVICES/SERVICE_REQUEST_DENIED;
the property remains unchanged. This is a local admission policy, not a delivery
guarantee. Equal-value, NULL and ordinary writes retain their existing behavior;
other failed writes retain ordinary filters.
AV/BV absent/NULL priority filters inherit the Reporter under the object-specific
clauses, despite conflicting generic wording. See the [Rust policy contract](rust-api.md#object-owned-audit-policy)
for the interpretation, WPM/local-write behavior, and bounded lifecycle support.

#### Building Control

```python
server.add_loop(
    instance=1,
    name="PID Loop",
    output_units=98,  # percent
    controlled_variable_units=62,  # degrees Celsius
    priority_for_writing=10,
)
server.add_command(
    instance=1,
    name="Occupancy",
    # Writing N to Present_Value runs action[N - 1]; each command is an
    # ActionCommand mapping.
    action=[
        [
            {
                "object_identifier": ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1),
                "property_identifier": PropertyIdentifier.PRESENT_VALUE,
                "property_value": PropertyValue.real(21.0),
                "priority": 8,
                "post_delay": 5,  # seconds before the next write
            },
            {
                "object_identifier": ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1),
                "property_identifier": PropertyIdentifier.PRESENT_VALUE,
                "property_value": PropertyValue.enumerated(1),
                "priority": 8,
                "quit_on_failure": True,
            },
        ],
        [],  # Present_Value 2 writes nothing
    ],
    action_text=["Occupied", "Unoccupied"],  # one text per list
)
server.add_timer(instance=1, name="Timer")
server.add_load_control(instance=1, name="Load Control")
server.add_program(instance=1, name="Program")
server.add_averaging(
    instance=1,
    name="Averaging",
    window_interval=900,  # seconds the window spans (900 when omitted)
    window_samples=15,  # samples it holds, 1..=1440 (15 when omitted)
)
server.add_staging(
    instance=1,
    name="Two-stage fan",
    present_value=5.0,
    min_present_value=0.0,
    units=62,
    priority_for_writing=8,
    # Each tuple is (limit, target value bits, deadband).
    stages=[(10.0, [False], 1.0), (20.0, [True], 1.0)],
    # References are always local and must be BO, BV, or BLO objects.
    target_references=[ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)],
    stage_names=["Off", "On"],
)
```

`add_command`'s keyword-only `action` sets the Command's Action array: one list
per element, each a list of `ActionCommand` mappings with
`object_identifier`, `property_identifier` and `property_value`, and optionally
`property_array_index`, `priority`, `post_delay` (seconds), `quit_on_failure`
(False when omitted) and `device_identifier`. `action_text` serves
Action_Text, one text per list. Both are read-only over the network. Action
reads back as these mappings with every key, the Write_Successful flags
included (see [typed constructed values](#typed-constructed-values)). `write_successful` is
accepted so such a mapping can be given back, but ignored: only a run sets
the flag, so every command starts False. A wrong shape or Python
type raises TypeError, and an unknown or missing key raises ValueError, as
does a `device_identifier` that isn't a Device object identifier, naming the
command (`action[0][1]: ...`), the check the other device reference
arguments run. The object's own setters refuse a priority outside 1 to 16, a value with no
encoding, or a text count that differs from the list count, raising
BacnetProtocolError with VALUE_OUT_OF_RANGE. Once the server runs, writing N to
the Command's Present_Value, over the network or with `write_property_local`,
makes list N's writes in order as the Rust server does (see
[Building Control](rust-api.md#building-control-7)): In_Process reads True
until the list ends, and All_Writes_Successful then reads True only if every
write succeeded. Zero, or an empty list, writes nothing. A command whose
`device_identifier` names another device is sent there as a confirmed
WriteProperty when the server has a binding for it, from `add_device_binding`
or an I-Am heard in the last ten minutes. With neither, the server first
broadcasts one Who-Is for that device's instance alone, on every network or,
for a device whose old I-Am it still holds, on the network that I-Am came
from, and waits the APDU timeout (3 seconds) from the send for its I-Am,
which binds it; with no I-Am the command fails and no WriteProperty is sent.
A device gets at most one Who-Is a
minute, so a command naming it within a minute of one that drew nothing
fails at once, and nothing at all is sent while DeviceCommunicationControl
restricts initiation. `stop()` ends a run it cuts short with In_Process
False.

`add_staging` validates the complete ladder and target mapping atomically; it
does not invent stage limits, deadbands, names, priorities, or targets. Each
stage's bit list must have the same length as `target_references`, and remote
device references are not exposed by this local-only Python boundary. Runtime
array writes retain their configured lengths so the coupled arrays never pass
through a partially configured state. Local target writes complete during
write handling; failures set source Reliability to `UNRELIABLE_OTHER` until a
current plan succeeds. `Out_Of_Service` decouples targets and returning to
service reapplies the current stage. Staging does not advertise intrinsic
reporting. It supports COV: a SubscribeCOV notification carries Present_Value,
Status_Flags and Present_Stage, and fires on a Present_Value move of at least
the writable `COV_Increment`, a Status_Flags change or a Present_Stage change.
A Loop's COV notification carries Present_Value, Status_Flags, Setpoint and
Controlled_Variable_Value and fires on a `COV_Increment` move of Present_Value
or a Status_Flags change. Loop Present_Value is network-writable only while
Out_Of_Service is TRUE; in service the application supplies it with
`set_present_value_local`.

The application that runs the loop's algorithm also feeds its measurement:
`await server.set_controlled_variable_value_local(loop_id,
PropertyValue.real(21.5))` sets Controlled_Variable_Value, which stays
read-only over the network. It takes a finite REAL, rejects other objects with
OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, and is accepted while Out_Of_Service is
TRUE. A SubscribeCOVProperty on the property is notified of the change; a
SubscribeCOV on the Loop is not, and its next notification carries the new
value. The server doesn't follow Controlled_Variable_Reference itself.
`add_loop`'s keyword-only `controlled_variable_units`,
`proportional_constant_units`, `integral_constant_units`,
`derivative_constant_units` (NO_UNITS when omitted) and `priority_for_writing`
(16 when omitted) set rows that are read-only over the network; units above
65535 or a priority outside 1 to 16 raise VALUE_OUT_OF_RANGE, and a priority
outside 0 to 255 `OverflowError`. Peers can write the Loop's Action (DIRECT
until written).

The Loop's Controlled_Variable_Reference and Manipulated_Variable_Reference,
and a Pulse Converter's Input_Reference, read as `application_data` holding
the context-tagged BACnetObjectPropertyReference. The Loop's
Setpoint_Reference reads as the BACnetSetpointReference, the same octets
inside opening and closing tag 0, or an empty `list` while unset (#1312).
Peers and `write_property_local` write them in those encodings, so a value
read writes back unchanged. The flat list of object identifier and enumerated
property these used to read as is refused with INVALID_DATA_TYPE, and
malformed octets with INVALID_DATA_ENCODING.

An unset reference reads as one to the reserved instance 4194303 (#1417):
the Present_Value of Analog Input 4194303 for Controlled_Variable_Reference,
of Analog Output 4194303 for Manipulated_Variable_Reference and of
Accumulator 4194303 for Input_Reference, so `application_data` holding
`0c 00 3f ff ff 19 55`, `0c 00 7f ff ff 19 55` and `0c 05 ff ff ff 19 55`.
Writing any reference to that instance clears the property, and the empty
value clears Setpoint_Reference. None of the four datatypes has a NULL, so a
null written to any of them, by a peer or with `write_property_local`,
succeeds and leaves it as it is (#1396).

**Migration (#1417):** these properties used to read null while unset and a
null write cleared them. Treat a reference whose object instance is 4194303
as unset, and write the unset form (for example the octets a fresh object
reads) to clear one.

An Event Enrollment's Object_Property_Reference, which peers can only read,
reads as Analog Input 4194303's Present_Value (`0c 00 3f ff ff 19 55`) while
the enrollment has no reference, instead of null (#1417). Its
Fault_Parameters reads as the context-tagged `none` choice (`08`) while no
fault algorithm is set, and writing that clears it; a null written to it,
which used to clear it, now succeeds and changes nothing.

An Accumulator serves Prescale only once one is configured; until then a
read is UNKNOWN_PROPERTY and Property_List leaves it out, instead of a null
read.

A Pulse Converter checks what its Input_Reference names (#1341). Reliability
reads CONFIGURATION_ERROR (10), and Status_Flags FAULT, while the reference
names a missing object or a property that isn't an Unsigned or INTEGER, and
NO_FAULT_DETECTED once it names one that is, or is unset; an array index of
0 (the size) is a fault too. The server checks it when a peer or
`write_property_local` writes the reference, when the object it names is
created or deleted, and on each counting pass, and reports the Status_Flags
change to COV subscribers. While the converter is out of service, peers and
`write_property_local` can write Reliability to simulate a fault, and the
return to service puts back the checked value. At least once a second the
running server reads the named property and adds each increase over the
previous reading to Count. When the reference names an Accumulator's
Present_Value, a lower reading is counted as its wrap past Max_Pres_Value;
from any other property, or after the reference changes, a reading only
sets the baseline.

A running server samples an Averaging object's Object_Property_Reference
itself, every Window_Interval / Window_Samples seconds but never more often
than every 100 ms, starting one spacing after `start()` and over again after
each write that empties the window. A missing object or property, a failed
read, or a value it can't average counts as a missed attempt. An object
without a reference, which reads as Analog Input 4194303's Present_Value
(`0c 00 3f ff ff 19 55`, #1417), is the application's to feed: about that
often it passes
each result with `await server.add_averaging_sample_local(averaging_id,
PropertyValue.real(21.5))`, or `None` when its reading failed. On an object
the server samples, such a call is one more attempt and doesn't move the
server's spacing. A BOOLEAN (counted as 0 or 1), Signed, Unsigned and
Enumerated sample is accepted as well as a finite REAL. Another datatype,
Double included, raises INVALID_DATA_TYPE and NaN or an infinity
VALUE_OUT_OF_RANGE, and a refused sample isn't counted;
other objects raise OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. The object keeps the
most recent Window_Samples attempts, each sample filling the next slot;
Minimum_Value, Maximum_Value and Average_Value cover the valid samples among
them, Attempted_Samples counts the attempts and Valid_Samples the valid ones.
With no valid sample in the window the statistics read `math.inf`,
`-math.inf` and NaN. Peers can write Window_Interval and Window_Samples, and a
write of either, of Object_Property_Reference, or of zero to Attempted_Samples
empties the window; out-of-range values raise VALUE_OUT_OF_RANGE. An
Object_Property_Reference written with the server's own Device in it is kept
as the local reference it names, and one naming another device is refused with
OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (#1153). Peers read and write it as the
context-tagged BACnetDeviceObjectPropertyReference; the flat application-tagged
form is refused with INVALID_DATA_TYPE (#1182). Writing a reference whose
object or Device is at instance 4194303 removes the reference, and a null
succeeds and changes nothing (#1417); it used to remove it. The
statistics and counts change together and go through the server's COV path.
SubscribeCOV on an Averaging object is refused, since Table 13-1 has no row for
it, but a property subscription (SubscribeCOVProperty or
SubscribeCOVPropertyMultiple) is notified when its property moves by the
subscription's COV increment, or on any change without one, and always when it
moves to or from NaN or an infinity; the report has no Status_Flags.

#### Lighting

```python
server.add_lighting_output(instance=1, name="Dimmer")
server.add_binary_lighting_output(instance=1, name="On/Off Light")
```

A Lighting Output's Present_Value and Relinquish_Default take a REAL level
from 0.0 to 100.0. A level above 0.0 and below 1.0, written locally or over the
network, is stored and read back as 1.0, the dimmest on level (#1385); one
outside 0.0 to 100.0 raises `BacnetProtocolError` with VALUE_OUT_OF_RANGE,
except Present_Value's warn values -1.0 (WARN), -2.0 (WARN_RELINQUISH) and
-3.0 (WARN_OFF), which act as those lighting commands do (#1384).
Tracking_Value reads the same level as Present_Value whenever no fade or ramp
is running.

A Lighting Output's `Lighting_Command` is a BACnetLightingCommand (#1263). It
reads as `application_data` holding the command's context-tagged fields, and
reads `b"\x09\x00"` (operation NONE) until written. Write it the same way,
through `write_property_local` or a client's `write_property`:

```python
# FADE_TO (1), target level [1] 50.0 % (REAL 0x42480000), priority [5] 8.
fade = PropertyValue.application_data(bytes.fromhex("0901" "1c42480000" "5908"))
await client.write_property(address, lighting_output, PropertyIdentifier.LIGHTING_COMMAND, fade)
```

The object checks each command against its operation as the Rust API notes
describe: NONE, a reserved operation, FADE_TO or RAMP_TO without a target
level, or a field out of range raises `BacnetProtocolError` with
VALUE_OUT_OF_RANGE. An `octet_string`, or any other datatype, raises
INVALID_DATA_TYPE. The object carries the command out (#1384): the FADE_TO
above puts 50.0 in Present_Value at once and moves Tracking_Value there over
the fade time, with In_Progress reading FADE_ACTIVE (1) until it arrives. The
[Rust API notes](rust-api.md#lighting--color-5) list what each operation does.
A [Channel](#channels) with a `Lighting_Command` member passes on a lighting
command written to its Present_Value: the fields above between `b"\x0e"` and
`b"\x0f"`, the opening and closing context tag 0.

#### Channels

```python
dimmer = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
fan = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
server.add_channel(
    instance=1,
    name="Zone Scene",
    channel_number=11,  # the number a WriteGroup names, 0 to 65535
    members=[
        (dimmer, PropertyIdentifier.PRESENT_VALUE),
        # The mapping form; it can carry a device_identifier too.
        {
            "object_identifier": fan,
            "property_identifier": PropertyIdentifier.PRESENT_VALUE,
        },
    ],
    execution_delay=[0, 500],  # milliseconds, one per member
    control_groups=[5, 7],  # WriteGroup groups; [0] (none) when omitted
    allow_group_delay_inhibit=True,
)
```

`add_channel(instance, name, channel_number, members=None,
execution_delay=None, control_groups=None, *, allow_group_delay_inhibit=False)`
registers a Channel. A member is an `(object, property)` or
`(object, property, array_index)` tuple for a property in this device, or a
`DeviceObjectPropertyReference` mapping, the shape an access rule's
`time_range` takes: `object_identifier`, `property_identifier`, and optionally
`property_array_index` and `device_identifier`. A member naming the server's
own Device is stored as the local reference it stands for, as it is when a
peer writes the list. A member in another device keeps its Device and is
written there with a confirmed WriteProperty when the server has a binding for
it, from `add_device_binding` or an I-Am heard in the last ten minutes, or
finds one with a Who-Is first, as for a Command's remote action (see
[Building Control](#building-control)). The server reads that member's
property there first to learn its datatype and converts the value to it as for
a local member. A read the device refuses leaves the value as written; one it
doesn't answer fails the member with no write sent (see the Channel paragraphs
under [Lighting & Color](rust-api.md#lighting--color-5)).
`execution_delay` holds one delay in milliseconds per member (zeros when
omitted), `control_groups` the groups whose WriteGroup the Channel takes, and
`allow_group_delay_inhibit` whether a WriteGroup that asks for no delays skips
them. Peers can write all of these, and Channel_Number, over the network.

Once the server runs, a value written to the Channel's Present_Value, over the
network or with `write_property_local`, goes on to each member at the write's
priority once that member's own delay has passed, converted to the member
property's datatype, as the Rust server does; a member in another device that
waits for its answer holds back no other member. Write_Status reads IN_PROGRESS
until every member is done and then SUCCESSFUL or FAILED, and Reliability
reports what kind of failure the first member to fail had; another
Present_Value write meanwhile is refused with BUSY. A
[WriteGroup](#write-group) naming one of the Channel's groups and its number
writes the value the same way.

A wrong shape or Python type raises `TypeError`. An unknown or missing mapping
key or a device that isn't a Device raises `ValueError`. A channel number
outside unsigned16, or an index (a tuple's or a mapping's), a delay or a group
outside unsigned32, raises `OverflowError` (#1360). The Channel's own checks
raise `BacnetProtocolError`: VALUE_OUT_OF_RANGE for a delay count that differs
from the member count or an empty group list, and
NO_SPACE_TO_WRITE_PROPERTY for more than 1024 members or 64 groups. Nothing is
registered after any of them.

#### Life Safety

```python
server.add_life_safety_point(instance=1, name="Smoke Detector")
server.add_life_safety_zone(instance=1, name="Floor 3 Zone")
```

Python-hosted servers currently expose no LifeSafetyOperation authorization or
trusted `Operation_Expected` state channel. Inbound LifeSafetyOperation is
therefore fail-closed (`SERVICES / SERVICE_REQUEST_DENIED`) for these objects.
Use the Rust server API when authorized silence/unsilence execution is required.

A running server's application sets the states it derives:
`await server.set_present_value_local(point_id, PropertyValue.enumerated(2))`
sets Present_Value (here ALARM) and
`await server.set_tracking_value_local(point_id, PropertyValue.enumerated(0))`
sets Tracking_Value. Each takes an Enumerated BACnetLifeSafetyState, standard
or from 256 to 65535, and raises VALUE_OUT_OF_RANGE for another number,
INVALID_DATA_TYPE for another datatype and OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
for an object other than a Life Safety Point or Zone. Each changes only its own
property, so latching Present_Value until reset is up to the application.
Present_Value is taken while Out_Of_Service is set; a Tracking_Value then waits
for the return to service behind a client's simulated one. A SubscribeCOV on
the object hears Present_Value changes, and a property subscription on
Tracking_Value hears that one.

#### Access Control

```python
lock = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)
door = ObjectIdentifier(ObjectType.ACCESS_DOOR, 1)
server.add_access_door(instance=1, name="Main Entry", door_members=[lock])
server.add_access_point(instance=1, name="Lobby Access", access_doors=[door])
server.add_access_credential(instance=1, name="Badge 001")
badge = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 1)
server.add_access_user(instance=1, name="John Doe", credentials=[badge])
server.add_access_rights(instance=1, name="Employee Access")
lobby = ObjectIdentifier(ObjectType.ACCESS_POINT, 1)
server.add_access_zone(instance=1, name="Building A", entry_points=[lobby])
# Wiegand 26 (8) in class 0, and vendor 260's CUSTOM (2) format 7 in class 3.
server.add_credential_data_input(
    instance=1,
    name="Card Reader",
    supported_formats=[(8, 0), ((2, 260, 7), 3)],
)
```

The keyword arguments set arrays that are read-only over the network.
`door_members` and `access_doors` take an `ObjectIdentifier` for an object in
this device or a `(device, object)` pair for one in another device; a pair
whose device isn't a Device raises `ValueError`, and an `access_doors`
element that isn't an Access Door raises `BacnetProtocolError`
(VALUE_OUT_OF_RANGE). `entry_points` and `exit_points` set an Access Zone's
Entry_Points and Exit_Points lists in the same element forms, and an element
that isn't an Access Point raises `BacnetProtocolError` (VALUE_OUT_OF_RANGE).
`add_access_user` sets an Access User's Credentials, Members and Member_Of
lists the same way: `credentials` names the user's Access Credentials, and
`members` and `member_of` the Access Users one level below and above it,
here or in another device. An element of another object type raises
`BacnetProtocolError` (VALUE_OUT_OF_RANGE).
A read of Entry_Points, Exit_Points, Credentials, Members or Member_Of
gives the references back in the forms the keywords take (#1344), or `[]`
while the list is empty. `supported_formats` takes
`(format, format_class)` pairs, a format being a
BACnetAuthenticationFactorType number or a
`(format_type, vendor_id, vendor_format)` triple, which a CUSTOM format
needs; an ill-formed format raises VALUE_OUT_OF_RANGE, and a vendor member
outside unsigned16 `OverflowError`.

`add_access_point` also takes `number_of_authentication_policies` (1 when
omitted, never 0) and `priority_for_writing` (16 when omitted, else 1 to
16; outside 0 to 255 `OverflowError`), which set rows that are read-only over
the network, and
`supported_authorization_modes`: the BACnetAuthorizationMode numbers the
application carries out, AUTHORIZE (0) alone when omitted, AUTHORIZE always
among them, proprietary ones from 64 to 65535. A value outside those raises
VALUE_OUT_OF_RANGE. Peers pick the policy in effect by writing
Active_Authentication_Policy (1 to the policy count) and the mode by writing
Authorization_Mode (one of the supported modes). Another value is refused
with VALUE_OUT_OF_RANGE, and another datatype with INVALID_DATA_TYPE.

`authentication_policies` describes the policies as `(name, policy)` pairs
(#1325), a policy being `([(credential_data_input, index), ...],
order_enforced, timeout)`, each Credential Data Input in the forms
`access_doors` takes and the timeout in seconds (0 for none). The point then
serves Authentication_Policy_List and Authentication_Policy_Names, read-only
over the network, and the policy count becomes the number of pairs; a
`number_of_authentication_policies` given too is applied after and resizes
both arrays. An empty list, more than 256 pairs, or a count above 256 with
the pairs raises VALUE_OUT_OF_RANGE. A policy with no entries, a reference to
anything but a Credential Data Input, or indexes that don't start at 1 and
climb by at most one is kept but can't be in effect: peers can't select it,
and while it is the one in effect Active_Authentication_Policy reads 0.
While any such policy is listed, or the active policy is 0, Reliability reads
CONFIGURATION_ERROR and the point takes no access events. The point's
Reliability then takes simulated writes while it is out of service.

`add_access_door` also takes `alarm_values`, `fault_values` and
`masked_alarm_values`, the door's starting Alarm_Values, Fault_Values and
Masked_Alarm_Values as BACnetDoorAlarmState numbers other than NORMAL (1 to
8, or 256 to 65535); peers can write all three. Any other number, NORMAL (0)
included, raises VALUE_OUT_OF_RANGE. Door_Alarm_State stays
NORMAL or a member of the alarm or fault values and never takes a masked
state, so a client's simulated value outside them is refused, and masking the
state the door is in returns it to NORMAL. The door raises a CHANGE_OF_STATE
alarm once Door_Alarm_State has stayed in Alarm_Values for Time_Delay, and a
fault value makes Reliability MULTI_STATE_FAULT. Deciding that the door is in
alarm, DOOR_OPEN_TOO_LONG included, is up to the application.

```python
# Alarm on DOOR_OPEN_TOO_LONG (2) and FORCED_OPEN (3); fault on DOOR_FAULT (5).
server.add_access_door(
    instance=2, name="Side Entry", alarm_values=[2, 3], fault_values=[5]
)
```

`add_access_zone` takes `alarm_values` too: the zone's starting Alarm_Values,
as BACnetAccessZoneOccupancyState numbers other than NORMAL (1 to 6, or 64 to
65535), which peers can also write. The zone raises a CHANGE_OF_STATE alarm
once Occupancy_State has stayed in Alarm_Values for Time_Delay. Any other
number, NORMAL (0) included, raises `BacnetProtocolError` (VALUE_OUT_OF_RANGE)
whose `first_failed_element_number` names it, from 1, and nothing is
registered. Left out, the list starts empty.

```python
# Alarm when the zone is above its upper limit (4) or counting is DISABLED (5).
server.add_access_zone(instance=2, name="Building B", alarm_values=[4, 5])
```

`add_access_rights` takes `positive_access_rules` and `negative_access_rules`,
lists of `AccessRule` mappings for Positive_Access_Rules and
Negative_Access_Rules, and `enable` for the object's Enable flag:

```python
# Access Zone 3 in Device 99.
remote_zone = (
    ObjectIdentifier(ObjectType.DEVICE, 99),
    ObjectIdentifier(ObjectType.ACCESS_ZONE, 3),
)
server.add_access_rights(
    instance=3,
    name="Lobby Weekdays",
    positive_access_rules=[
        {
            # When: Schedule 1's Present_Value; leave out for any time.
            "time_range": {
                "object_identifier": ObjectIdentifier(ObjectType.SCHEDULE, 1),
                "property_identifier": PropertyIdentifier.PRESENT_VALUE,
            },
            # Where: an Access Point or Access Zone; leave out for anywhere.
            "location": ObjectIdentifier(ObjectType.ACCESS_POINT, 1),
            "enable": True,
        },
    ],
    negative_access_rules=[{"location": remote_zone, "enable": True}],
)
```

`enable` is required. A `time_range` mapping takes `object_identifier`,
`property_identifier` and the optional `property_array_index` and
`device_identifier`; a `location` takes the forms `door_members` does. A
missing or `None` member makes the rule apply at any time (ALWAYS) or at every
access point (ALL). A wrong type raises `TypeError`; an unknown or missing key,
or a device that isn't a Device, raises `ValueError`; a location naming
another object type raises `BacnetProtocolError` (VALUE_OUT_OF_RANGE), and so
does a list of more than 1024 rules (NO_SPACE_TO_WRITE_PROPERTY). Each
rule reads back as an `AccessRule` mapping with every key, `None` standing
for ALWAYS and ALL, so a read array can be handed back to the keyword
(#1344).

`enable` (a bool, `True` when omitted) sets Enable, which a peer reads and
writes as `PropertyIdentifier.LOG_ENABLE` (property 133); `False` disables
every rule in both arrays. Peers can also write the arrays: the whole array
as `PropertyValue.application_data` holding the rules' octets back to back,
one rule at an `array_index`, or the size at index 0. A grown array gets
disabled SPECIFIED rules with unspecified references, and a shrunk one loses
its last rules. Writes get the same checks as the keywords, and a refused
write leaves the array unchanged. The server stores and serves the rules and
the flag; it doesn't evaluate them.

`accompaniment` serves the object's optional Accompaniment row: the Access
Rights, Access Credential or Access User object a second credential,
presented with the first, has to match. It takes the forms `door_members`
does, and instance 4194303 asks for no accompaniment:

```python
# A holder of these rights needs Access User 7 of Device 99 alongside.
server.add_access_rights(
    instance=4,
    name="Vault Escort",
    accompaniment=(
        ObjectIdentifier(ObjectType.DEVICE, 99),
        ObjectIdentifier(ObjectType.ACCESS_USER, 7),
    ),
)
```

Without the keyword the object has no Accompaniment row, and a read or write
of it gets UNKNOWN_PROPERTY. Once served, it is in Property_List and peers
can write it as `PropertyValue.application_data` holding the reference's
octets; a read gives the reference back in the form the keyword takes
(#1344). A pair whose device isn't a
Device raises `ValueError`, and another object type raises
`BacnetProtocolError` (VALUE_OUT_OF_RANGE), from the keyword or a write.

With `storage_path`, the arrays, Enable and Accompaniment that peers write are
kept in that file and served again after a restart; without it they live in
memory only:

```python
server.add_access_rights(
    instance=3,
    name="Lobby Weekdays",
    positive_access_rules=[
        {"location": ObjectIdentifier(ObjectType.ACCESS_POINT, 1), "enable": True},
    ],
    storage_path="/application/state/access-rights-3",  # optional
)
```

Each written array, Enable or Accompaniment is saved before the object serves
it, on a thread of its own while the server goes on answering other requests.
A write that cannot be saved is refused with DEVICE / OPERATIONAL_PROBLEM, and
nothing changes. Once a write has set one of them, the saved value wins at
every later start: the keyword for it is still checked, but not applied, and
a saved Accompaniment is served even without the keyword. To lift a saved
Accompaniment requirement, write the no-accompaniment reference (instance
4194303), which keeps the row; to drop the row, remove the storage file,
which also drops the saved rules and Enable. A keyword whose
property no write has set applies as usual, and keyword values alone are
never saved, but a write saves the whole array it leaves, so an element write
also saves the keyword rules it didn't touch. `storage_path` takes a `str` (a
`pathlib.Path` raises `TypeError`). Give each object its own file: the file
records which object it belongs to, so two objects sharing a path fail to
register after a restart. A file this backend did not write, or a corrupt
one, makes `add_access_rights` raise `BacnetError`, and one holding a rule or
an Accompaniment the object refuses raises `BacnetProtocolError`.

Access Door, Access Point, Credential Data Input and Load Control take
SubscribeCOV, and each report carries the values their Table 13-1 rows name:
Door_Alarm_State on a door; Access_Event (in place of Present_Value),
Access_Event_Tag, Access_Event_Time, Access_Event_Credential and
Access_Event_Authentication_Factor on an Access Point; Update_Time on a
Credential Data Input; and Requested_Shed_Level, Start_Time and Shed_Duration
on a Load Control. While a door's Out_Of_Service is TRUE, clients can write
its Door_Status, Lock_Status and Door_Alarm_State to simulate it (Table 12-30
footnote 1); returning it to service brings back the door's own values. A
door's Secured_Status follows its command, Door_Status and Lock_Status,
simulated or not (Clause 12.26.14). A Credential Data Input's Present_Value
and Reliability take writes the same way (Table 12-43 footnote 1); a
simulated Present_Value must name one of the reader's `supported_formats`
with its class, or be the UNDEFINED or ERROR factor with class 0. An Access
Zone's Occupancy_Count and Reliability take writes the same way too (Table
12-37 footnote 1), and the return to service brings back the zone's own
values. Writing an Access Point's Out_Of_Service records an access event on
each edge, OUT_OF_SERVICE on entry and OUT_OF_SERVICE_RELINQUISHED on the
return, each with the next Access_Event_Tag and the Device clock's time, and
each sends the point's COV report. Supported_Formats,
Supported_Format_Classes, Door_Members and Access_Doors read as arrays (index
0 is the size), each reference and format in the form its keyword argument
takes (see [typed constructed values](#typed-constructed-values)).

A running server takes these objects' inputs from the application (#1132):
`await server.report_access_event_local(point, event, tag, time=...,
credential=..., authentication_factor=...)` records an access event,
`await server.report_credential_read_local(reader, (format_type,
format_class, value), update_time=...)` a reader's read, and
`await server.report_door_state_local(door, door_status=...,
lock_status=..., door_alarm_state=...)` the door's hardware state. A time
left out is the Device clock's, a credential left out the no-credential
reference and a factor left out the UNDEFINED one. Each changes its values
together, sends the object's COV report when its trigger moves and runs the
door's event algorithm at once. A value the object refuses raises
VALUE_OUT_OF_RANGE with nothing changed, and another object raises
OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. While an object's Out_Of_Service is
TRUE, the point refuses an event with WRITE_ACCESS_DENIED, and the door and
the reader keep the reported values aside in place of the device's earlier
ones: a client's simulated values stay served, with no COV report, and the
return to service serves the latest values reported.

#### Transportation

```python
server.add_elevator_group(instance=1, name="Elevator Bank A")
# Optional Machine_Room_ID: must name a Positive Integer Value object, else a
# BacnetProtocolError (VALUE_OUT_OF_RANGE). Read-only over the network.
server.add_elevator_group(
    instance=2,
    name="Elevator Bank B",
    machine_room_id=ObjectIdentifier(ObjectType.POSITIVE_INTEGER_VALUE, 5),
)
server.add_escalator(instance=1, name="Escalator 1")
server.add_lift(instance=1, name="Elevator 1", num_floors=10)
```

#### Groups & Views

```python
server.add_group(instance=1, name="HVAC Group")
server.add_group(
    instance=2,
    name="Zone 1",
    members=[(ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
              [(PropertyIdentifier.PRESENT_VALUE, None),
               (PropertyIdentifier.OBJECT_NAME, None)])],
)
server.add_global_group(instance=1, name="All Temps")
server.add_structured_view(instance=1, name="Floor Plan")
```

`add_group(instance, name, members=None)` takes the members as the endpoint
owners' `add_group` does: the `read_property_multiple` spec shape, checked
member by member when it is added, with the same `ValueError` and
`OverflowError` cases (see
[Endpoint Groups](#endpoint-groups-and-the-read-work-limit)). The server
rebuilds the Group's Present_Value from the members on every read, one result
per member, and each member row counts against `rpm_max_result_elements`; a
read past it is aborted with OUT_OF_RESOURCES (see [RPM budgets](rpm-budget.md)).
List_Of_Group_Members reads back in the spec shape, and Present_Value as the
`read_property_multiple` results of the members (see
[typed constructed values](#typed-constructed-values)).

#### Extended Value Types

```python
server.add_integer_value(instance=1, name="Counter")
server.add_positive_integer_value(instance=1, name="Index")
server.add_large_analog_value(instance=1, name="Energy Total")
server.add_character_string_value(instance=1, name="Description")
server.add_octet_string_value(instance=1, name="Raw Data")
server.add_bit_string_value(instance=1, name="Flags")
server.add_date_value(instance=1, name="Install Date")
server.add_time_value(instance=1, name="Start Time")
server.add_date_time_value(instance=1, name="Timestamp")
server.add_date_pattern_value(instance=1, name="Weekdays")
server.add_time_pattern_value(instance=1, name="Work Hours")
server.add_date_time_pattern_value(instance=1, name="Schedule Pattern")
```

#### Measurement & File

```python
server.add_accumulator(instance=1, name="kWh Meter", units=70)      # 70 = kilowatt-hours
# Present_Value x 10**-2 in units, and one count per 100 pulses.
server.add_accumulator(instance=2, name="Gas Meter", units=80, scale=-2, prescale=(1, 100))
server.add_pulse_converter(instance=1, name="Pulse Count", units=95) # 95 = counts
server.add_file(instance=1, name="Config File", file_type="text/plain")
server.set_file_data(instance=1, data=b"mode=occupied\n")

server.add_file(instance=2, name="Record File")
server.set_file_access_method(instance=2, access_method="record")
server.set_file_records(instance=2, records=[b"first", b"second"])
```

`add_accumulator`'s keyword-only `scale` sets Scale: a `float` is a float
scale, a single-precision REAL that multiplies Present_Value, and an `int` an
integer scale, the power of ten that does. Left out, Scale is the float scale
1.0. `prescale`, a `(multiplier, modulo_divide)` pair of unsigned32 values,
serves the optional Prescale, which is absent (a read is UNKNOWN_PROPERTY)
without it. Both go out in their context-tagged Clause 21 forms and read back
as these values (#1487). A bool or another type raises `TypeError`, an
integer outside its type `OverflowError`, and a float that isn't finite as a
REAL or a `prescale` of another length `ValueError`.

#### Configured Network Port snapshots

```python
server.add_bip_network_port(instance=1, name="BIP Port", ip_address="192.0.2.10",
                            udp_port=47808, network_number=0, apdu_length=1476)
```

`add_bip_network_port` registers a configured, **unbound** IPV4/NORMAL application
snapshot. It does not inspect or configure the server's socket. The local Port ID
is its object instance, restricted to 1–255; UDP zero is independently allowed.
The constructor accepts keyword-only `ip_address`, `udp_port`, `network_number`,
`apdu_length`, `subnet_mask`, `default_gateway`, and `dns_servers`. Defaults use
zero addresses/mask/gateway and one zero DNS address for unavailable configuration;
an explicitly empty DNS list is invalid. Addresses must be IPv4 strings.
Network numbers range from 0 to 65534; zero has UNKNOWN quality, and explicitly
configured nonzero values have CONFIGURED quality. APDU_Length (399) is any
unsigned value at least 50, independent of Device property62's discrete limits.
The default 1476 is declared B/IP capacity, not a measured or verified live limit.

Port configuration and the derived six-octet IP/UDP MAC are read-only. Successful
BACnet configuration writes require pending-change activation under Clause12.56;
this snapshot has no activation owner and rejects those writes. Changes_Pending
remains false, Command and obsolete Network Port property62 are absent. Description
and Out_Of_Service retain their existing object-local behavior. This is not a live
port control or complete Network Port/BBMD/foreign/DHCP support claim. The pre-1.0
raw `add_network_port(network_type=...)` API is removed.

### Registered B/IP Network Port

This receiving-port association remains a bounded single NORMAL B/IP profile. Local Network Number behavior is described below; complete Network Port conformance, BBMD/foreign-device registration authority and multiport routing remain outside this registration contract.

### Local Network Number controls

Three Python owners consume the two local nonrouter controls automatically, one per transport: `BACnetServer`, the endpoint classes and `BACnetClient`. `BACnetServer` and `BACnetClient` own them with `transport="bip"` (NORMAL B/IP), `"ipv6"` (normal multicast B/IPv6), `"sc"` and `"mstp"`; `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` own them on their links. Python has no BBMD, foreign-device or Ethernet constructor, so the B/IP BBMD/foreign-device, B/IPv6 foreign-device and Ethernet paragraphs below describe Rust-only modes. A valid local unicast or broadcast What-Is-Network-Number receives a local-broadcast Network-Number-Is when the owner knows its number (Clauses 6.4.19–6.4.20). There is no proactive startup announcement: the Standard asks that of configured routers, not of every nonrouter node. An explicit registered Network Port with a nonzero configured number reports `CONFIGURED` and never replaces that number from an announcement. Zero starts `UNKNOWN`; an owner without registration also starts unknown, regardless of other declared objects.

A valid local-broadcast announcement with flag zero updates an unknown/learned owner to `LEARNED`. Flag one sets `LEARNED_CONFIGURED` and takes precedence over all subsequent flag-zero announcements. Further flag-one announcements may replace that learned value, including conflicts; an equal value still upgrades its quality. Both learned qualities transmit flag zero in their own responses. Selected-object `Network_Number` and `Network_Number_Quality` reads use the same database-owned state. Configuration remains immutable, so a new registration resets the pair from configured provenance; a new unregistered runtime starts unknown. There is no persistence of learned state across constructing a new runtime. After stop, the object retains the last observed pair until reconstruction or a new registration.

Routed controls, malformed payloads and unicast Network-Number-Is are ignored. A BBMD Forwarded-NPDU is a logical broadcast even when its UDP hop is unicast and remains eligible. Ignoring number zero, 65535 and flags outside zero/one is this implementation's validation policy, rather than an additional quoted Standard mandate. Conflicting announcements against a locally configured number produce a debug diagnostic without changing configuration.

Standalone clients start UNKNOWN on transports that opt into local nonrouter Number controls. They learn and reply using the same validation and precedence rules, without a Device object, registered Network Port, configured-number setter or persistence. One 256-entry serial worker owns this state; full or closed admission drops only Number controls. A held Number send leaves routed reason-4 Reject correlation and independent APDU dispatch available. Stop aborts and joins both the Number worker and dispatch before transport cleanup, retaining their joins across a canceled stop waiter. Drop aborts both. Already transmitted bytes cannot be retracted. Controlled-client tests qualify the shared intake/lifecycle behavior; Linux NORMAL-B/IP loopback and Ethernet virtual-link tests independently observe actual reply frames. Constrained-TLS SC tests observe Hub broadcast VMAC and exact Number bytes, including replies to direct-peer queries, while ordinary confirmed client requests complete. SC stop/drop retires client connections; the external DirectListener must separately be stopped and joined before its bind is released. Pending-send/queue cancellation remains covered by the generic controlled-client tests, rather than inferred from wire silence. Rust standalone-client B/IPv6 tests independently capture normal selected-link OriginalBroadcast and configured-foreign DBTN with exact source, destination, interface and Number bytes. Positive reply fences cover UNKNOWN, precedence and invalid/admission refusal; stop and eventual Drop release the socket, and reconstruction starts UNKNOWN. These external ignored Linux tests require the integration `ipv6` feature and isolated observer; ordinary hosted CI does not execute them. They add no Python foreign-device API, configured-client authority or physical-LAN claim. Separate isolated Linux standalone-client BBMD/foreign tests capture own Original-Broadcast versus forwarding traffic and exact DBTN to the configured BBMD. Positive Number fences cover UNKNOWN, BDT/FDT admission/refusal, alternate-sender compatibility, precedence and representative invalid/routed controls; registration NAKs retain DBTN attempts and the timer retries registration. An ordinary client ReadProperty completes during live Number controls, and awaited stop permits exclusive socket rebind before client Drop. No configured client number, new registration policy or complete Annex J claim is added. Rust MS/TP LoopbackSerial tests also cover the standalone client in both execution modes; see the MS/TP paragraph below.

Once `BACnetClient` has learned its network's number, a device its table holds as routed through a network with that number is on the client's own network (#1358). `read_property_from_device`, `write_property_to_device` and the other `_from_device` and `_to_device` methods send to it as a local request: a unicast to the device's own address with no DNET, not through the router it was heard through. An answer from that address completes the request, and so does one a router relays back with that number as its SNET and that address as its SADR (#1465): network numbers are unique, so both name the same device. While the number is unknown, these requests go through the router as before.

SC starts UNKNOWN with no configured SC Network Port API; unrelated configured objects provide no authority. SC logical broadcast is the BVLC broadcast destination VMAC. A direct unicast What-Is is valid, but its Number reply uses the Hub broadcast path, never the saved original-direct APDU response capability. Hub-relayed controls do not identify an originating TLS leaf; SC control-origin authorization remains separate (#518). A message from a direct-connection peer is never a logical broadcast, so it can ask but cannot teach.

B/IP BBMD and configured foreign modes start UNKNOWN with no registered Network Port authority. BBMD mode learns admitted Original-Broadcast, BDT Forwarded-NPDU and registered foreign-device DBTN announcements; its own Number reply is a local Original-Broadcast, which it also forwards as a Forwarded-NPDU to its BDT peers and registered foreign devices like its other broadcasts (#937). It rejects an exact self UDP source tuple before forwarded delivery or fanout, preserving the self BDT row and admitted peers on the same IP at different ports. Configured foreign mode accepts structurally valid Forwarded-NPDU from alternate UDP senders under its existing compatibility policy and answers by DBTN to its configured BBMD. Logical broadcast conveys no authenticated origin. Registration rejection does not suppress DBTN attempts; the existing periodic registration loop continues. Shared-endpoint Number wire tests cover BBMD ServerOnly admission and Original-Broadcast replies, foreign ClientOnly alternate forwarding and DBTN through registration rejection/retry, and Both requester/responder progress during live controls plus stop/drop socket release. Linux loopback supplies the independent BBMD broadcast capture; foreign direct capture also runs on macOS. Existing controlled endpoint tests separately prove held-send cancellation and resumed stop. Broader shared-endpoint BBMD/foreign behavior remains experimental; these modes add no configured Network Port authority.

B/IPv6 starts UNKNOWN with no configured IPv6 Network Port authority. Normal mode learns admitted OriginalBroadcast announcements and answers by multicast OriginalBroadcast on the selected link. In the Rust configured foreign-device mode, an admitted Forwarded-NPDU from the configured BBMD is a logical broadcast despite its unicast UDP hop; replies use DBTN to that BBMD. A different BBMD endpoint cannot teach. Unicast NNI, routed controls and malformed payloads remain ineligible. Existing selected-link, source-address, destination/interface and VMAC checks still apply. There is no new IPv6 endpoint builder, number setter or Python foreign-device API.

Linux Ethernet full servers and standalone Rust clients start UNKNOWN with no configured Ethernet Network Port authority. The AF_PACKET receive path admits only the bound MAC and all-FF broadcast, before UI, XID or TEST handling; only all-FF is a logical group. This is the local single-link admission policy, not an additional quoted Clause 7 mandate. Existing self-source refusal remains. Actual isolated Docker Ethernet tests independently inspect destination/source, 802.3 length, LLC bytes, exact learned reply flag zero and padding, and verify stop/drop raw-FD release plus canceled transport-stop resumption. No privileged host interface or physical LAN is involved. The [opt-in fixture](../crates/bacnet-integration-tests/tests/ethernet_network_numbers/README.md) requires Linux and CAP_NET_RAW and is excluded from ordinary CI. There is no Ethernet endpoint builder or Python Ethernet API in this slice.

MS/TP uses the same unregistered UNKNOWN state through the Python full server's `AnyTransport` adapter, `MstpEndpoint`'s shared session and the `transport="mstp"` client, with no configured MS/TP Network Port API. Rust LoopbackSerial tests cover the full-server, shared-endpoint and standalone-client owners in Tokio and DedicatedThread modes, independently decoding broadcast standard frames and checking application progress and cancellation. Installed Python regression checks preserve the existing construction surface; they do not establish physical serial or RS-485 timing qualification. See the [MS/TP Number evidence](rust-api.md#local-network-number-controls).

Control work has its own 256-entry receiver and serial worker, so a blocked learning operation or SC Hub write does not stop incoming APDU handling or already-admitted Audit acknowledgment dispatch. A blocked socket writer still serializes physical egress; this does not promise a second concurrent send. Stop seals, aborts and joins that worker before transport cleanup; cancellation retains cleanup ownership. Queued endpoint control sends are caller-owned and canceled with the worker, while a send already started may have reached the wire. The registered-object lease remains with the final socket and admitted work.

Tests cover actual inbound BVLL controls through both B/IP owners and outgoing NPDU observation after a successful real broadcast send, and separate tests cover the unchanged BVLL framing layer. The NORMAL B/IP owner fixture does not capture outgoing BVLL frames. Separate full-server BBMD/foreign tests independently decode actual loopback UDP: Linux BBMD tests distinguish own Original-Broadcast replies from Forwarded-NPDU fanout, and cross-platform foreign tests capture DBTN directly. Positive response fences cover refusal and recovery; held producer tests qualify APDU progress, canceled stop, drop and socket release. This loopback evidence does not qualify a physical LAN. Separate constrained-TLS SC fixtures independently decode actual broadcast-VMAC NNI bytes through both owners and `AnyTransport`; deterministic single-writer socket gates qualify bounded control queues, ACK/handler progress, cancellation and joined teardown. Already-admitted detached ordinary APDU sends retain their existing ownership; canceling Number work is not their retraction. Separate opt-in Linux tests capture full-server normal multicast and foreign DBTN bytes with an independent raw observer/BBMD, and a fresh installed Python extension qualifies normal multicast intake/output on the same isolated topology. These external-network tests are distinct from ordinary CI coverage. These controls do not establish a complete Network Port, Annex U/AB or router profile.


`BipEndpoint(..., network_port_instance=2, registered_network_port=2)` explicitly
associates its declared port 2 with one owned NORMAL B/IP transport. Omitting
`registered_network_port` leaves the declaration unbound. Selection requires a
concrete unicast `interface`, matching declared instances 1–255 and a matching
configured IP/UDP snapshot; `port=0` is supported. The full server accepts the same
`registered_network_port=2` constructor keyword, with the selected object supplied
separately by `add_bip_network_port(2, ..., ip_address=..., udp_port=...)` before
start. Declaring or adding an object alone never selects a live port.

Successful start publishes actual announced IP/UDP/derived MAC and `APDU_Length` (399) = 1476.
The selected configuration and Out_Of_Service are read-only while its lifetime
lease survives; pending activation, BBMD/foreign modes, rebind and same-device
multiport routing remain unsupported. Other configured rows remain unbound.

`BipEndpoint.local_address()` and `status()["local_address"]` report the same
active announced address and actual ephemeral port for **all** active B/IP
endpoints, including unregistered wildcard-interface defaults. They raise
`RuntimeError` before successful publication, while stopping, after failure and
after close. Queries ordered behind startup wait for its publication/cleanup;
creating a Future is not admission. SC/MS/TP address behavior is unchanged.

The seven File configuration methods are synchronous and operate only on a
pending built-in File before `start()`:

- `set_file_access_method(instance, access_method)` accepts only `"stream"` or
  `"record"`. Select the mode before loading its corresponding payload; a mode
  change preserves both stored channels and does not convert between them.
- `set_file_data` / `get_file_data` require stream mode.
- `set_file_records` / `get_file_records` require record mode.
- `set_max_file_size` and `set_max_record_count` return the effective value
  after the built-in File clamp. These are growth limits for later
  AtomicWriteFile requests; lowering a cap does not truncate preloaded content.

Inputs are copied. Getters return a fresh `bytes`, or a fresh `list` containing
fresh `bytes`, so later Python-side mutation cannot alter Rust storage.
Preloading is trusted local configuration and is independent of `Read_Only`;
network clients read and write runtime content through AtomicReadFile and
AtomicWriteFile, where `Read_Only`, access method, caps, and all-or-failure
behavior remain enforced. These methods do not mutate a live database and do
not persist File content across stop/restart.

### Lifecycle

```python
await server.start()
# Server is now responding to BACnet requests
address = await server.local_address()  # e.g., "0.0.0.0:47808"
await server.stop()
```

### Runtime Object Access

#### `read_property(object_id, property_id, array_index=None) -> PropertyValue`

Read a property from a local object through the server's ReadProperty
evaluator, the one network reads use, so the result equals what a network
`read_property` of the same property returns. A Group's Present_Value is
rebuilt from its members, Device instance `4194303` names this server's
Device, and the Device's Active_COV_Subscriptions lists the live
subscriptions. Its Device_Address_Binding lists the server's device bindings
(#1369): each `add_device_binding` and each device whose I-Am arrived in the
last ten minutes, one `"address_binding"` element each (see the typed
constructed values below). The value takes the
[read result](#read-results) shape. An
unknown object or property raises `BacnetProtocolError` with the error a
network read gets (`UNKNOWN_OBJECT`, for example). A Group whose member rows
exceed `rpm_max_result_elements` raises `BacnetAbortError` with
`reason == 9` (OUT_OF_RESOURCES), the abort a network read of it gets. The
server lock is held for the read, as for `write_property_local`.

```python
value = await server.read_property(
    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    PropertyIdentifier.PRESENT_VALUE,
)
print(value.value)  # 72.5
```

#### `write_property_local(object_id, property_id, value, priority=None, array_index=None, *, source_object)`

Write a property on a local object with a required keyword source. Explicit
`source_object=None` selects the server Device; an ObjectIdentifier names an
existing local initiating object. Six-family command-source writes require a
concrete server Device, and source corrections retain that Device as owner.
Unrelated local properties retain their no-Device behavior.

The value is encoded as a network WriteProperty would carry it and decoded
by the server's WriteProperty handler before the object sees it, so any
value a network client could write works locally, and whatever
`read_property` returns writes back: a Schedule's Effective_Period, a
Staging's Stages or `Stages[1]`, or `application_data` octets. An array index
is checked as over the network: a property the object doesn't hold raises
`UNKNOWN_PROPERTY`, one that isn't an array `PROPERTY_IS_NOT_AN_ARRAY`.
`PropertyValue.null()` on a property that isn't commandable and has no NULL in
its datatype succeeds and changes nothing, as it does over the network (#1396);
a read-only property still raises `WRITE_ACCESS_DENIED`.

```python
await server.write_property_local(
    ObjectIdentifier(ObjectType.ANALOG_VALUE, 1),
    PropertyIdentifier.PRESENT_VALUE,
    PropertyValue.real(73.0),
    source_object=None,
)
```

Local writes are trusted-local by design: `write_property_local` bypasses network
mutation authorization. The constructor's keyword-only
`mutation_policy="permissive"` selects the native default; `"deny_all"` denies
valid inbound property writes (WP/WPM), object creation/deletion, list additions/
removals, file writes, and the three COV subscription services through the same
Rust gate. Denials return SERVICES / SERVICE_REQUEST_DENIED, preserving WPM's
failed-reference error shape. It also denies each Channel write of an inbound
WriteGroup, silently, since nothing answers one. Remote reads and trusted local
writes still work.
No Python callback or duplicate gate is involved.

Invalid mode strings raise `ValueError`; non-strings raise `TypeError` during
construction, before startup drains registrations or performs I/O. DCC,
ReinitializeDevice, LifeSafety and Audit keep separate policies; this option
neither configures endpoint Device-write authorization nor identifies certificate
principals. See [Local mutation authorization](mutation-policy.md) for exact
service coverage, validation precedence and exclusions.

#### `purge_audit_log(object_id)`

Purge an Audit Log on a running server: clear its records and append a
BUFFER_PURGED status record, whether or not logging is enabled. The record
also carries LOG_DISABLED while logging is off. Total_Record_Count keeps
counting, and a confirmed notification already stored is still recognized
when it is sent again. AuditLogQuery returns no records afterwards, since it
returns notifications only; ReadRange shows the purge record. The purge
reaches the log's storage before the log serves it, and the server's other
requests carry on meanwhile. A notification batch lands whole on one side of
the purge, in the order the two reached the log.

An unknown object raises `BacnetProtocolError` with OBJECT / UNKNOWN_OBJECT,
any object other than an Audit Log OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
and a missing clock or a failed commit DEVICE / OPERATIONAL_PROBLEM; the log is
then left as it was. A server that is not running raises `RuntimeError`.

```python
await server.purge_audit_log(ObjectIdentifier(ObjectType.AUDIT_LOG, 1))
```

#### `comm_state() -> EnableDisable`

Get the server's current DeviceCommunicationControl state.

```python
state = await server.comm_state()
if state == EnableDisable.DISABLE_INITIATION:
    ...  # the server is holding back what it would start
```

The result is `EnableDisable.ENABLE` or `EnableDisable.DISABLE_INITIATION`,
the same class `device_communication_control` takes. The server refuses the
deprecated `DISABLE`, so `comm_state()` never returns it.

- `EnableDisable` doesn't compare equal to an `int`, and it is truthy in both
  states, so `if await server.comm_state():` can't tell them apart. Compare
  with the constants; `state.to_raw()` gives the number.
- Like every enum class here, it can be copied with `copy.copy` and
  `copy.deepcopy` and pickled; each rebuilds the value with `from_raw`.

`comm_state()` raises `RuntimeError` before start and after stop.

#### `cov_counters() -> CovCounters`

Sample the server's COV telemetry: a dict holding every field of the Rust
`CovCounters` under the same name, typed as the `CovCounters` TypedDict in the
stub. `subscriptions_active` is a gauge; the other fields are running totals
that start at zero on each `start()`. Each field is read on its own, so one
sample is not an atomic aggregate. Like `comm_state()`, it raises
`RuntimeError` before start and after stop.

```python
counters = await server.cov_counters()
counters["subscriptions_active"]          # subscriptions held now
counters["subscriptions_rejected_quota"]  # refused by the per-peer quota
counters["notifications_throttled_peer"]  # skipped at the confirmed in-flight limit
counters["timed_changes_dropped"]         # timestamped COV-multiple changes lost
```

| Field | Counts |
|---|---|
| `subscriptions_active` | Subscriptions held now |
| `subscriptions_created` | Subscriptions admitted (a renewal is not a new one) |
| `subscriptions_rejected_quota` | Refusals by the per-peer subscription quota |
| `subscriptions_rejected_capacity` | Refusals by the global or reserved capacity |
| `subscriptions_rejected_indefinite` | Refusals of indefinite lifetimes, by policy or the per-peer indefinite quota |
| `subscriptions_cancelled` | Subscriptions explicitly cancelled |
| `subscriptions_purged` | Expired subscriptions removed |
| `notifications_sent` | Notifications sent, confirmed and unconfirmed |
| `notifications_confirmed` | Confirmed notifications sent |
| `notifications_unconfirmed` | Unconfirmed notifications sent |
| `notification_bytes_sent` | APDU bytes of the notifications in `notifications_sent` |
| `notifications_throttled_fanout` | Notifications not sent because the per-event count or byte budget ran out |
| `notifications_throttled_peer` | Confirmed notifications not sent because the peer was at its in-flight limit |
| `timed_changes_dropped` | Timestamped COV-multiple changes discarded for good, in whole or in part, the running signal for a subscriber whose maximum APDU can't hold one timestamped value |
| `untimed_references_oversized` | Each COV-multiple report that left out an untimestamped reference too large for one notification |

The binding builds the dict from an exhaustive pattern over the Rust struct, so
a counter added in Rust must be added here before the bindings compile.

#### COV policy

The keyword-only `cov_policy` constructor argument sets the limits behind those
counters. It takes a dict, typed as the `CovPolicy` TypedDict in the stub, whose
keys are the fields of the Rust `CovPolicy` under the same names. A key left
out keeps its default, and `None` or `{}` gives the defaults. The policy is
copied at construction and applies to every later `start()`.

```python
server = BACnetServer(
    1234,
    cov_policy={
        "max_subscriptions_per_peer": 8,
        "allow_indefinite_subscriptions": False,
        "reserved_peers": [bytes([192, 168, 1, 20, 0xBA, 0xC0])],  # B/IP: IP then port
    },
)
```

| Key | Default | Unit | Limits | Counter it moves |
|---|---|---|---|---|
| `max_subscriptions_global` | 1024 | subscriptions | Positive. Subscriptions held across all peers | `subscriptions_rejected_capacity` |
| `max_subscriptions_per_peer` | 64 | subscriptions | Positive. Subscriptions one peer may hold | `subscriptions_rejected_quota` |
| `reserved_capacity` | 64 | subscriptions | Slots of the global cap kept for reserved peers, clamped to it. Has no effect while both reserved lists are empty; 0 keeps none | `subscriptions_rejected_capacity` |
| `reserved_peers` | `[]` | MAC `bytes` | Directly attached peers that may use the reserved slots; each MAC is 1 to 18 octets, the longest source the network layer delivers | |
| `reserved_recipients` | `[]` | `(network, bytes)` | As `dcc_source_restriction`: `None` for a local peer, otherwise its routed source network (1 to 65534), and a MAC of 1 to 18 octets | |
| `allow_indefinite_subscriptions` | `True` | `bool` | Whether a subscription without a lifetime is admitted | `subscriptions_rejected_indefinite` |
| `max_indefinite_per_peer` | 16 | subscriptions | Indefinite subscriptions one peer may hold, clamped to its per-peer cap; 0 admits none | `subscriptions_rejected_indefinite` |
| `max_notifications_per_event` | 64 | notifications | Positive. Notifications one change of a monitored object may send | `notifications_throttled_fanout` |
| `max_notification_bytes_per_event` | 65536 | APDU bytes | Positive. Bytes those notifications may total | `notifications_throttled_fanout` |
| `max_confirmed_in_flight_per_peer` | 16 | notifications | Positive. Confirmed notifications awaiting one peer's acknowledgment | `notifications_throttled_peer` |

The quotas count a directly attached peer by its MAC, and a peer behind a router
by its routed source network and MAC, so only `reserved_recipients` can reserve
slots for the latter. A request is checked against its peer's quota before the
global cap.

The constructor checks the dict before any I/O, with the exception types the
other keywords use: an unknown or non-`str` key, or a value of the wrong type,
raises `TypeError`, a negative or oversized integer `OverflowError`, and a value
the Rust `CovPolicy::validate` refuses `ValueError`. The Rust server runs the
same check before it starts a transport. The conversion names every field of
the Rust struct without `..`, so a field added in Rust must get a key here
before the bindings compile.

#### Time synchronization policy

The keyword-only `time_sync_policy` constructor argument limits which
TimeSynchronization and UTCTimeSynchronization requests set the Device clock.
It takes a dict, typed as the `TimeSyncPolicy` TypedDict in the stub, whose
keys are the fields of the Rust `TimeSyncPolicy`, with `_ms` on the durations.
A key left out keeps its default, and `None` or `{}` gives the default, which
sets the clock from every valid request. The policy is copied at construction
and applies to every later `start()`.

```python
server = BACnetServer(
    1234,
    time_sync_policy={
        "source_restriction": [(None, bytes([192, 168, 1, 10, 0xBA, 0xC0]))],  # B/IP: IP then port
        "max_step_ms": 300_000,   # refuse a correction of more than five minutes
        "global_rate": (0.2, 2),  # (max_per_second, burst_capacity)
    },
)
```

| Key | Default | Limits |
|---|---|---|
| `enabled` | `True` | `False` refuses every request |
| `source_restriction` | `None` | As `dcc_source_restriction`: `(None, mac)` for a direct source, `(network, address)` for a routed one; at most 256 entries of 1 to 18 octets, networks 1 to 65534. `[]` refuses every source |
| `max_step_ms` | `None` | Largest correction, forward or back; 0 allows only an exact match |
| `per_source_rate` | `None` | `(max_per_second, burst_capacity)`: rate positive and finite, burst positive |
| `global_rate` | `None` | The same, for all sources together |
| `coalesce_window_ms` | 0 | Least time between accepted requests from one source; 0 is off |
| `global_coalesce_window_ms` | 0 | Least time between accepted requests from any source; 0 is off |
| `max_sources` | 256 | Sources the rate and coalescing state tracks, 1 to 65536 |

The constructor checks the dict before any I/O: an unknown or non-`str` key, or
a value of the wrong type, raises `TypeError`, a negative or oversized integer
`OverflowError`, and a limit in the table `ValueError`. Sources are claimed
addresses, not authenticated identities. See
[time synchronization policy](time-sync-policy.md) for the order of the checks
and how each limit behaves.

#### `event_notification_counters() -> EventNotificationCounters`

Sample the totals of event notifications the server did not deliver: a dict
holding every field of the Rust `EventNotificationCounters` under the same name,
typed as the `EventNotificationCounters` TypedDict in the stub. Every field is a
running total that starts at zero on each `start()` and saturates at 2**64-1.
Each field is read on its own, so one sample is not an atomic aggregate. Like
`cov_counters()`, it raises `RuntimeError` before start and after stop. Each
counted refusal also logs a warning; the counters are the running signal.

```python
counters = await server.event_notification_counters()
counters["notification_class_missing"]  # transitions whose class doesn't exist
counters["confirmed_unanswered"]        # confirmed notifications never acknowledged
```

| Field | Counts |
|---|---|
| `notification_class_missing` | Transitions sent nowhere because no Notification Class object has the class number the event object names |
| `recipient_list_unavailable` | Transitions sent nowhere because reading the class's Recipient_List failed |
| `recipient_list_invalid` | Transitions sent nowhere because the Recipient_List did not decode as a whole (no decodable prefix is used) |
| `recipient_list_too_long` | Transitions sent nowhere because a custom class served more than 32 destinations |
| `device_recipient_unbound` | Matched Device recipients skipped because no binding was configured or observed, or the observed one expired, and a targeted Who-Is drew no I-Am within the APDU timeout (or none could go out, within a minute of one that drew nothing); also a confirmed notification ended at a retry because the observed binding had expired by then |
| `recipient_unroutable` | Matched recipients skipped because they can't be routed as written: a Device identifier that isn't a Device (or a binding unusable on this link), or a MAC on network 65535 |
| `confirmed_broadcast_recipient` | Matched recipients skipped because they ask for confirmed notifications at a broadcast address, or another group address such as a multicast one, which only unconfirmed requests may use (Clause 6.3) |
| `confirmed_no_invoke_id` | Confirmed notifications to one recipient not sent because no invoke ID was free |
| `confirmed_rejected` | Confirmed notifications the recipient answered with an Error, Reject or Abort |
| `confirmed_unanswered` | Confirmed notifications with no acknowledgment after the last retry |
| `unconfirmed_send_failed` | Unconfirmed notifications the transport refused to send, once per destination; the other destinations are still served |
| `apdu_too_large` | Notifications not sent to one destination because they exceed the local APDU size (notifications are never segmented); usually a forwarded copy of one that arrived segmented |
| `received_not_forwarded` | Received event notifications that decoded but that no Notification Forwarder took; a confirmed one is still acknowledged |
| `forwarding_cap_dropped` | Destinations a notification was not forwarded to because 64 copies were already on their way across the Notification Forwarders, one per destination dropped; destinations the loop rules refuse take no room |
| `received_not_logged` | Received event notifications an Event Log collecting them would have taken, but their source had used its 5 records for that second; one per notification |

The first four count event and acknowledgment notifications alike, once per
transition. A class whose list is empty, or whose destinations all filter the
transition out by day, time or transition, is configured behaviour and is not
counted. Neither are notifications held back by DeviceCommunicationControl or
Event_Enable, nor confirmed reservations refused while the server stops. The
three route fields (#1160) count once per skipped destination, and the
transition's other destinations are still served; the warning logged with each
skip gives the finer reason. `unconfirmed_send_failed` (#1196) counts once
per destination whose send fails; no field counts an encode failure, since a
well-formed transition always encodes. The binding builds the dict from an exhaustive
pattern over the Rust struct, like `cov_counters()`.

#### `forwarder_save_counters() -> dict[int, ForwarderSaveCounters]`

Sample each Notification Forwarder's save counters: a
dict keyed by forwarder instance whose values are `ForwarderSaveCounters`
dicts with one field, `failed_saves`. It counts the saves the `storage_path`
file refused: a write that needed one (and failed with DEVICE /
OPERATIONAL_PROBLEM) and a periodic save the server retries a minute later.
The totals belong to the objects, so they count from registration, saturate at
2**64-1, and stay zero for a forwarder without `storage_path`. Like
`cov_counters()`, it raises `RuntimeError` before start and after stop.

```python
counters = await server.forwarder_save_counters()
counters[1]["failed_saves"]  # refused saves of forwarder 1's lists
```

#### `local_address() -> str`

Get the server's bound address after start.

```python
addr = await server.local_address()  # "0.0.0.0:47808"
```

---

## Exceptions

All BACnet errors are raised as Python exceptions:

| Exception | Meaning |
|-----------|---------|
| `BacnetError` | Base exception for all BACnet errors |
| `BacnetProtocolError` | A remote or local BACnet protocol check returned an error (class + code) |
| `BacnetTimeoutError` | Request timed out (APDU retries exhausted) |
| `BacnetRejectError` | Remote device rejected the request |
| `BacnetAbortError` | Remote device aborted the request |
| `BacnetReadRangeViolationError` | A strict `read_range` refused an answer that breaks a ReadRange rule; `rule` names it (for example `"zero_first_sequence_number"`) |
| `BacnetLogNotAdvancingError` | A paged log read can't advance: the device answered from before the record asked for, or with none its counts say it holds. `requested` and `returned` (`None` when it sent none) name the sequence numbers |
| `BacnetTransportError` | A socket or I/O failure: a bind or listen that fails (B/IP, B/IPv6, `ScHub.start`), a failed SC dial. Also an `OSError` |

```python
from rusty_bacnet import (
    BacnetError, BacnetProtocolError, BacnetTimeoutError,
    BacnetRejectError, BacnetAbortError, BacnetTransportError,
)

try:
    value = await client.read_property(addr, oid, pid)
except BacnetTimeoutError:
    print("Device not responding")
except BacnetProtocolError as e:
    print(f"Protocol error: class={e.error_class}, code={e.error_code}")
except BacnetRejectError as e:
    print(f"Rejected: reason={e.reason}")
except BacnetAbortError as e:
    print(f"Aborted: reason={e.reason}")
except BacnetError as e:
    print(f"BACnet error: {e}")
```

`BacnetProtocolError` has `error_class` and `error_code` integer attributes. Some services answer with a structured error body (Clause 21) that adds fields; each attribute below is `None` unless the device's error carried it:

| Attribute | Set by | Value |
|-----------|--------|-------|
| `first_failed_element_number` | AddListElement/RemoveListElement ChangeList-Error, CreateObject-Error; a server's `write_local` of a list an object refuses one element of | Position, from 1, of the list element or initial value that failed; 0 when the request failed for another reason. From `write_local`, the element's position in the list written |
| `first_failed_write_attempt` | WritePropertyMultiple-Error | `{"object_identifier", "property_identifier", "property_array_index"}` of the first write that failed |
| `first_failed_subscription` | SubscribeCOVPropertyMultiple-Error about one COV reference | The same dict for the refused reference; `None` for a general failure |
| `vendor_id`, `service_number` | ConfirmedPrivateTransfer-Error | The private service the error answers |
| `error_parameters` | ConfirmedPrivateTransfer-Error | Encoded vendor-defined error parameters (`bytes`), when present |
| `vt_session_identifiers` | VT-Close error that lists them | Local identifiers of the sessions that could not be closed |

`BacnetRejectError` and `BacnetAbortError` have a `reason` integer attribute.

`BacnetTransportError` derives from both `BacnetError` and `OSError`, so either `except` clause catches it. Its `errno` is the operating system's code when it reported one, otherwise the code for the failure's kind where one exists (`None` if not), and `strerror` is the message:

```python
import errno

try:
    await hub.start()
except OSError as e:
    if e.errno == errno.EADDRINUSE:
        print("port already in use")
    else:
        raise
```

`errno` is the operating system's own code, so platforms differ: Windows can refuse a bind to a UDP port another socket holds with `errno.WSAEACCES` rather than `errno.EADDRINUSE`, so check both there. It is not narrowed to the `OSError` subclasses (`ConnectionRefusedError` and the like), so compare `errno` rather than the class. `list_serial_ports()` raises a plain `OSError` instead.

---

## Complete Example

```python
import asyncio
from rusty_bacnet import (
    BACnetClient, BACnetServer,
    ObjectType, ObjectIdentifier, PropertyIdentifier, PropertyValue,
    EnableDisable, ReinitializedState,
)

async def server_example():
    """Run a BACnet server with some objects."""
    server = BACnetServer(device_instance=1234, device_name="Test Device")
    server.add_analog_input(instance=1, name="Zone Temp", units=62, present_value=72.5)
    server.add_binary_value(instance=1, name="Override")
    await server.start()
    print(f"Server running at {await server.local_address()}")

    # Publish an application-owned Input sample at runtime
    await server.set_present_value_local(
        ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
        PropertyValue.real(73.0),
    )

    await asyncio.sleep(60)
    await server.stop()

async def client_example():
    """Read and write to a remote BACnet device."""
    async with BACnetClient() as client:
        addr = "192.168.1.100:47808"
        ai1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

        # Discover devices
        await client.who_is()
        await asyncio.sleep(2)
        for dev in await client.discovered_devices():
            print(f"Found device {dev.object_identifier.instance}")

        # Read single property
        value = await client.read_property(addr, ai1, PropertyIdentifier.PRESENT_VALUE)
        print(f"Temperature: {value.value}")

        # Read multiple properties at once
        results = await client.read_property_multiple(addr, [
            (ai1, [
                (PropertyIdentifier.PRESENT_VALUE, None),
                (PropertyIdentifier.OBJECT_NAME, None),
                (PropertyIdentifier.UNITS, None),
            ]),
        ])
        for obj in results:
            for prop in obj["results"]:
                if prop["value"]:
                    print(f"  {prop['property_id']}: {prop['value'].value}")

        # Subscribe to COV notifications
        await client.subscribe_cov(addr, 1, ai1, confirmed=True, lifetime=300)

        # Listen for changes (in a separate task)
        async def listen():
            notifications = await client.cov_notifications()
            async for notif in notifications:
                for v in notif.values:
                    print(f"COV: {v['property_id']} = {v['value']}")

        listener = asyncio.create_task(listen())
        await asyncio.sleep(30)
        listener.cancel()

asyncio.run(client_example())
```

---

## Endpoint (one transport, both roles)

`BipEndpoint`, `ScEndpoint`, and `MstpEndpoint` own one transport above both
sibling roles. Each exposes `await endpoint.client()` (`EndpointClient`) and
`await endpoint.server()` (`EndpointServer`); roles hold no lifecycle and fail
closed with `BacnetError` after the owner closes. Builder-time config only:
`DeviceIdentity` (instance/vendor/APDU/segmentation/services/ports/UUID) is
validated in the constructor; there is no post-start owner mutation.

```python
endpoint = BipEndpoint(device_instance=1001, vendor_id=42, port=47808)
endpoint.add_analog_input(instance=1, name="Zone Temp", present_value=21.5)
async with endpoint:
    client = await endpoint.client()
    value = await client.read_property("127.0.0.1:47808", oid, pid)
```

`await endpoint.status()` returns a redacted snapshot (`is_running`,
`device_instance`, `vendor_id`, `max_apdu`, `transport`, `local_address`,
`active_leases`, plus policy counters). `local_address()` is `"ip:port"`
(BIP), VMAC hex (SC), or station string (MS/TP). Roles expose no callbacks;
the client initiates `read_property`, `read_range` and `read_property_multiple`; concurrent use is
`asyncio.gather` over these reads plus
`is_session_alive()` polling — never Rust-calls-Python. Interpreter
finalization only seals forcefully; always await `close()` or context exit.
BIPv6/Ethernet have no endpoint owner; use the standalone path there.

Python endpoint server roles execute and advertise ReadProperty only (the Rust
Device-write and ReinitializeDevice opt-ins have no Python binding) and expose
neither Device COV list property. A supplied `services` list must include RP and
cannot claim unsupported execution; incompatible server-role profiles fail
startup before ingress.
ClientOnly has no responder and keeps its service list as a local declaration.

### Endpoint lifecycle and cancellation

For B/IP, SC and MS/TP, `start()`, `close()` and context exit return native
`Awaitable[None]` Futures whose successful awaited value is Python `None`.
An explicit second start while running raises `BacnetError`; context reentry
returns the same endpoint. After awaited close, a new start can create a new
session. Registrations consumed by a successful start are not replayed on restart.

Admission occurs when the Rust operation acquires the endpoint lifecycle lock,
not when Python creates its Future. Competing start/context-entry calls cannot
replace a live session or acquire a second transport. TLS file loading, dialing
and serial open now occur after admission; their setup errors arrive when the
Future is awaited, rather than synchronously from the method call. Constructor
validation remains synchronous.

When polled, close requests cancellation of connection preparation ahead of its
lifecycle admission and then waits for ownership. That request remains visible
until admission or cancellation of the close waiter; it cannot cancel a later
restart after close finishes. A start interrupted there
raises `RuntimeError("endpoint startup cancelled by close")`. After transport
session startup begins, the owner lets it settle, then joins teardown before
close returns. This avoids abandoning partially started ingress. Close therefore
waits for that startup/teardown work, including the transport's existing handshake
limits; there is no new universal close timeout. Later admitted startup is a new
operation and may run after close.

Cancelling a Python startup waiter cancels safe connection preparation. If session
startup has begun, its worker retains ownership and finishes startup and cleanup.
Cancellation can race successful publication, so cancelling alone does not prove
that no resource was acquired or that the endpoint is stopped. Always await
`close()` (or context exit) when cleanup matters. If a close waiter is cancelled
after admission, cleanup continues under the owner; another awaited close joins
that work. Context exit does not suppress exceptions from the context body.

Synchronous `add_*` registrations are rejected while starting, running or stopping.
Failed startup, or cancellation observed before successful publication, restores
the exact pending registrations after owned resources have been released. The
registration gate reopens after terminal cleanup. Role access and status share the
lifecycle lock, so they cannot observe a half-published session.

### Endpoint Groups and the read work limit

`add_group(instance, name, members=None)` registers a Group before start.
`members` uses the `read_property_multiple` spec shape,
`[(object_id, [(property_id, array_index), ...]), ...]`, and the server role
rebuilds Present_Value from it on every read, one result per member. A member
with an empty property list, or one that reports a Group or Global Group's
Present_Value, raises `ValueError` when it is added; the message names the
member's position and the rule it breaks. Any property identifier is taken,
including those ASHRAE assigns above 4194303, such as `DEFAULT_COLOR`. As for `read_property_multiple` specs,
an array index outside unsigned32 raises `OverflowError` during conversion.

Each owner takes a keyword-only `read_work_limit=256`: the result rows one
ReadProperty served by the server role may expand. A read counts its own row,
and a Group's Present_Value adds a row for every member property after ALL,
REQUIRED and OPTIONAL expand. A read over the limit is answered with an Abort
carrying OUT_OF_RESOURCES (a peer `BACnetClient` raises `BacnetAbortError` with
`reason == 9`) before any member is read. Zero raises `ValueError`, negative or
native-overflow values raise `OverflowError`, all in the constructor. It is the
endpoint counterpart of `BACnetServer`'s `rpm_max_result_elements`; see
[RPM budgets](rpm-budget.md).

```python
endpoint = BipEndpoint(device_instance=1001, port=0, read_work_limit=2)
endpoint.add_analog_input(instance=1, name="Zone Temp")
ai = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
# Present_Value and Object_Name of AI 1: three rows, over the limit of 2.
endpoint.add_group(1, "Zone", [(ai, [(PropertyIdentifier.PRESENT_VALUE, None),
                                     (PropertyIdentifier.OBJECT_NAME, None)])])
```

### Endpoint ReadPropertyMultiple

`await client.read_property_multiple(address, specs)` shares standalone RPM's
ordered list of object dictionaries (`object_id`, `results`). Each result has
`property_id`, `array_index`, `value` (`PropertyValue`, raw `bytes`, or `None`) and
`error` (an `ErrorClass`/`ErrorCode` tuple or `None`). Duplicate occurrences remain
separate. Successful values echo the requested index; an inline error may omit
it. Entire ACK identity/order/count correlation precedes returning results.

The endpoint accepts 1–64 explicit references across nonempty object lists and
concrete identifiers. Empty lists, ALL/REQUIRED/OPTIONAL and wildcard object
instances raise `ValueError` before address parsing/I/O. Index zero is valid;
indexes outside native unsigned32 raise `OverflowError` during conversion.
Requests and raw ACKs must fit the configured unsegmented max APDU. Protocol,
size, malformed/mismatched ACK and closed-owner failures raise `BacnetError`.
Standalone RPM retains its broader selector profile but now rejects empty lists
through the shared fallible encoder. Endpoint source Reporter configuration is
still Rust-only; this method does not expose it in Python. The bundled server's
known-scalar error-index producer correction remains separate (#789).

### Migration: separately-constructed client/server to endpoint

Two separately constructed objects (`BACnetClient(...)` + `BACnetServer(...)`
used together) are documented as two connections: two sockets/ports (BIP),
two hub sessions (SC), or two serial opens (MS/TP, which cannot share one
port). The endpoint is the one-transport path.

| Old standalone | Endpoint equivalent |
|----------------|---------------------|
| `BACnetClient()` + `BACnetServer(device_instance=..., ...)` on different ports | `BipEndpoint(device_instance=..., vendor_id=..., port=...)` with `add_*` registrations, one `start()` |
| SC client + SC server with separate `sc_device_uuid` values | `ScEndpoint(..., sc_device_uuid=...)` — one UUID for dial + DEVICE_UUID |
| MS/TP client + server with separate `serial_port` opens | `MstpEndpoint(..., serial_port=...)` — one serial owner |

Old constructors are unchanged (zero behavior change). New code uses the
endpoint builders; see `examples/python/endpoint_bip.py` for a runnable
receive + initiate flow through one endpoint.

---


## Hub certificate bindings

```python
from rusty_bacnet import ScHub, ScHubCertificateBinding

node = ScHubCertificateBinding(
    uuid=node_uuid,
    allowed_vmacs=(port_vmac,),
    leaf_sha256=(current_leaf_sha256, renewal_leaf_sha256),
)
hub = ScHub(
    "127.0.0.1:0", "hub.pem", "hub.key", hub_vmac,
    ca_cert="site-ca.pem", device_uuid=hub_uuid,
    certificate_bindings=(node,),
)
```

Each frozen group's constructor is keyword-only. UUID, VMAC and leaf digest
lengths are 16, 6 and 32 bytes respectively. UUID must be nonzero; VMACs must not
be all zero/all ff. Both lists are nonempty and distinct. Read-only getters return
bytes or tuples of bytes; input sequences are copied and `repr` is redacted.
Compute fingerprints as `hashlib.sha256(leaf_der).digest()`, not from PEM text or
the public key. Multiple leaf digests explicitly permit certificate rotation.

`certificate_bindings=None` retains CA-valid admission. A configured sequence
must be nonempty, with no UUID, VMAC or fingerprint overlap between groups and
no allowed VMAC equal to the Hub's own VMAC. Shape/ownership failures raise
`ValueError` synchronously before PEM I/O or bind, using the native validator.
Tuple triples/dictionaries are not alternate group representations. The Hub
copies the validated policy; later edits to input lists cannot change it.

Bound mode rejects unmapped leaves, wrong UUIDs and unlisted VMACs, including
claims on offline or otherwise unreserved identities. Existing `admission_policy`
remains conjunctive: `allow_all` cannot override binding refusal, while
`deny_uuid_replacement` can refuse even a listed renewal. Rejection leaves an
incumbent intact. `admin_denied` includes binding denials without certificate
contents in status/errors.

This is explicit installation policy under Annex AB.7.4. It does not authenticate
relayed operations end to end, authorize operations, or secure direct SC ingress.
Every Hub feeding a trusted router ingress must enforce the selected policy;
#518/#524 remain separate. See the [native contract](rust-api.md#hub-certificate-bindings).

## ScHub

BACnet/SC Hub — a TLS WebSocket relay for BACnet Secure Connect. Both `BACnetClient` and `BACnetServer` with `transport="sc"` connect to a hub as clients. The hub relays messages between connected nodes using VMAC addresses.

### Constructor

```python
from rusty_bacnet import ScHub
from uuid import UUID
import os

hub = ScHub(
    listen="127.0.0.1:0",       # Bind address (port 0 = auto-assign)
    cert="hub-cert.pem",         # Server TLS certificate
    key="hub-key.pem",           # Server TLS private key
    vmac=b"\xff\x00\x00\x00\x00\x01",  # Hub's 6-byte VMAC
    ca_cert="ca-cert.pem",       # Trusted issuer CA for mutual TLS
    device_uuid=UUID(os.environ["SC_HUB_DEVICE_UUID"]).bytes,
)
```

`ca_cert` is required: omission, `None`, or an empty string raises `ValueError`
at construction, before listening. Its fifth positional slot and `None` default
remain for argument-layout compatibility only; there is no server-auth-only
mode or insecure flag. Existing callers must supply the trusted issuer CA PEM
file, not a peer leaf certificate. Configure each SC node with its own
operational certificate/key pair and the CA that signs the hub certificate.

**Hub identity migration:** `device_uuid` is a new required keyword-only option.
Its `None` signature default preserves CA-first diagnostics, not a fallback UUID.
Missing/None, lengths other than 16 bytes, and all-zero UUIDs raise `ValueError`.
`bytes` and `bytearray` are copied into an owned 16-byte array; mutating the source
cannot change identity. The existing five positional slots remain unchanged.
After CA presence, the constructor checks VMAC length (the existing `RuntimeError`),
reserved all-zero/all-ff VMACs (`ValueError`), then UUID, all before file I/O/bind.
No additional VMAC bit-shape or UUID version/variant policy is imposed.

Admission and timeout policy is also constructor-validated, before bind, via
keyword-only options (positional layout unchanged):

```python
hub = ScHub(
    listen="127.0.0.1:0",
    cert="hub-cert.pem", key="hub-key.pem",
    ca_cert="ca-cert.pem",
    vmac=b"\xff\x00\x00\x00\x00\x01",
    device_uuid=hub_uuid,
    max_clients=256,              # registered-client cap (zero/overflow: ValueError)
    max_handshakes=256,           # pre-handshake cap (same error mapping)
    admission_policy="allow_all", # also "deny_all" or "deny_uuid_replacement"
    graceful_disconnect_ack_ms=5000,   # per-peer Disconnect-Ack budget
    graceful_ws_close_ms=5000,         # per-peer AB.7.5.5 close budget
    graceful_overall_ms=15000,         # whole-drain bound (must cover ack + close)
    handshake_tls_ms=10000,            # TCP-admission to TLS handshake budget
    handshake_websocket_upgrade_ms=10000,
    handshake_connect_request_ms=10000,  # 5s minimum per Annex AB.6.2.3
    probe_scan_interval_ms=30000, probe_idle_age_ms=60000,
    probe_ack_age_ms=5000, probe_send_budget_ms=5000,
    relay_send_budget_ms=5000,            # each transit relay acquisition + send
    broadcast_sender_burst=1024, broadcast_sender_per_second=128,
    broadcast_global_burst=4096, broadcast_global_per_second=512,
)
```

Out-of-range durations raise `ValueError` mirroring the native
`ScHubGracefulTimeouts::new` / `ScHubHandshakeTimeouts::new` errors;
negative integers raise `OverflowError`. `admission_policy` is a static string
only: `"allow_all"` (default), `"deny_all"`, or `"deny_uuid_replacement"`.
The last mode refuses an incoming connection that would replace an incumbent
with the same claimed UUID, at either the same or a moved VMAC. It evaluates the
current native registration classification under the same registry lock; no
Python callback or second registration map is involved. This is an explicit
local security policy before protocol acceptance. Default mode retains Annex AB
known-UUID acceptance/replacement; different-UUID VMAC conflicts still receive
the standard duplicate-VMAC NAK. No certificate-to-UUID identity is inferred.
Both denial modes use the existing `RESOURCES`/`OTHER` NAK family and increment
`admin_denied` when either the certificate binding or admin policy denies. Unknown mode strings raise `ValueError`
at construction, before credential I/O or binding. Non-string values — including Python callables — raise
`TypeError`: the native policy runs synchronously under the registry lock,
where attaching the GIL could deadlock, so no Python callback can be
installed. There are no deny-lists, issuance/rotation orchestration, or
distributed-admin features.

The `probe_*_ms` settings configure optional accepting-Hub probes, not the
initiating node's Annex AB keepalive. They share one native monotonic clock.
Idle and pending-ACK ages must be strictly exceeded at a scan; ACK age begins at
reservation before sending. Delayed scans are skipped, and serial sends can
postpone detection, so `probe_ack_age_ms` is not a hard closure deadline. A valid
matching ACK clears pending and refreshes activity; wrong/invalid ACKs do not.
`probe_send_budget_ms` includes sink acquisition and send.
`relay_send_budget_ms` independently bounds each transit acquisition-plus-send
attempt: NPDU/opaque unicast, each concurrent broadcast recipient, and forwarded
BVLC-Result. Timeout does not retire, retry, or fabricate a Result and cannot
retract buffered bytes. Healthy broadcast recipients progress independently.
Probe, control, cleanup and graceful-shutdown policies remain separate. The
pre-1.0 unicast-only keyword is removed without an alias; use `relay_send_budget_ms`.
Outcome counters retain their documented unicast-only scope.

Probe and relay values are positive integers in milliseconds, at most
`2**63 - 1` with platform monotonic representability also checked. Zero/excessive
values raise `ValueError`; negative/out-of-u64 integers raise `OverflowError`;
nonintegers raise `TypeError`. These are local representation bounds, not
normative BACnet probe ranges. Broadcast burst/refill fields use the existing
native rate policy with bounds `1..=(2**64 - 1)//1_000_000_000`; one token admits
one broadcast request, not one recipient. Exhaustion silently drops and updates
`broadcast_sender_exhausted` or `broadcast_global_exhausted`. All these settings
are validated in the constructor before file I/O or bind. Installed mutual-TLS
tests cover the probe and the rate and counter settings.

The UUID identifies the hosting **device**, while VMAC identifies its hosting
**port**; Connect-Accept carries their exact configured bytes (base 2020 AB.2.11,
AB.6). The caller must provision the UUID before deployment and durably reuse it
for the device's entire lifetime (AB.1.5.3), including same-object stop/start and
fresh objects. No per-connection generation, storage backend, lifetime-history
check, or automatic certificate binding is provided. See [UUID provisioning](#sc-device-uuid-migration).

`start()` validates the CA store and server certificate/key before binding;
unreadable, empty, or malformed credentials and mismatched server keys raise
`BacnetError`. The hub accepts only TLS 1.3 with verified client certificates.
Missing, untrusted, expired, or not-yet-valid peer certificates are rejected.
This addresses the Python hub admission boundary of Annex AB.7.4, not full
security-profile conformance. CA membership does not authorize BACnet operations
or by itself bind a certificate to a claimed VMAC/Device UUID; the optional
`certificate_bindings` policy provides the separate registration check. Python startup loads/parses
files and delegates policy construction to native `ScHubTlsConfig::from_der`, then
uses `ScHub::start_with_tls_config` with the retained explicit UUID and default phase
timeouts. The identity migration is not another certificate-less-access fix;
required CA, credential error categories, file repair/retry and
async lifecycle remain unchanged. Native construction does not itself load files
or certify local certificate dates/issuer relationships.

All public Rust hub startup now requires the [constrained configuration](rust-api.md#bacnetsc-hub);
raw `TlsAcceptor` hub injection is retired by a Rust source-breaking change.
Python uses that alias and now passes its required owned hub identity.
Built-in Rust node APIs now require `ScNodeTlsConfig` too. This is an internal
Python integration, not another certificate-less-access fix or a signature change.
Issue #513 remains open for final acceptance assessment and remaining profile
limits. Python nodes require the
explicit credentials described [below](#bacnetsc-secure-connect).

### Methods

#### `start()`

Start the hub. Begins accepting WebSocket connections.

```python
await hub.start()
```

#### `stop()`

Forcefully stop the hub: seals admission and aborts workers without the
Disconnect exchange. Idempotent — stopping a stopped or never-started hub is
a safe no-op.

```python
await hub.stop()
```

#### `shutdown_gracefully() -> "graceful" | "forced"`

Bounded graceful shutdown: seals admission first, then drives the
hub-initiated Disconnect-Request, awaited Disconnect-Ack, and AB.7.5.5
WebSocket close handshake per established peer within the configured graceful
bounds, with a forceful fallback on expiry. Returns `"graceful"` only when
every peer completed the exchange, `"forced"` otherwise. Consumes the running
hub: a second call raises `RuntimeError` (use `stop()` for an idempotent
close). Cancelling the future leaves shutdown running; `stop()` stays safe
to call afterwards.

```python
outcome = await hub.shutdown_gracefully()  # "graceful" or "forced"
```

#### `status() -> ScHubStatus`

Bounded redacted snapshot: listener state, handshake/client counts, and
deny/drop counters. Counts and kind labels only — no certificates, keys,
VMAC maps, or payloads. Raises `RuntimeError` before start and after stop.

```python
status = await hub.status()
# {"listening": True, "max_clients": 256, "max_handshakes": 256,
#  "client_count": 1, "handshake_count": 0, "admin_denied": 0,
#  "broadcast_sender_exhausted": 0, "broadcast_global_exhausted": 0,
#  "outcomes": {...}}
print(status["outcomes"]["unicast_no_target"])
```

The nested `ScHubOutcomeCounts` typed dictionary has thirteen integer fields:
`uuid_replacements`, `vmac_collision_rejections`,
`registered_capacity_rejections`, `total_active_accept_drops`,
`handshake_accept_drops`, `tls_timeouts`, `websocket_timeouts`, `connect_timeouts`,
`unicast_no_target`, `unicast_target_limit`, `unicast_send_timeout`,
`unicast_send_error`, and `heartbeat_retirements`.
All saturate at `u64::MAX` and are independent for each started Hub. Refusals
count selected decisions, not delivered NAKs; a later Connect timeout may also
count. Unicast outcomes aggregate eligible NPDU and addressed opaque traffic;
malformed, pre-registration, stale-source, self/local, broadcast, and forwarded
Result paths are excluded. Only actual committed replacement or matching-generation
heartbeat removal counts. No send-success inference, payloads, peer identifiers,
or raw errors are retained. Fields are sampled independently and never drive
policy. Existing admin/broadcast counters and Python status lifecycle are unchanged.

#### Context manager

`async with` starts the hub on entry and forcefully stops it on exit, even
if the body raised. Exiting twice, or after an explicit `stop()`, is safe.

```python
async with ScHub(..., device_uuid=hub_uuid) as hub:
    ...
```

Dropping the hub without awaiting `stop()`, `shutdown_gracefully()`, or
context-manager exit only requests forceful-seal cleanup on a running
runtime and cannot guarantee awaited close.

#### `address() -> str | None`

Get the hub's bound address after start.

```python
addr = await hub.address()  # "127.0.0.1:47900"
```

#### `url() -> str | None`

Get the hub's WebSocket URL.

```python
url = await hub.url()  # "wss://127.0.0.1:47900"
```

### Complete SC Example

Provision distinct hub-hosting device and node identities as described under
[SC Device UUID migration](#sc-device-uuid-migration); the explicit environment
variables here contain those already-stored UUID strings, not new per-start IDs.

```python
import asyncio
import os
from uuid import UUID
from rusty_bacnet import (
    BACnetClient, BACnetServer, ScHub,
    ObjectType, ObjectIdentifier, PropertyIdentifier, PropertyValue,
)

async def main():
    # 1. Start the SC hub
    hub_uuid = UUID(os.environ["SC_HUB_DEVICE_UUID"]).bytes
    server_uuid = UUID(os.environ["SC_SERVER_DEVICE_UUID"]).bytes
    client_uuid = UUID(os.environ["SC_CLIENT_DEVICE_UUID"]).bytes
    hub = ScHub(
        listen="127.0.0.1:0",
        cert="hub-cert.pem", key="hub-key.pem",
        ca_cert="ca-cert.pem",
        vmac=b"\xff\x00\x00\x00\x00\x01",
        device_uuid=hub_uuid,
    )
    await hub.start()
    hub_url = await hub.url()
    print(f"Hub running at {hub_url}")

    # 2. Start a server connected to the hub
    server = BACnetServer(
        device_instance=1000, device_name="SC Device",
        transport="sc",
        sc_hub=hub_url,
        sc_vmac=b"\x00\x01\x02\x03\x04\x05",
        sc_device_uuid=server_uuid,
        sc_ca_cert="ca-cert.pem",
        sc_client_cert="server-cert.pem",
        sc_client_key="server-key.pem",
    )
    server.add_analog_input(instance=1, name="Temp", units=62, present_value=72.5)
    await server.start()

    # 3. Connect a client to the same hub
    async with BACnetClient(
        transport="sc",
        sc_hub=hub_url,
        sc_vmac=b"\x00\x02\x03\x04\x05\x06",
        sc_device_uuid=client_uuid,
        sc_ca_cert="ca-cert.pem",
        sc_client_cert="client-cert.pem",
        sc_client_key="client-key.pem",
    ) as client:
        # Address the server by its VMAC (hex-colon notation)
        value = await client.read_property(
            "00:01:02:03:04:05",
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(f"SC read: {value.value}")  # 72.5

    await server.stop()
    print(await hub.status())
    print(await hub.shutdown_gracefully())  # "graceful" (no peers left) or "forced"

asyncio.run(main())
```

---

## Transport Configuration Examples

### BACnet/IP (default)

```python
# Client
client = BACnetClient(
    interface="0.0.0.0",
    port=47808,
    broadcast_address="255.255.255.255",
)

# Server
server = BACnetServer(
    device_instance=1234,
    device_name="BIP Device",
    interface="0.0.0.0",
    port=47808,
)
```

### BACnet/IPv6

`ipv6_interface=None` and `"::"` select one unambiguous usable
local link and address. A non-loopback multicast interface is preferred; a unique
non-link-local address on it is preferred over a unique link-local address.
Ambiguity fails async client entry or server startup; use a concrete local IPv6
address to select its unique interface. In 0.12.0 this replaced 0.11.0's
selection, which asked the routing table for an address and fell back to `::1`,
without changing constructor signatures. There is no silent `::1`
fallback for failed physical selection. Explicit loopback remains node-local.

The selected address and actual bound port are used for outgoing data and control
frames. Incoming packets must belong to that interface and the selected unicast
address or BACnet multicast group before they can populate the VMAC table or
reach the application. Link-local addresses retain the interface zone internally;
multicast scope does not make them routable beyond their link. There is no Python
foreign-device entry point in this change.

Fresh installed Linux tests cover default and explicit ULA server Who-Is/I-Am and
client discovery. macOS loopback covers transport multicast intake and
unicast/control source preservation; Windows has compile evidence only. Full
Annex U conformance and deployment-network reachability remain unqualified.

```python
# Client
client = BACnetClient(
    transport="ipv6",
    ipv6_interface="::",
    port=47808,
)

# Server
server = BACnetServer(
    device_instance=1234,
    device_name="IPv6 Device",
    transport="ipv6",
    ipv6_interface="::",
    port=47808,
)
```

### BACnet/SC (Secure Connect)

#### SC Device UUID migration

Both `BACnetClient` and `BACnetServer` require the new **keyword-only**
`sc_device_uuid` for `transport="sc"`. Supply exactly 16 bytes whose entire value
is not zero. Omission, `None`, lengths such as 0/15/17, and all-zero values raise
`ValueError` in the constructor **after existing credential-presence checks** and
before certificate-file or network I/O. Accepted `bytes`/`bytearray` values are
copied into an owned fixed-size array; later mutation of the input buffer cannot
change the retained identity. UUID version/variant bits are not validated.

This is an intentional SC runtime compatibility break. The shared signature keeps
`sc_device_uuid=None` only for non-SC use; B/IP, IPv6, and MS/TP ignore the option
like other SC-only configuration. All old positional slots, including heartbeat,
IPv6, and server passwords, are unchanged. Existing SC callers must add the new
keyword even when using the old positional credentials.

Base Standard 135-2020 AB.1.5.3 wants a device's UUID created once, before the
device is first installed, kept in storage that survives a restart, and never
changed afterwards. **The caller owns all of this provisioning and persistence.** Load the
same stored bytes into every fresh hub/client/server object; do not call `uuid4()` or
otherwise generate a new ID during startup. The library does not choose a path,
store UUIDs, or detect a changed UUID without application-owned history. It cannot
guarantee lifetime identity. Python nodes retain their existing lifecycle support:
stop/start or recreate them with the persisted UUID; no automatic Python reconnect
support is added or claimed.

For illustration only, three independently provisioned **test** identities might be:

```python
from uuid import UUID
# Test fixtures only. Production must load its own durably provisioned values.
hub_uuid = UUID("9a21f164-1a15-454d-9ed7-e3a2710d7001").bytes
server_uuid = UUID("8e62ac46-d708-4226-9137-76a32b619315").bytes
client_uuid = UUID("95dfe4ef-97f6-490d-9a2c-f2b4b0c0e682").bytes
```

Do not share a UUID between distinct devices. Same-UUID replacement at the hub is
intentional (AB.6.2.3), including when a device's VMAC differs; it is not a promise
that two same-UUID nodes coexist. Distinct UUIDs also need non-colliding VMACs.
`ScHub` now requires its hosting device UUID and rejects reserved local VMACs,
as described [above](#schub). The underlying raw Rust transport now requires a
configured UUID and nonreserved local VMAC at start, before transport-owned I/O;
it cannot undo a caller's prior WebSocket dial. Python signatures and earlier
constructor preflights are unchanged. The raw guard runs at startup, and the Rust
transport gives applications no mutable access to its connection afterwards; see the
[Rust startup/retry limits](rust-api.md#bacnetsc-client-transport).
The owner-approved #517 acceptance closeout
resolves the scoped default/nil identity problem with caller-owned provisioning
and storage. These local API checks are not certificate-to-UUID
binding, full identity-profile validation, or full Annex AB conformance.

**Receiving hub compatibility break:** legacy raw peers sending an all-zero UUID
in Connect-Request are now rejected after TLS/WebSocket establishment, before
registration, activity refresh or replacement. Eligible NAKs use
`COMMUNICATION/PARAMETER_OUT_OF_RANGE` (7/80), not Duplicate-VMAC, with the existing
reply-addressing/suppression rules. New malformed peers close; malformed repeats
leave the registered peer's limits and heartbeat/activity state intact. Python
constructors are unchanged by this receive check. Nonzero UUID bits remain opaque;
generic Rust codecs and manual raw WebSocket sending still permit nil syntax.
Native `BACnetClient` async entry and `BACnetServer.start()` silently discard
Connect-Accept with a zero UUID after TLS/WebSocket setup. AB.2 prohibits replies
to response messages, so no NAK is sent. Invalid Accepts do not complete startup
or install peer identity/limits and do not reset the original connect wait. A
later valid Accept can complete that handshake; nil-only traffic times out.
Generated-certificate installed-native tests cover both paths without changing
Python signatures or exception mapping. This is local policy, not UUID-profile
or lifetime-storage validation; post-handshake startup rollback is not expanded.

**Zero-limit receive policy (Refs #519):** native `BACnetClient` async
entry and `BACnetServer.start()` also silently discard Connect-Accept advertising
zero Max-BVLC or Max-NPDU. AB.2 forbids a response: no NAK, startup completion,
peer-limit commit or original connect-deadline reset. A later valid Accept can
recover; zero-only traffic expires the original wait. The native hub rejects
eligible zero-limit Requests with `COMMUNICATION/PARAMETER_OUT_OF_RANGE` (7/80)
before registration/activity/replacement, preserving existing reply suppression
and malformed-repeat behavior. Generated-certificate installed-native tests cover
both public node APIs and a surviving hub peer's ReadProperty.
This is **zero-only local policy**, not a universal minimum-capacity conformance
claim. All positive values remain compatible, including very small/inverted
pairs; positive floors and field relationship checks are deferred. Defaults,
outgoing budgets and Python signatures/exception mapping remain unchanged.
Generic Rust zero codec syntax and post-start public mutation exclusions remain.
#519 stays open/partial and does not reopen the closed #517 identity acceptance.

#### Required operational credentials

**Compatibility change:** both `BACnetClient` and `BACnetServer` require all of
`sc_ca_cert`, `sc_client_cert`, and `sc_client_key` when `transport="sc"`.
Omission, `None`, or an empty string raises `ValueError` at construction. Supply
the installation's trusted CA PEM file and this node's operational certificate
with its matching private key. There is no native/system-root fallback,
certificate-less mode, or insecure flag. The `None` defaults and positional slots
(including subsequent heartbeat, IPv6, and server password arguments) remain
unchanged for non-SC use and argument-layout compatibility only.

File I/O remains at startup: client async entry and server `start()` load and
validate the files before TCP/DNS connection attempts. Unreadable, empty, malformed
credentials and cert/key mismatch raise `RuntimeError` with `TLS config error:`.
Server **local TLS configuration** failures leave pending object registrations
intact; repair files at the same paths and retry `start()`. The client also reloads
files on retry. This is not general startup rollback: later failures, including
TLS peer rejection after dialing, retain existing lifecycle behavior and may
consume server registrations. Preflight checks rustls configuration validity,
not the local certificate's date or issuer against the site store; the remote
peer validates that certificate during the real handshake.

TLS 1.3-only remains the existing local policy. Base Standard 135-2020 Annex
AB.7.4/AB.7.4.1.1 supplies the mutual-authentication and installation-credential
context, but this change is not full security-profile conformance, certificate
to VMAC/UUID authorization, or a change to hostname/revocation/issuer policy.
The shared native `ScNodeTlsConfig::from_der` now constructs this local policy;
Python interfaces and error categories remain unchanged. Normal TLS resumption is
preserved and may not retransmit certificates. Credentials are offered if requested
and compatible; a trusted server with no CertificateRequest can complete without
receiving the node certificate. Local configuration does not attest an arbitrary
remote hub's verification policy (#513 remains open/partial).

```python
# Client connecting to a hub
client = BACnetClient(
    transport="sc",
    sc_hub="wss://hub.example.com:47900",
    sc_vmac=b"\x00\x02\x03\x04\x05\x06",
    sc_device_uuid=client_uuid,  # Loaded from caller-owned lifetime storage.
    sc_ca_cert="ca-cert.pem",
    sc_client_cert="client-cert.pem",
    sc_client_key="client-key.pem",
    sc_heartbeat_interval_ms=30000,
    sc_heartbeat_timeout_ms=60000,
)

# Server connecting to a hub
server = BACnetServer(
    device_instance=1234,
    device_name="SC Device",
    transport="sc",
    sc_hub="wss://hub.example.com:47900",
    sc_vmac=b"\x00\x01\x02\x03\x04\x05",
    sc_device_uuid=server_uuid,  # Distinct provisioned device, not a new ID per start.
    sc_ca_cert="ca-cert.pem",
    sc_client_cert="server-cert.pem",
    sc_client_key="server-key.pem",
)
```

#### Heartbeat interval and timeout

Production BACnet/SC clients validate the configured heartbeat interval as `3000..=300000`
ms and require `sc_heartbeat_timeout_ms` to be greater than the interval.

#### Accepted-direct identity and responses

Native code retains a verified direct leaf fingerprint
and connection incarnation through queued server work, duplicate/replay admission,
and partial request reassembly. Python does not expose a principal authorizer or
the direct listener through this API; its mutation policy remains the existing
static `permissive`/`deny_all` choice. The Rust identity APIs are new in
0.12.0. Hub admission's scope-only channel assertion and Hub-relayed application
traffic never become downstream direct leaf identities. See the
[Rust identity contract](rust-api.md#accepted-direct-tls-identity). Native
`BACnetServer` confines accepted-direct confirmed replies, LSO replay and
segmented-request controls to the original socket. Unconfirmed Who-Is/Who-Has
discovery replies retain ordinary routing. Python still exposes neither
direct-listener setup nor this response capability. This adds no Python direct-connection entry point
or Python direct-connection support. Native code also confines the standalone
client's inbound confirmed replies and the shared endpoint's narrow responder;
that does not change outgoing client transaction/retry policy or expose a Python
response capability. See the [server response scope](rust-api.md#accepted-direct-server-responses)
and [native client/endpoint scope](rust-api.md#accepted-direct-client-and-endpoint-replies).
Native `ScTransport::with_direct_tls` admits bidirectional application traffic
with matching verified identity and original reply authority; established accepted
peers also serve ordinary unicast with discovery disabled. Arbitrary custom dialers
remain send-only. Outgoing transactions retain standard address/Invoke-ID correlation
and Hub/direct path switching, not a same-leaf continuity guarantee. These APIs are new
in 0.12.0 and add no Python direct-connection entry point. See the
[native routing contract](rust-api.md#bidirectional-direct-traffic).

## Request admission limits

GetAlarmSummary has positive keyword-only `alarm_summary_max_objects=4096`
and `alarm_summary_max_service_ack_bytes=16384`. The work preflight includes
all database objects, even non-alarming objects, before any projection callback.
Overflow returns whole-service server Abort OUT_OF_RESOURCES; encoded service
byte overflow returns BUFFER_OVERFLOW, never a partial successful ACK. Bytes
exclude APDU/NPDU and are independent of peer APDU size. See
[GetAlarmSummary budgets](alarm-summary-budget.md) for migration, preserved
segmentation, error behavior and precise callback/allocation exclusions.

RPM additionally has independent, positive keyword-only constructor limits:
`rpm_max_result_elements=256` and `rpm_max_service_ack_bytes=16384`.
Whole-request expanded-result overflow returns server Abort OUT_OF_RESOURCES
before handler property reads. Accumulated encoded service-ACK overflow returns
server Abort BUFFER_OVERFLOW, without a partial ACK; previous read side effects
are not rolled back. Zero raises `ValueError`, negative/native-overflow values
raise `OverflowError`, before any transport startup. These limits do not bound
decoder/metadata allocations, individual property reads or value encodings,
allocator capacity, or whole-process memory. See [RPM budgets](rpm-budget.md)
for counting rules, Rust configuration, compatibility, and protocol rationale.

`BACnetServer(...)` accepts keyword-only `max_confirmed_in_flight=64` and
`max_unconfirmed_in_flight=32`, followed by keyword-only
`max_confirmed_in_flight_per_peer=16`, `max_unconfirmed_in_flight_per_peer=8`,
`confirmed_recovery_reserve=4`, and `max_recovery_in_flight_per_peer=1`.
Existing positional arguments and global defaults are unchanged.
All except the reserve must be positive and within the native semaphore range; zero is rejected during
construction, before any transport opens. These provisional defaults bound
top-level handler concurrency, not all server work or per-peer fairness.
Each effective peer cap is the smaller of its configured cap and global cap;
the reserve must satisfy `0 <= reserve < max_confirmed_in_flight`. Thus global=1
requires explicit reserve=0, and custom globals <=4 must specify a smaller reserve.
Reserve=0 makes ENABLE ordinary, not denied. A valid routed network (1..65534) and
nonempty source MAC identify the logical peer, otherwise the immediate MAC does.
On SC, the supplied VMAC/logical source is not an authenticated principal.
Identity multiplication/spoofing can exhaust global capacity; quotas do not
guarantee availability once that capacity is full. The default confirmed total64
is strictly partitioned into ordinary60/protected4 with no lending in either
direction. Only decoder-accepted DCC ENABLE is protected; [local DCC policy](dcc-policy.md)
defaults to denial without changing eligibility. Existing password checks
still apply in the handler, so wrong/missing required passwords can consume a slot
before PASSWORD_FAILURE. Ordinary peer16 and protected peer1 are independent:
the same peer can hold 16+1 handlers in either arrival order. This replaces the
former inclusive peer16 policy without changing constructor fields or defaults.
Ordinary peer capacity is clamped to global minus reserve; recovery peer capacity
is clamped only to reserve, no longer to the ordinary peer setting. For example,
ordinary peer1/recovery peer3/reserve3 permit 1+3 if global room exists. Reserve0
retains shared ordinary accounting with the ordinary peer cap clamped to global.
DCC ENABLE is the sole designated critical service in the owner-accepted bounded
#521 scope; other services have no recovery reserve. Seven service-specific
budgets are delivered, not blanket-deferred. See the
[acceptance/evidence matrix](request-admission.md#bounded-acceptance-and-evidence)
and [service budget index](request-admission.md#delivered-service-budgets) for
the completed scope, configuration/migration links and precise exclusions.
This acceptance is not a general fairness, all-configurations availability,
total-work or whole-memory guarantee; #522 remains separate and open.

`await server.request_admission_counters()` returns a stable typed dictionary
of independent active/admitted/overload/shutdown counters, including the
separate eight-worker Abort pool and its counted confirmed-drop fallback.
Like `comm_state()`, this accessor raises `RuntimeError` before start and after
stop. Admission totals do not imply successful response sends.
The additive `confirmed_global_overloaded_total`, `confirmed_peer_overloaded_total`,
`unconfirmed_global_overloaded_total`, and `unconfirmed_peer_overloaded_total`
fields classify rejections partition/global-first. New `recovery_active`,
`recovery_admitted_total`, and `recovery_overloaded_total` are protected subsets;
existing confirmed aggregates include both partitions. They remain zero when the
reserve is zero. Each aggregate overload total equals
its two reason totals at quiescence; independently sampled snapshots are not
atomic. No peer identity history is exposed.

See [server request admission](request-admission.md) for exact fields, accepted
ranges, overload behavior, and the known extreme-overload/conformance limitation.

### Multi-device batch concurrency

`BACnetClient.read_property_from_devices`,
`read_property_multiple_from_devices`, and `write_property_to_devices` accept
`max_concurrent=None` (32) or a positive integer that fits the platform native
`usize`. Zero raises `ValueError` synchronously before starting a future or I/O,
including an empty batch on an unstarted client. Negative or oversized integers
raise `OverflowError`. Normal calls require a running client; empty batches then
return an empty list. These methods synchronously return an `asyncio.Future`:
use `await` or `asyncio.ensure_future`, not `asyncio.create_task`. Installed stubs
express `Awaitable[list[...]]` with three private, structural `TypedDict` helpers;
those helper names are not runtime classes to import.

Results stay in completion order. Every dictionary now includes `request_index`,
the zero-based position in the original input list, so repeated identical requests
to the same Device remain distinguishable. The other keys are `device_instance`
and `error`, plus `value` for RP or `results` for RPM. `error` is `None` on success
or an existing `BacnetError` instance on a per-request failure, replacing the old
string. `BacnetProtocolError.error_class/error_code` and
`BacnetRejectError.reason`/`BacnetAbortError.reason` are numeric attributes.

```python
outcomes = await client.read_property_from_devices(requests, max_concurrent=8)
for outcome in outcomes:
    original_request = requests[outcome["request_index"]]
    if outcome["error"] is not None:
        print(original_request, type(outcome["error"]).__name__)
    else:
        print(original_request, outcome["value"])
```

RPM property-level `(ErrorClass, ErrorCode)` tuples and undecodable application
values returned as `bytes` remain nested RPM data, not top-level item errors.
Genuine Python result-construction exceptions propagate from the whole batch;
they are not fabricated BACnet item failures. Mixed success means outcomes in a
completed batch. Cancellation returns no partial list, cancels pending requests,
and prevents queued requests from starting; it cannot retract writes already sent.
Remote WP inputs remain six-tuples; the server's local `source_object` argument
is unrelated. Results do not echo complete requests or encoded write values.
Returned values remain inspectable; this is not a general secrecy guarantee.

### Endpoint direct WriteProperty

`EndpointClient.write_property(address, object_id, property_id, value,
priority=None, array_index=None, *, commandability=...)` returns `Awaitable[None]`
through the endpoint's shared requester. The required keyword is exactly
`"commandable"` or `"noncommandable"`, including when no Reporter is configured.
It declares remote property semantics and never changes the supplied wire priority.
Only direct IPv4 B/IP peers are supported. This adds no Python source Reporter
configuration surface; Rust owns that existing optional profile.

Invalid commandability, priority, raw TLV framing and complete request size fail
synchronously with `ValueError`; priorities outside the native u8 range raise
`OverflowError`. `PropertyValue.application_data(b"")` can represent an empty list;
`PropertyValue.null()` represents NULL. The target still decides datatype validity
and may return a protocol error. Existing endpoint read methods keep their native
Awaitable results and established correlation rules.

```python
role = await endpoint.client()
await role.write_property(
    "127.0.0.1:47808", ObjectIdentifier(ObjectType.ANALOG_VALUE, 7),
    PropertyIdentifier.PRESENT_VALUE, PropertyValue.real(21.0),
    priority=8, commandability="commandable",
)
```
