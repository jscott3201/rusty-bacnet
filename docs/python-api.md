# Python API Reference

`rusty_bacnet` provides Python bindings for the Rust BACnet protocol stack via PyO3. All I/O operations are async (`asyncio`-based).

## Installation

```bash
pip install rusty-bacnet
```

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

All enums have class-level named constants, plus `from_raw(int)` and `to_raw()` for raw access. They support `==`, `hash()`, and `repr()`.

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

For `device_communication_control`. Constants: `ENABLE`, `DISABLE`, `DISABLE_INITIATION`.

```python
ed = EnableDisable.DISABLE
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

---

## ObjectIdentifier

Immutable BACnet object identifier (type + instance).

```python
oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
oid.object_type   # ObjectType.ANALOG_INPUT
oid.instance       # 1
```

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
```

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
| `"date"` | `tuple(year, month, day, day_of_week)` |
| `"time"` | `tuple(hour, minute, second, hundredths)` |
| `"list"` | `list` of native Python values |

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
    "value": PropertyValue | bytes | None,
}
```

---

## CovNotificationIterator

Async iterator yielding `CovNotification` objects from the client's broadcast channel.

```python
async for notification in client.cov_notifications():
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
    # IPv6 options:
    ipv6_interface=None,         # IPv6 bind address
    # SC options:
    sc_hub=None,                 # WebSocket hub URL
    sc_vmac=None,                # 6-byte VMAC
    sc_ca_cert=None,             # CA certificate path
    sc_client_cert=None,         # Client certificate path
    sc_client_key=None,          # Client private key path
    sc_heartbeat_interval_ms=None,
    sc_heartbeat_timeout_ms=None,
)
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

```python
value = await client.read_property(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    PropertyIdentifier.PRESENT_VALUE,
)
print(value.value)  # 72.5
```

#### `write_property(address, object_id, property_id, value, priority=None, array_index=None)`

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
- `"results"`: `list[dict]` with `"property_id"`, `"array_index"`, `"value"` (PropertyValue or None), `"error"` (tuple of ErrorClass, ErrorCode or None)

#### `write_property_multiple(address, specs)`

Write multiple properties to multiple objects in one request.

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

#### `subscribe_cov(address, subscriber_process_identifier, monitored_object_identifier, confirmed, lifetime=None)`

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

#### `cov_notifications() -> CovNotificationIterator`

Returns an async iterator. Can be called multiple times for independent consumers.

```python
async for notif in client.cov_notifications():
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
    EnableDisable.DISABLE,
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

#### `acknowledge_alarm(address, acknowledging_process_identifier, event_object_identifier, event_state_acknowledged, acknowledgment_source)`

```python
await client.acknowledge_alarm(
    "192.168.1.100:47808",
    acknowledging_process_identifier=1,
    event_object_identifier=ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    event_state_acknowledged=3,       # EVENT_STATE value
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

#### `read_range(address, object_id, property_id, array_index=None, range_type=None, reference_index=None, reference_seq=None, count=None) -> dict`

Read a range of items from a list or log object.

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

# Read by sequence number
result = await client.read_range(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.TREND_LOG, 1),
    PropertyIdentifier.LOG_BUFFER,
    range_type="sequence",
    reference_seq=100,
    count=10,
)

# Read all (no range)
result = await client.read_range(
    "192.168.1.100:47808",
    ObjectIdentifier(ObjectType.TREND_LOG, 1),
    PropertyIdentifier.LOG_BUFFER,
)
```

Return dict keys: `"object_id"`, `"property_id"`, `"array_index"`, `"result_flags"` (tuple of 3 bools), `"item_count"` (int), `"item_data"` (bytes).

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

#### `confirmed_private_transfer(address, vendor_id, service_number, service_parameters=None) -> bytes`

Send a vendor-specific confirmed service request.

```python
raw = await client.confirmed_private_transfer(
    "192.168.1.100:47808",
    vendor_id=999,
    service_number=1,
    service_parameters=b"\x01\x02\x03",
)
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

#### `confirmed_text_message(address, source_device, message_priority, message, message_class_type=None, message_class_value=None) -> bytes`

Send a confirmed text message to a device.

```python
from rusty_bacnet import MessagePriority

raw = await client.confirmed_text_message(
    "192.168.1.100:47808",
    source_device=ObjectIdentifier(ObjectType.DEVICE, 1234),
    message_priority=MessagePriority.URGENT,
    message="Fire alarm on floor 3",
    message_class_type="numeric",     # "numeric" or "string"
    message_class_value=1,            # int for numeric, str for string
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

#### `get_enrollment_summary(address, acknowledgment_filter, event_state_filter=None, event_type_filter=None, min_priority=None, max_priority=None, notification_class_filter=None) -> bytes`

Get enrollment summary with filters.

```python
from rusty_bacnet import EventState, EventType

raw = await client.get_enrollment_summary(
    "192.168.1.100:47808",
    acknowledgment_filter=0,                        # 0=all, 1=acked, 2=not-acked
    event_state_filter=EventState.OFFNORMAL,
    min_priority=0,
    max_priority=255,
)
```

---

### COV Property Multiple

#### `subscribe_cov_property_multiple(address, subscriber_process_identifier, specs, max_notification_delay=None, issue_confirmed_notifications=None)`

Subscribe to COV on multiple properties across multiple objects.

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
    max_notification_delay=10,
    issue_confirmed_notifications=True,
)
```

---

### Write Group

#### `write_group(address, group_number, write_priority, change_list, inhibit_delay=None)`

Write to a channel group.

```python
await client.write_group(
    "192.168.1.100:47808",
    group_number=1,
    write_priority=8,
    change_list=[
        # (object_id_or_None, channel_or_None, encoded_value_bytes)
        (ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1), 1, encoded_bytes),
    ],
    inhibit_delay=False,
)
```

---

### Virtual Terminal

#### `vt_open(address, vt_class) -> bytes`

Open a virtual terminal session.

```python
raw = await client.vt_open("192.168.1.100:47808", vt_class=1)
```

#### `vt_close(address, session_ids)`

Close one or more virtual terminal sessions.

```python
await client.vt_close("192.168.1.100:47808", session_ids=[1, 2])
```

#### `vt_data(address, session_id, data, data_flag) -> bytes`

Send data on a virtual terminal session.

```python
raw = await client.vt_data(
    "192.168.1.100:47808",
    session_id=1,
    data=b"Hello VT",
    data_flag=False,
)
```

---

### Audit Services

#### `confirmed_audit_notification(address, service_data) -> bytes`

Send a confirmed audit notification.

```python
raw = await client.confirmed_audit_notification(
    "192.168.1.100:47808",
    service_data=encoded_audit_bytes,
)
```

#### `unconfirmed_audit_notification(address, service_data)`

Send an unconfirmed audit notification (fire-and-forget).

```python
await client.unconfirmed_audit_notification(
    "192.168.1.100:47808",
    service_data=encoded_audit_bytes,
)
```

#### `audit_log_query(address, acknowledgment_filter, query_options_raw) -> bytes`

Query the audit log.

```python
raw = await client.audit_log_query(
    "192.168.1.100:47808",
    acknowledgment_filter=0,
    query_options_raw=encoded_query_bytes,
)
```

---

### Additional Discovery

#### `who_am_i()`

Broadcast a WhoAmI request for network path verification.

```python
await client.who_am_i()
```

---

## BACnetServer

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
    # SC options same as BACnetClient
)
```

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
server.add_calendar(instance=1, name="Holiday Calendar")
server.add_schedule(instance=1, name="Occupancy Schedule")
server.add_notification_class(instance=1, name="Critical Alarms", notification_class=1)
server.add_notification_forwarder(instance=1, name="Forwarder")
server.add_alert_enrollment(instance=1, name="Alert")
server.add_event_enrollment(instance=1, name="Event", event_type=0)
```

#### Logging & Trending

```python
server.add_trend_log(instance=1, name="Temp Log", buffer_size=1000)
server.add_trend_log_multiple(instance=1, name="Multi Log", buffer_size=1000)
server.add_event_log(instance=1, name="Event Log", buffer_size=500)
server.add_audit_log(instance=1, name="Audit Trail", buffer_size=500)
server.add_audit_reporter(instance=1, name="Reporter")
```

#### Building Control

```python
server.add_loop(instance=1, name="PID Loop", output_units=62)
server.add_command(instance=1, name="Command")
server.add_timer(instance=1, name="Timer")
server.add_load_control(instance=1, name="Load Control")
server.add_program(instance=1, name="Program")
server.add_averaging(instance=1, name="Averaging")
server.add_channel(instance=1, name="Channel", channel_number=1)
server.add_staging(instance=1, name="Staging", num_stages=4)
```

#### Lighting

```python
server.add_lighting_output(instance=1, name="Dimmer")
server.add_binary_lighting_output(instance=1, name="On/Off Light")
```

#### Life Safety

```python
server.add_life_safety_point(instance=1, name="Smoke Detector")
server.add_life_safety_zone(instance=1, name="Floor 3 Zone")
```

#### Access Control

```python
server.add_access_door(instance=1, name="Main Entry")
server.add_access_point(instance=1, name="Lobby Access")
server.add_access_credential(instance=1, name="Badge 001")
server.add_access_user(instance=1, name="John Doe")
server.add_access_rights(instance=1, name="Employee Access")
server.add_access_zone(instance=1, name="Building A")
server.add_credential_data_input(instance=1, name="Card Reader")
```

#### Transportation

```python
server.add_elevator_group(instance=1, name="Elevator Bank A")
server.add_escalator(instance=1, name="Escalator 1")
server.add_lift(instance=1, name="Elevator 1", num_floors=10)
```

#### Groups & Views

```python
server.add_group(instance=1, name="HVAC Group")
server.add_global_group(instance=1, name="All Temps")
server.add_structured_view(instance=1, name="Floor Plan")
```

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
server.add_pulse_converter(instance=1, name="Pulse Count", units=95) # 95 = counts
server.add_file(instance=1, name="Config File", file_type="text/plain")
server.add_network_port(instance=1, name="BIP Port", network_type=0)
```

### Lifecycle

```python
await server.start()
# Server is now responding to BACnet requests
address = await server.local_address()  # e.g., "0.0.0.0:47808"
await server.stop()
```

### Runtime Object Access

#### `read_property(object_id, property_id, array_index=None) -> PropertyValue`

Read a property from a local object in the server's database.

```python
value = await server.read_property(
    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    PropertyIdentifier.PRESENT_VALUE,
)
print(value.value)  # 72.5
```

#### `write_property_local(object_id, property_id, value, priority=None, array_index=None)`

Write a property on a local object.

```python
await server.write_property_local(
    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
    PropertyIdentifier.PRESENT_VALUE,
    PropertyValue.real(73.0),
)
```

#### `comm_state() -> int`

Get the server's current DeviceCommunicationControl state.

```python
state = await server.comm_state()
# 0 = Enable, 1 = Disable, 2 = DisableInitiation
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
| `BacnetProtocolError` | Remote device returned a BACnet error (class + code) |
| `BacnetTimeoutError` | Request timed out (APDU retries exhausted) |
| `BacnetRejectError` | Remote device rejected the request |
| `BacnetAbortError` | Remote device aborted the request |

```python
from rusty_bacnet import BacnetError, BacnetTimeoutError

try:
    value = await client.read_property(addr, oid, pid)
except BacnetTimeoutError:
    print("Device not responding")
except BacnetError as e:
    print(f"BACnet error: {e}")
```

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

    # Update a value at runtime
    await server.write_property_local(
        ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
        PropertyIdentifier.PRESENT_VALUE,
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
            async for notif in client.cov_notifications():
                for v in notif.values:
                    print(f"COV: {v['property_id']} = {v['value']}")

        listener = asyncio.create_task(listen())
        await asyncio.sleep(30)
        listener.cancel()

asyncio.run(client_example())
```

---

## ScHub

BACnet/SC Hub — a TLS WebSocket relay for BACnet Secure Connect. Both `BACnetClient` and `BACnetServer` with `transport="sc"` connect to a hub as clients. The hub relays messages between connected nodes using VMAC addresses.

### Constructor

```python
from rusty_bacnet import ScHub

hub = ScHub(
    listen="127.0.0.1:0",       # Bind address (port 0 = auto-assign)
    cert="hub-cert.pem",         # Server TLS certificate
    key="hub-key.pem",           # Server TLS private key
    vmac=b"\xff\x00\x00\x00\x00\x01",  # Hub's 6-byte VMAC
    ca_cert="ca-cert.pem",       # Optional CA cert for mutual TLS
)
```

### Methods

#### `start()`

Start the hub. Begins accepting WebSocket connections.

```python
await hub.start()
```

#### `stop()`

Stop the hub. Disconnects all clients.

```python
await hub.stop()
```

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

```python
import asyncio
from rusty_bacnet import (
    BACnetClient, BACnetServer, ScHub,
    ObjectType, ObjectIdentifier, PropertyIdentifier, PropertyValue,
)

async def main():
    # 1. Start the SC hub
    hub = ScHub(
        listen="127.0.0.1:0",
        cert="hub-cert.pem", key="hub-key.pem",
        vmac=b"\xff\x00\x00\x00\x00\x01",
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
    await hub.stop()

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

```python
# Client connecting to a hub
client = BACnetClient(
    transport="sc",
    sc_hub="wss://hub.example.com:47900",
    sc_vmac=b"\x00\x02\x03\x04\x05\x06",
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
    sc_ca_cert="ca-cert.pem",
    sc_client_cert="server-cert.pem",
    sc_client_key="server-key.pem",
)
```
