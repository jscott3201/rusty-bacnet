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
- Other content with a context tag (a Lighting Output's
Lighting\_Command, a vendor's proprietary property) is `tagged`
(#2618): `value` is the tree of its tags, a list of `TagNode`, and
`octets` are the octets exactly as served, so a write sends them
back unchanged. Every tag reads without the property's datatype
(Clause 20.2.1.6); a context-tagged primitive's contents stay
`bytes`, since only the property's datatype says what they are.
- Content this type has no form for (a UCS-4, DBCS or JIS string, bad
UTF-8, an ENUMERATED past 32 bits, anywhere in the value), or a tree
past the decoder's 10,000 elements, is `application_data` holding
the octets exactly as served.
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

- `tag: str`: Type tag: 'null', 'boolean', 'unsigned', 'signed', 'real', 'double', 'octet\_string', 'character\_string', 'bit\_string', 'enumerated', 'date', 'time', 'object\_identifier', 'list', 'application\_data', 'tagged' (a value read as its tags, `value` a list of `TagNode`), or one typed constructed element: 'destination', 'port\_permission', 'read\_access\_specification', 'read\_access\_result', 'action\_list', 'device\_object\_reference', 'authentication\_factor\_format', 'stage\_limit\_value', 'access\_rule', 'device\_object\_property\_reference', 'object\_property\_reference', 'property\_access\_result', 'recipient', 'daily\_schedule', 'special\_event', 'calendar\_entry', 'date\_range', 'timestamp', 'cov\_subscription', 'value\_source', 'scale', 'prescale', 'address\_binding', 'shed\_level', 'authentication\_factor', 'event\_parameter'.
- `value: Any`: The Python-native value (int, float, str, bytes, bool, dict, tuple, ObjectIdentifier, BitString, list, or None); `application_data` is `bytes` (except a DateTime Value's set Priority\_Array slot, `[date, time]`), `tagged` a list of `TagNode`, and a typed constructed element the form its typed write takes, or the form docs/python-api.md gives it. A date is `(year, month, day, day_of_week)` with the full year, or 255 for an unspecified year, as in every other date the binding reads. A bit string is a `BitString`, named when the value was read from a property of known bit-string type.
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

### PropertyValue.tagged

```python
@staticmethod
def tagged(octets: bytes) -> PropertyValue
```

The value `octets` encode, as their tags: what a read of a value
with no typed form and a context tag gives (#2618). `value` is a
list of `TagNode`; the octets are kept as given, so a write sends
them unchanged. Octets that don't read as tagged elements (cut short,
a closing tag that closes nothing, opening tags nested past 32, more
than 10,000 elements), or hold an application value this type has no
form for, raise ValueError. A `list` of tagged values keeps its
items tagged.

## TagNode

One tagged element of a value read as its tags (#2618), frozen:
`PropertyValue.tagged` values hold a list of them.

BACnet tags read without knowing a value's datatype (Clause 20.2.1.6):
an application tag names its datatype, every primitive gives its length,
and a constructed element sits between an opening and a closing tag of
one number. A context tag's meaning is the property's to say, so a
context-tagged primitive keeps its contents as `bytes`.

Two nodes are equal when they hold the same octets, and equal nodes hash
alike. `copy` and `pickle` rebuild a node from its octets.

Properties:

- `tag_class: Literal["application", "context"]`: The tag's class (Clause 20.2.1.1).
- `tag_number: int`: The tag number, 0 to 254: an application tag's datatype (0 NULL, 1 BOOLEAN, 2 Unsigned, 3 Signed, 4 REAL, 5 Double, 6 OCTET STRING, 7 CharacterString, 8 BIT STRING, 9 ENUMERATED, 10 Date, 11 Time, 12 BACnetObjectIdentifier; Clause 20.2.1.4), or a context tag's field.
- `constructed: bool`: Whether it is the members between an opening and a closing tag (Clause 20.2.1.3.2).
- `value: PropertyValue | bytes | list[TagNode]`: An application primitive's `PropertyValue` (the value a bare read of it gives, without names from a property), a context primitive's contents as `bytes`, or a constructed element's members.
- `octets: bytes`: The element's octets exactly as read: its tag and contents, or its opening tag through its closing tag.

## DiscoveredDevice

A device discovered via Who-Is / I-Am.

Properties:

- `object_identifier: ObjectIdentifier`
- `mac_address: bytes`: The MAC the I-Am came from: IPv4 and port on B/IP, the 3-octet VMAC on B/IPv6, the station on MS/TP.
- `max_apdu_length: int`
- `segmentation_supported: Segmentation`
- `vendor_id: int`
- `seconds_since_seen: float`: Seconds since the device was last heard from when this object was made: its latest I-Am, or its latest answer to one of this client's confirmed requests.
- `source_network: Optional[int]`
- `source_address: Optional[bytes]`

## Inventory223Experimental

A device's objects as the experimental Standard 223 export read them
(`BACnetClient.export_223_experimental`). Device text is kept exactly
as the device sent it. Refuses `copy` and `pickle`.

Properties:

- `device_instance: int`: The device instance.
- `device_name: Optional[str]`: The Device object's Object\_Name, exactly as the device sent it; None when it couldn't be read.
- `objects: tuple[Object223Experimental, ...]`: The objects read, in Object\_List order, as a tuple.
- `stopped: Optional[str]`: Why the reading stopped early, None when it didn't: a request to the device timed out or its link failed while its objects were being read, so the objects after that point are missing.

### Inventory223Experimental.to_turtle

```python
def to_turtle(
    self,
    base_iri: Optional[str] = None,
    write_priority: Optional[int] = None,
) -> str
```

The inventory as Turtle. `base_iri` defaults to
`http://example.org/bacnet/`; each device's nodes go under
`<base_iri>device-<instance>/`. `write_priority` (1 to 16) goes in
the reference of every commandable point. A bad base IRI or a
priority outside 1 to 16 raises ValueError saying which rule failed;
a priority outside 0 to 255 raises OverflowError.

## Object223Experimental

One object of an `Inventory223Experimental`: a point, or an object
whose read failed, which has only its identifier and `error`.

Properties:

- `object_identifier: ObjectIdentifier`: The object.
- `name: Optional[str]`: Its Object\_Name; None when its read failed.
- `description: Optional[str]`: Its Description; None when it has none or couldn't be read.
- `units: Optional[EngineeringUnits]`: The Units of an analog object or an integer value; None for other objects or when its read failed.
- `number_of_states: Optional[int]`: A multi-state object's Number\_Of\_States; None for other objects or when its read failed.
- `state_texts: list[str]`: A multi-state object's State\_Text, state 1 first; empty for other objects, when it has none or when its read failed. An entry that wasn't a string is "".
- `commandable: bool`: Whether it has a Priority\_Array: always for an output, and for a value when the device gave one. The Turtle makes such a point actuatable.
- `error: Optional[str]`: Why its read failed, None when it didn't: the device's error, which names the property when the object lacked its Object\_Name, the Units of an analog object or an integer value, or a multi-state object's Number\_Of\_States. The Turtle skips such an object.

## Value

One value a high-level call read or sent (#2561): typed (`value`),
as text (`text`), as plain data (`data`) and as octets (`raw`).

A value class: two are equal when they hold the same octets, text and
data (what they are a value of isn't compared), equal values hash
alike, comparing with anything else gives NotImplemented, and one
copies and pickles. It keeps nothing of the client or
the device it came from. `value` is the typed Python form, so there
are no `as_float`-style accessors: `reading.value.value` is the
float, int, str or other value itself.

Properties:

- `value: PropertyValue`: The value as a `PropertyValue`, shaped as `read_property` of the same property returns it (a Priority\_Array slot as a read of that slot by its index).
- `text: str`: The value as text, as the `bacnet` tool writes it: every digit of a number, a state or enumeration by its name (`"active"`), `"null"` for NULL. A `read_many` that reads the object's units or state texts in the same call names them (`"21.5 °C"`, `"2 (Heat)"`); a single `read` names neither.
- `data: Any`: The value as plain data, the shape of the `bacnet` tool's JSON: None, a bool, an int, a float (a REAL as the shortest decimal that reads back as it) or a str, or a dict or list of them; an enumeration is `{"value": 1, "name": "active"}`. A new copy each time.
- `raw: bytes`: The value's octets as the device sent them, or as a write sends them: application-tagged.

## Reading

One property as a bulk read read it (#2561): `value`, or `error`
when the device gave none. Exactly one of the two is None. Refuses
`copy` and `pickle`; its `value` copies and pickles.

Properties:

- `object_id: ObjectIdentifier`: The object read.
- `property_id: PropertyIdentifier`: The property read.
- `array_index: Optional[int]`: The array element read, None for the whole property.
- `value: Optional[Value]`: The value; None when the device gave none (see `error`).
- `error: Optional[BacnetError]`: Why the device gave no value, as the exception a typed read of the property would raise, never raised here: a `BacnetProtocolError` with `error_class` and `error_code` for an unknown property or object, a reject or abort error, or a `BacnetError` for an answer that left the property out. None when the device gave a value.
- `text: Optional[str]`: The value's text; None when the device gave no value.
- `ok: bool`: True when the device gave a value.

## Readings

What `Device.read_many` or `Device.read_object` read (#2561): a
`Reading` each property, in the order asked for, and whether the
device stopped answering part way. Iterating it gives the readings;
`len` and an index work too. Refuses `copy` and `pickle`.

Properties:

- `readings: tuple[Reading, ...]`: The readings, in the order asked for; after `stopped`, those read before the device stopped answering.
- `via: Optional[Literal["rpm", "rp", "mixed"]]`: How the values were read: `"rpm"` (ReadPropertyMultiple), `"rp"` (ReadProperty each) or `"mixed"`; None when nothing was read.
- `stopped: Optional[BacnetError]`: Set when the read ended after it began: the device stopped answering (the `BacnetTimeoutError` or `BacnetTransportError` a single read would have raised), or the client's own link failed a later request (a `BacnetError`). Never raised here. The properties after `readings` weren't read. Always None for `read_object`.

### Readings.get

```python
def get(
    self,
    object: ObjectIdentifier | str,
    property: PropertyIdentifier | str | None = None,
    *,
    index: Optional[int] = None,
) -> Optional[Reading]
```

The first reading of `property` of `object` (its
Present\_Value when `property` is left out), in the forms
`Device.read` takes; None when the read holds none.

### Readings.__len__

```python
def __len__(self) -> int
```

### Readings.__iter__

```python
def __iter__(self) -> Iterator[Reading]
```

### Readings.__getitem__

```python
def __getitem__(self, index: int) -> Reading
```

## ObjectList

A device's Object\_List as read (`Device.objects`, #2561).
Iterating it gives the `ObjectIdentifier`s; `len`, an index and
`in` (an `ObjectIdentifier` or text such as `"ai:1"`) work too.
Refuses `copy` and `pickle`.

Properties:

- `objects: tuple[ObjectIdentifier, ...]`: The objects, in the device's order.
- `indexed: bool`: True when the list was read an element at a time, because the device couldn't send it in one answer.
- `stopped: Optional[str]`: Why an element-by-element read ended early, as text; None when the whole list was read. `objects` then holds the elements before it.

### ObjectList.__len__

```python
def __len__(self) -> int
```

### ObjectList.__iter__

```python
def __iter__(self) -> Iterator[ObjectIdentifier]
```

### ObjectList.__getitem__

```python
def __getitem__(self, index: int) -> ObjectIdentifier
```

### ObjectList.__contains__

```python
def __contains__(self, object: ObjectIdentifier | str) -> bool
```

## DeviceSummary

A device's summary (`Device.summary`, #2561): its instance, and
the Device object's identifying properties (name, vendor, model,
firmware, protocol revision, what it supports). Refuses `copy` and
`pickle`.

Properties:

- `instance: int`: The device instance.
- `name: Optional[str]`: The device's Object\_Name with control characters removed, for display; None when the device gave none. Text the device supplied.
- `readings: Readings`: The properties read: one the device refused has its `error` set.

### DeviceSummary.value

```python
def value(self, property: PropertyIdentifier | str) -> Optional[Value]
```

The `Value` of `property` of the Device object
(`"vendor-name"`, `PropertyIdentifier.MODEL_NAME`); None when
the summary doesn't read it or the device gave none.

## Discovered

One device that answered a `BACnetClient.discover()` (#2561). A
`BACnetClient.device()` target, which pins the handle to `address`.
Refuses `copy` and `pickle`.

Properties:

- `address: str`: Where the device answers, as text `device()` reads back: `"192.168.1.10:47808"`, `"mstp:5"`, `"bip6:0x0004D2"`, an SC VMAC in hex, or a routed station as `"2:0x05"`.
- `network: int`: The network it is on: 0 for this client's own, else the number of the remote network a router passed its answer on from.
- `instance: int`: Its Device instance.
- `duplicate: bool`: True when another address claims the same instance: name this one by its address, or by this row, never by the instance.
- `entry: DiscoveredDevice`: Its row in the discovery table: the largest APDU it takes, its segmentation support and its vendor.

## PriorityView

An object's commands as read: its Priority\_Array's sixteen slots,
the slot in control, Relinquish\_Default and Present\_Value
(`Device.priority_array` and `Device.relinquish`, #2561). Refuses `copy` and `pickle`; its values
copy and pickle.

Properties:

- `object_id: ObjectIdentifier`: The object read.
- `slots: tuple[Optional[Value], ...]`: The sixteen slots, priority 1 first (`slots[7]` is priority 8): each a `Value`, or None for an empty slot.
- `in_control: Optional[int]`: The slot in control: the lowest-numbered priority holding a command. None when every slot is empty, so Present\_Value follows Relinquish\_Default.
- `relinquish_default: Optional[Value]`: Relinquish\_Default; None when the device gave none.
- `present_value: Value`: Present\_Value.

### PriorityView.slot

```python
def slot(self, priority: int) -> Optional[Value]
```

The command at `priority` (an int from 1 to 16), None when its
slot is empty. Another int raises ValueError, a bool or a non-int
TypeError.

## Written

A write the device acknowledged, as it was sent (#2561): what
`Device.write` returns, and a `BacnetReadBackError`'s and a
`BacnetNoPriorityArrayError`'s `written`. Refuses `copy` and
`pickle`.

Properties:

- `address: str`: The device's address the write went to: `"ip:port"`, `"mstp:N"`, a B/IPv6 VMAC (`"bip6:0x0004D2"`), or on BACnet/SC its VMAC in hex (`"02:00:00:00:00:04"`), or a routed station as `"2:0x05"`: text `BACnetClient.device` reads back.
- `object_id: ObjectIdentifier`: The object written.
- `property_id: PropertyIdentifier`: The property written.
- `array_index: Optional[int]`: The array index written, None for the whole property.
- `value: Value`: The value as sent: NULL for a relinquish.
- `priority: Optional[int]`: The priority sent: the one asked for, or 16 for a property the standard makes commandable when none was. None when none was sent: a property that isn't commandable, or one that may be, written without one (the device then takes it at 16 if it is).
- `commandable: Literal["always", "optional", "never"]`: Whether the property takes priorities, as its object type and identifier say with nothing read: `"always"` (an output's Present\_Value), `"optional"` (a value object's, commandable only when the object has a Priority\_Array) or `"never"`.

## WritePreview

What `Device.write` of the same arguments would do, read first and
sent nowhere (`Device.preview_write`, #2561). Refuses `copy` and
`pickle`.

Properties:

- `address: str`: Where the read went, and the write would go, as `Device.address` writes it.
- `object_id: ObjectIdentifier`: The object it would write.
- `property_id: PropertyIdentifier`: The property it would write.
- `array_index: Optional[int]`: The array index it would write, None for the whole property.
- `value: Value`: The value as it would be sent.
- `priority: Optional[int]`: The slot it would take: the priority asked for, or 16, on a property the read found commandable; None on one it found isn't.
- `occupancy: Literal["empty", "held", "not-commandable", "unknown"]`: Who holds that slot now: `"empty"`, `"held"` (the write replaces a command, without telling whoever sent it), `"not-commandable"`, or `"unknown"` when the device wouldn't give its slots.
- `before: Optional[Value]`: What the write replaces: the slot's command when one holds it, or the property's value for one that isn't commandable. None for an empty slot, or when the device refused the read.
- `effect: Literal["changes", "unchanged", "not-present-value", "unknown"]`: What the write does to Present\_Value: `"changes"` (see `changes_from` and `changes_to`), `"unchanged"` (a stronger slot keeps control when `held_by` names one), `"not-present-value"` for a write to another property, or `"unknown"`.
- `changes_from: Optional[str]`: Present\_Value's text now, when `effect` is `"changes"`.
- `changes_to: Optional[str]`: Present\_Value's text after the write, when `effect` is `"changes"`.
- `held_by: Optional[int]`: The stronger priority that keeps control, when `effect` is `"unchanged"` because of one.
- `priorities: Optional[PriorityView]`: The object's commands as read, for a commandable property.
- `denied: Optional[BacnetWriteDeniedError]`: The `BacnetWriteDeniedError` that `Device.write` would raise, never raised here, when the client's write policy refuses the priority; else None.

## Device

One device, as a `BACnetClient` reaches it: what `await client.device(target)` returns (#2561). Its methods are the high-level
calls.

Made from an address (`"192.168.1.10"`, `"mstp:5"`, `"2:0x05"`, a
`discover()` row) it is pinned there. Made from an instance (`1234`,
`"2:1234"`) it follows the instance: when the device stops answering
at its address and one other address claims the instance, the handle
moves there. Its calls go to the device one at a time.

An object is an `ObjectIdentifier` or text (`"ai:1"`,
`"analog-input:1"`); a property a `PropertyIdentifier` or text
(`"pv"`, `"present-value"`, `"priority-array[8]"`). A wrong type
raises TypeError and text the grammar refuses ValueError, when the call
is made.

A handle belongs to its client and keeps nothing of it between calls,
so a `Device` left alive never holds a `stop()` back. After the
client's `stop()` a call raises RuntimeError, as any call on a stopped
client does, and `address` and `instance` say what was last known.
After the client starts again, the next call makes the handle again
from what it was named by, and it works as before: an address at once,
an instance by looking it up afresh, which raises `BacnetLookupError`
until the client has heard from the device again. Refuses `copy` and
`pickle`.

Properties:

- `address: str`: Where the device answers now, as text `BACnetClient.device` reads back: a pinned handle's address, or where a following handle last reached its instance.
- `instance: Optional[int]`: The device instance, when known: from discovery, the instance the handle follows, or an earlier `summary()` or `resolve_instance()`. None for a handle made from an address nothing has named yet.
- `follows: bool`: True when the handle follows its instance, False when it is pinned to its address.

### Device.cov_support

```python
def cov_support(
    self,
) -> Literal["unknown", "not-offered", "offered", "accepted", "refused"]
```

What is known of the device's COV support, with nothing sent:
`"not-offered"` or `"offered"` from the services `summary()`
read, `"accepted"` or `"refused"` from a subscription a watch
made, else `"unknown"`, as it is while the client is stopped.

### Device.read

```python
def read(
    self,
    object: ObjectIdentifier | str,
    property: PropertyIdentifier | str | None = None,
    *,
    index: Optional[int] = None,
) -> Awaitable[Value]
```

Read one property: one ReadProperty.

`property` is Present\_Value when left out, and `index` an array
element (0 is an array's size). With no `property`, `object`
may be both as text: `"ai:1 pv"`. The text of the value names no
units and no state: `read_many` with the object's units or state
texts in the same call names them.

On await, the device's refusal raises `BacnetProtocolError`,
`BacnetRejectError` or `BacnetAbortError`; no answer
`BacnetTimeoutError` (where `read_property` raises
`BacnetAbortError`, reason 10); a following handle whose device
can't be placed any more `BacnetLookupError`. ALL, REQUIRED and
OPTIONAL raise ValueError: `read_object` reads those.

### Device.read_many

```python
def read_many(
    self,
    properties: Iterable[str | ObjectIdentifier | tuple[ObjectIdentifier | str, PropertyIdentifier | str] | tuple[ObjectIdentifier | str, PropertyIdentifier | str, int]],
) -> Awaitable[Readings]
```

Read many properties of any of the device's objects, in batches
of ReadPropertyMultiple (up to 16 objects and 32 properties each;
ReadProperty each once the device has refused that), in the order
asked for.

Each item is text (`"ai:1 pv"`, `"ai:1"`), an
`ObjectIdentifier` (its Present\_Value), or an `(object, property)` or `(object, property, index)` tuple of the forms
`read` takes. A value's text names its object's units, and its
state by the object's texts, when the same call reads them.

A property the device refuses, or one of an object it doesn't
have, never raises: its `Reading.error` is set and the rest
stand. A device that stops answering part way ends the read with
what was read, and `Readings.stopped` says so; so does a later
request the client's own link fails (a route lost, a send
refused). A read raises only when nothing could be read: a bad item
TypeError or ValueError when the call is made; on await a device
that can't be placed `BacnetLookupError`, and a link that fails
the first request what a single read raises for it (a
`BacnetError`).

### Device.read_object

```python
def read_object(
    self,
    object: ObjectIdentifier | str,
    properties: Optional[Iterable[PropertyIdentifier | str]] = None,
) -> Awaitable[Readings]
```

Read one object's properties in one ReadPropertyMultiple (or by
ReadProperty each once the device has refused that): every property
when `properties` is left out, else those listed, whole
properties only. A property the device refused has its
`Reading.error` set. A device that stops answering raises
`BacnetTimeoutError` and keeps nothing, since an object's rows go
together: `stopped` is always None.

### Device.write

```python
def write(
    self,
    object: ObjectIdentifier | str,
    property: PropertyIdentifier | str,
    value: str | int | float | PropertyValue,
    *,
    priority: Optional[int] = None,
    index: Optional[int] = None,
) -> Awaitable[Written]
```

Write one property: one WriteProperty, acknowledged.

`value` is text in the value grammar, read for the property's
datatype (`"72.5"`, `"active"`, `'"a string"'` in quotes,
`"72.5@8"` with a priority), an int or a float (read as its
text; a bool raises TypeError), or a typed `PropertyValue`, sent
as it is. The priority is `priority`, or the one the text names,
or both when they agree. A property the standard makes commandable
(an output's Present\_Value) goes at the priority given, or at 16;
one that may be (a value object's) at the priority given, or with
none; any other with none. `Written.priority` says what went.

Raised with nothing sent: TypeError or ValueError when the call is
made, for a wrong type, bad text or a priority outside 1 to 16; on
await ValueError for a value the property's datatype refuses, two
different priorities, a priority on a property that isn't
commandable, NULL to a commandable Present\_Value (use
`relinquish`) and any write to Priority\_Array; and
`BacnetWriteDeniedError` for a priority the client's
`write_policy` refuses (1, 2, 5 and 6 unless allowed). Then the
device's refusal raises the usual protocol, reject or abort error,
and no answer `BacnetTimeoutError`: the write may or may not have
been applied, and is never sent again.

### Device.preview_write

```python
def preview_write(
    self,
    object: ObjectIdentifier | str,
    property: PropertyIdentifier | str,
    value: str | int | float | PropertyValue,
    *,
    priority: Optional[int] = None,
    index: Optional[int] = None,
) -> Awaitable[WritePreview]
```

What `write` of the same arguments would do, read first and
sent nowhere: the slot it takes and who holds it, whether a
stronger slot keeps Present\_Value where it is, and what
Present\_Value becomes. One request. It raises what `write` raises
before sending, except for the write policy, which
`WritePreview.denied` reports instead.

### Device.relinquish

```python
def relinquish(
    self,
    object: ObjectIdentifier | str,
    priority: int,
) -> Awaitable[PriorityView]
```

Release a command: NULL written to `object`'s Present\_Value at
`priority`, so the next slot down, or Relinquish\_Default, takes
over; then the object's priorities as read just after. One
WriteProperty, then one ReadPropertyMultiple, or a ReadProperty of
each of the three properties when the device refuses that.

`priority` is required and is an int from 1 to 16: a bool, a
float or anything else raises `TypeError`, and another int
`ValueError`, when the call is made. On await, with nothing sent:
an object type with no Priority\_Array (an input, or a Channel)
raises `ValueError`, and a priority the client's `write_policy`
refuses (1, 2, 5 and 6 by default) `BacnetWriteDeniedError`. Then
the device's refusal of the write raises the usual protocol, reject
or abort error, and no answer `BacnetTimeoutError`. A relinquish
the device acknowledged whose read after it failed raises
`BacnetReadBackError`, and one whose read says the object has no
Priority\_Array `BacnetNoPriorityArrayError`; both say what was
written (`written`), which needs no second send.

### Device.priority_array

```python
def priority_array(self, object: ObjectIdentifier | str) -> Awaitable[PriorityView]
```

Read `object`'s commands: its Priority\_Array,
Relinquish\_Default and Present\_Value in one ReadPropertyMultiple (a
ReadProperty of each when the device refuses that). An object type
with no Priority\_Array raises `ValueError` on await with nothing
sent; a device that says the object has none
`BacnetNoPriorityArrayError`; no answer `BacnetTimeoutError`.

### Device.objects

```python
def objects(self) -> Awaitable[ObjectList]
```

The device's Object\_List: whole, or an element at a time when
the device can't send it in one answer.

### Device.summary

```python
def summary(self) -> Awaitable[DeviceSummary]
```

The device's summary: its name, vendor, model, firmware,
protocol revision and what it supports, in one request. A handle
that doesn't know its instance reads that first.

### Device.resolve_instance

```python
def resolve_instance(self) -> Awaitable[int]
```

The device instance: as known (`instance`), or from one read
of the Device object by the wildcard instance, remembered
afterwards.

### Device.moved_to

```python
def moved_to(self) -> Awaitable[Optional[str]]
```

For a pinned handle whose instance is known: the one other
address its instance is heard at now, while its own has gone quiet,
as text `BACnetClient.device` reads back; else None. Always None
for a following handle, which moves itself. Sends nothing.

## WritePolicy

Which command priorities a `BACnetClient`'s `relinquish` may use,
given with its `write_policy=` keyword (#2561).

`WritePolicy()` refuses 1 and 2 (life safety, Table 19-1), 5
(critical equipment) and 6 (minimum on and off times, Clause 19.2.3):
the four a program should not reach by accident. `allow_priorities`
gives a policy that allows some of them as well. The check costs no
traffic: a refused call raises `BacnetWriteDeniedError` and sends
nothing. `write_property` and the other typed calls don't look at
it. Frozen (the class too: `GUARDED` can't be reassigned), equal to
another policy refusing the same priorities, and hashable.

Usage:

```python
policy = WritePolicy().allow_priorities([1])
async with BACnetClient(write_policy=policy) as client:
    await client.relinquish("192.168.1.100:47808", oid, 1)
```

Attributes:

- `GUARDED: tuple[int, ...]`: The priorities the default refuses: `(1, 2, 5, 6)`.

Properties:

- `refused: tuple[int, ...]`: The priorities refused, 1 first.

### WritePolicy.__init__

```python
def __init__(self) -> None
```

The default policy, refusing `GUARDED`.

### WritePolicy.allow_priorities

```python
def allow_priorities(self, priorities: Iterable[int]) -> WritePolicy
```

The same policy, allowing `priorities` (ints from 1 to 16) as
well; this one is unchanged. One that isn't an int raises TypeError,
a bool included, and an int outside 1 to 16 ValueError.

### WritePolicy.allows

```python
def allows(self, priority: int) -> bool
```

Whether a relinquish at `priority` may go.

## CovNotification

A Change-of-Value notification about one object received from a remote
device: a ConfirmedCOVNotification or UnconfirmedCOVNotification, which
`subscribe_cov` and `subscribe_cov_property` subscriptions bring.

Properties:

- `kind: Literal["object"]`: `"object"`; a `CovNotificationMultiple` reads `"multiple"`.
- `subscriber_process_identifier: int`
- `initiating_device_identifier: ObjectIdentifier`
- `monitored_object_identifier: ObjectIdentifier`
- `time_remaining: int`
- `delivery: str`
- `source_mac: bytes`
- `source_network: Optional[int]`
- `source_address: Optional[bytes]`
- `values: Any`: List of property value change entries: dicts with `property_id`, `array_index` and `value`. `value` is a `PropertyValue` shaped as a read result is, or `bytes` when the octets' framing is broken.

## CovNotificationMultiple

A Change-of-Value notification about several objects received from a
remote device: a ConfirmedCOVNotificationMultiple or
UnconfirmedCOVNotificationMultiple, which `subscribe_cov_property_multiple`
subscriptions bring (#2654). It arrives on `cov_notifications()` beside
`CovNotification` objects, in the order they came.

Properties:

- `kind: Literal["multiple"]`: `"multiple"`; a `CovNotification` reads `"object"`.
- `subscriber_process_identifier: int`
- `initiating_device_identifier: ObjectIdentifier`
- `time_remaining: int`
- `timestamp: Optional[tuple[tuple[int, int, int, int], tuple[int, int, int, int]]]`: When the last change the notification conveys happened, as `((year, month, day, day_of_week), (hour, minute, second, hundredths))`, the year in full; `None` when no value carries a time of change.
- `delivery: str`
- `source_mac: bytes`
- `source_network: Optional[int]`
- `source_address: Optional[bytes]`
- `objects: Any`: The objects reported, in order: dicts with `monitored_object_identifier` and `values`. Each value is a dict with `property_id`, `array_index`, `value` (as in `CovNotification.values`) and `time_of_change`, an `(hour, minute, second, hundredths)` tuple, or `None` when the subscription didn't ask for timestamps. One property may appear more than once, a timestamped one changing several times.

## CovNotificationIterator

Async iterator yielding `CovNotification` and
`CovNotificationMultiple` objects, in the order they arrived; `kind`
tells them apart. One that falls behind
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
def __anext__(self) -> Awaitable[CovNotification | CovNotificationMultiple]
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

## RoutingTableEntry

One entry of a router's routing table (#2612), frozen. Refuses
`copy` and `pickle`.

Properties:

- `network: int`: The network the entry is for.
- `port_id: int`: The router's identifier for the port that reaches `network`.
- `port_info: bytes`: The port information octets, which the standard defines only as an octet string (typically dial-up details for a point-to-point link); empty when the entry has none.

## RoutingTable

A router's routing table, as `BACnetClient.routing_table()` read it
(#2612), frozen. Refuses `copy` and `pickle`.

Properties:

- `entries: list[RoutingTableEntry]`: The entries in the order the router listed them, across every acknowledgement. Empty for a router that answered with an empty table: a router that did not answer raises instead.
- `acks: int`: How many acknowledgements carried the entries.
- `truncated: bool`: Whether the call stopped at its `total` or `max_entries` bound, or lost acknowledgements, rather than after a quiet interval. It covers those cases only: a quiet end does not prove the table whole (on MS/TP, acknowledgements after the first wait for the router's token), so use a larger `quiet` on a slow trunk. A late acknowledgement of an earlier query to the same router can also be counted by the next one (nothing pairs answers with questions), so leave a gap between repeated queries; entries are reported as received, so a network number can appear twice.

## RoutedPathLimit

One routed path, a router on the local link and a destination
network, with the size limit the client holds for it, as
`BACnetClient.routed_path_limits()` lists it (#2667), frozen. Its
`repr` writes the router in the link's own form. Refuses `copy` and
`pickle`.

Properties:

- `router: bytes`: The router's MAC on the local link, as `DiscoveredRouter.mac` gives it; the routed-path calls take it back.
- `network: int`: The destination network.
- `max_npdu: int`: The limit requests on the path are sized to, in NPDU octets: the configured limit (228 without one), lowered to the learned one.
- `source: Literal["default", "configured", "learned"]`: Where `max_npdu` comes from: `"default"` (228 octets), `"configured"`, or `"learned"` from a router's too-long reject when that is lower than the configured limit.
- `configured_max_npdu: Optional[int]`: The limit set with `configure_routed_path_max_npdu`, or None.
- `learned_max_npdu: Optional[int]`: One octet under the smallest frame a router refused as too long on this path, or None; never below 228.
- `below_minimum_reject_seen: bool`: Whether a too-long reject for a request of 228 NPDU octets or fewer was dropped on this path since its limit was last set or cleared: the sign of a router that carries less than every router must, whose requests end by timeout.

## NetworkReport

A network-layer report this client received (#1850), frozen. Fields
that don't apply to its `kind` are None.

Properties:

- `kind: Literal['i_am_router_to_network', 'network_number_is', 'reject_message_to_network', 'routing_table', 'other']`
- `source_mac: bytes`: The link-layer peer that sent it: for an I-Am-Router-To-Network, the router to send through.
- `source_network: int | None`: SNET, when a router passed it on from another network.
- `source_address: bytes | None`: SADR, when a router passed it on from another network.
- `networks: list[int] | None`: The networks an I-Am-Router-To-Network lists, in its order.
- `network: int | None`: A Network-Number-Is's network, or the one a refused message was for.
- `configured: bool | None`: Whether a Network-Number-Is sender had its number configured.
- `entries: list[RoutingTableEntry] | None`: The entries one Initialize-Routing-Table-Ack lists, for the `"routing_table"` kind (#2612).
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

`bbmd` names a B/IP BBMD as `"ip"`, `"host"` or either with
`":port"`, or a B/IPv6 one as `"ip"`, `"[ip]"` or `"[ip]:port"`
(port 47808 by default; an IPv6 port needs the brackets, since
`"2001:db8::10:4780"` is read as a bare address); the owner's
transport must match, else its constructor raises ValueError. A host
name (#2308) serves either transport (B/IPv6 since #2365): it is looked
up when the owner starts and again alongside its registrations; one
that doesn't resolve still starts the owner, whose registration waits,
tried again every 30 seconds. A B/IPv6 owner bound to `::` picks its
source address from the route to the BBMD, so its start fails instead;
give it an interface to start and wait. A B/IPv6
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
to be answered, and `foreign_registration()` reports how it goes. A
B/IPv6 client needs `ipv6_device_instance`: a B/IPv6 foreign device
uses its instance's VMAC. While a B/IPv6 one waits for its BBMD's name,
its broadcasts raise BacnetError.

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

## RoutedTarget

A station on another BACnet network (#2612), where `BACnetClient`'s
`read_property`, `read_property_multiple`, `read_range`,
`read_log_page`, `write_property`, `atomic_read_file`,
`atomic_write_file`, `subscribe_cov`, `unsubscribe_cov`,
`subscribe_cov_property`, `unsubscribe_cov_property` and
`subscribe_cov_property_multiple` take an `address` (#2617, #2613,
\#2588); the other typed calls take a local address only:
`RoutedTarget(network, address, router=None)`, in that order. A
`str` address always names a station on the local link.

`network` is the station's network number, an `int` from 1 to 65534
(ValueError outside it, OverflowError outside 0 to 65535, TypeError for
a `bool`). `address` is the station's MAC on that network, 1 to 18
octets: `bytes` as given, such as a `DiscoveredDevice`'s
`source_address`, or a string in the binding's address forms:
`"ip:port"` (a B/IP station's MAC is its IPv4 address and UDP port on
any B/IP network, local or not), `"bip6:N"` or `"bip6:0x..."` (a
B/IPv6 VMAC), two or more hex octets `"aa:bb:..."`, or one octet in
decimal as `"N"` or `"mstp:N"` (0 to 254), so
`RoutedTarget(5, "10") == RoutedTarget(5, b"\x0a")`. The string forms
refuse 255 (MS/TP's broadcast); `b"\xff"` is taken, a station on an
ARCNET network, whose broadcast is 0. A `router` that is a broadcast
or group address on the local link raises BacnetError when a call is
made, before anything is sent.

`router` is the next hop's address on the local link, in the same
forms or as `bytes`, such as a routed `DiscoveredDevice`'s
`mac_address`, or `BACnetClient.discover_routers()`' `mac`; each
request goes through it. Without one, the client uses the route it
knows: the router its discovery table names for the station, after
one Who-Is addressed to that station when it has none, as
`BACnetClient.device()` does. A station that doesn't answer raises
`BacnetLookupError`, and for 30 seconds a call to it with no router
raises at once without asking again, unless an I-Am from it arrives
first; calls made together for one station share one Who-Is. To scan
stations that never announce themselves, such as MS/TP slaves, pass
`router` from `BACnetClient.discover_routers()`: without it each
costs one 3-second Who-Is wait. A station on the client's own network,
once it knows the number, is sent to directly either way. Other values raise ValueError, a type
other than `str` or `bytes` TypeError, and so does a call in another
order, since `network` takes an `int`.

Frozen; compares and hashes by its network and octets, however they were
written; copies and pickles.

Properties:

- `network: int`
- `address: bytes`: The station's MAC on its network.
- `router: bytes | None`: The router's MAC on the local link, or None for the route the client knows.

### RoutedTarget.__init__

```python
def __init__(
    self,
    network: int,
    address: str | bytes,
    router: str | bytes | None = None,
) -> None
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

## ShedLevel

`class ShedLevel(TypedDict)`

A Load Control shed level (`BACnetShedLevel`), the form a typed
read of Requested\_Shed\_Level gives: a `percent` or a `level` with an
`int` value, or an `amount` of kilowatts with a `float`.

Keys:

- `kind: Literal["percent", "level", "amount"]`
- `value: int | float`

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

## AuditOtherDatum

`class AuditOtherDatum(TypedDict)`

A datum under a tag the standard doesn't assign, as received (#2483).

Keys:

- `kind: Literal["other"]`
- `tag: int`
- `constructed: bool`
- `data: bytes`

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
each of which brings the next attempt forward. A BBMD named by host name
(#2308) counts its lookups in `bbmd_lookups`, the failed ones in
`bbmd_lookup_failures`, and names why the newest failed in
`last_bbmd_lookup_failure` (`"not_resolved"`, `"timed_out"` after
10 seconds, `"no_ipv4_address"`, `"unusable_address"`,
`"abandoned"`, or `"other"` for a failure this version doesn't
name), None once one succeeds and cleared when the owner
stops. `waiting_for_bbmd` is true while that name hasn't resolved yet:
no attempt is due, and it is tried again 30 seconds after each
failure.

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
- `bbmd_lookups: int`
- `bbmd_lookup_failures: int`
- `last_bbmd_lookup_failure: Optional[Literal['not_resolved', 'timed_out', 'no_ipv4_address', 'unusable_address', 'abandoned', 'other']]`
- `waiting_for_bbmd: bool`

## Bip6ForeignRegistrationStatus

`class Bip6ForeignRegistrationStatus(TypedDict)`

A B/IPv6 foreign device's registration, as `foreign_registration()`
reports it on a `transport="ipv6"` owner (#2365). `bbmd` is the
address in use, `"[ip]:port"`, or
None while a host name hasn't resolved yet, when `waiting_for_bbmd` is
true. `bbmd_name` is the host name the BBMD is known by,
`"host:port"` (configured, or written to FD\_BBMD\_Address), or None for
a BBMD known by address; a waiting device waits for it. While waiting,
nothing is registered, broadcasts are refused (counted in
`broadcasts_refused`), and the name is tried again 30 seconds after
each failure. `bbmd_lookups`, `bbmd_lookup_failures` and
`last_bbmd_lookup_failure` are as in `ForeignRegistrationStatus`,
with `"no_ipv6_address"` for a name with no IPv6 address. The
`previous_release*` totals count the deletions asked of BBMDs the
device moved away from, by `release_previous_bbmd`: released,
refused, or unanswered. `registrations_sent` counts the
Register-Foreign-Device messages sent. The BBMD's answer to the latest
one (#2388) reads as on B/IP: `last_outcome` is `"accepted"` or
`"refused"`, with `last_result` the BVLC result code (0x0090 for a
refusal), or `"timed_out"` when its renewal went first, the BBMD
silent, or an activation's registration did after 3 s of silence, with
`last_result` None; both are None before the BBMD's first answer, and
clear when the BBMD changes, while waiting, and when the owner stops.
`last_outcome_age` is how many seconds ago that outcome came,
`unanswered` how many registrations in a row the BBMD left unanswered,
and `refused` counts its refusals. The totals survive a stop; outside
`foreign_device=` they are 0.

Keys:

- `bbmd: Optional[str]`
- `bbmd_name: Optional[str]`
- `waiting_for_bbmd: bool`
- `bbmd_lookups: int`
- `bbmd_lookup_failures: int`
- `last_bbmd_lookup_failure: Optional[Literal['not_resolved', 'timed_out', 'no_ipv6_address', 'unusable_address', 'abandoned', 'other']]`
- `previous_releases: int`
- `previous_released: int`
- `previous_release_refused: int`
- `previous_release_unanswered: int`
- `broadcasts_refused: int`
- `registrations_sent: int`
- `last_outcome: Optional[Literal["accepted", "refused", "timed_out"]]`
- `last_result: Optional[int]`
- `last_outcome_age: Optional[float]`
- `refused: int`
- `unanswered: int`

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

## ShedStatus

```python
ShedStatus = tuple[Literal["waiting", "shedding", "compliant"], ShedLevel, ShedLevel]
```

What a Load Control's `shed` callable answers (#2479): where the loads stand, then the expected and the actual level, in the requested choice.

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
AuditLogDatum = AuditLogStatusDatum | AuditNotificationDatum | AuditTimeChangeDatum | AuditOtherDatum
```
<!-- generated:end -->
