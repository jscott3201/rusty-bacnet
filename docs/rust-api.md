# Rust API Reference

Rusty BACnet is a workspace of library crates implementing the BACnet protocol stack (ASHRAE 135-2020). The [README crate table](../README.md#crates) lists which are on crates.io.

This reference follows the `dev` branch. At the `v0.12.0` tag it describes the
published 0.12.0 crates, which the site's tutorials also target; see
[docs.rs](https://docs.rs/bacnet-client/0.12.0/bacnet_client/) and the
[installation guidance](../README.md#install). Changes merged after the release
wait in [`changelog.d/`](../changelog.d/); to use them, follow
[Build from source](../README.md#build-from-source).

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

**Key enums:** `ObjectType` (u32), `PropertyIdentifier` (u32), `ErrorClass` (u16), `ErrorCode` (u16), `EnableDisable` (u32), `ReinitializedState` (u32), `Segmentation` (u8), `EventState` (u32), `EventType` (u32), `NotifyType` (u32), `Polarity` (u32), `Reliability` (u32), `LifeSafetyOperation` (u32), `MessagePriority` (u32), `VTClass` (u32)

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
use bacnet_types::error::{Error, ErrorDetail};

// Protocol error from a remote device
let e = Error::Protocol { class: 2, code: 31 }; // ErrorClass(2)=PROPERTY, ErrorCode(31)=UNKNOWN_PROPERTY

// A structured error body: here an AddListElement/RemoveListElement
// ChangeList-Error naming the failed element. Error::protocol builds
// Error::Structured with a detail, Error::Protocol without one.
let e = Error::protocol(5, 81, Some(ErrorDetail::FirstFailedElementNumber(2))); // SERVICES / LIST_ELEMENT_NOT_FOUND
if let Error::Structured { detail, .. } = &e {
    assert_eq!(**detail, ErrorDetail::FirstFailedElementNumber(2));
}

// Other variants: Timeout, Reject, Abort, RoutedPathTooLong,
// RoutedPathCapacityExceeded, UnsupportedTransport, Encoding, etc.
```

`ErrorDetail` has one variant per shape of structured error body (Clause 21):

| Variant | Body | Fields |
|---------|------|--------|
| `FirstFailedElementNumber(u32)` | ChangeList-Error, CreateObject-Error | Position from 1 of the refused list element or initial value; 0 when no element failed |
| `FirstFailedWriteAttempt(BACnetObjectPropertyReference)` | WritePropertyMultiple-Error | Object, property and index of the first failed write |
| `FirstFailedSubscription(BACnetObjectPropertyReference)` | SubscribeCOVPropertyMultiple-Error, first-failed-subscription choice | Monitored object and the refused COV reference's property and index |
| `PrivateTransfer { vendor_id, service_number, error_parameters }` | ConfirmedPrivateTransfer-Error | The private service, and its encoded error parameters when present |
| `VtSessionIdentifiers(Vec<u8>)` | VTClose-Error with its list | Local identifiers of the sessions that could not be closed |

A body with nothing beyond the error (SubscribeCOVPropertyMultiple's general
choice, VTClose-Error without its list) is `Error::Protocol`.
`bacnet_services::structured_error::detail(&error_pdu)` reads the detail of any
Error PDU, and each body has its own type for encoding and decoding:
`list_manipulation::ChangeListError`, `object_mgmt::CreateObjectError`,
`wpm::WritePropertyMultipleError`,
`cov_multiple::SubscribeCOVPropertyMultipleError`,
`private_transfer::PrivateTransferError` and `virtual_terminal::VTCloseError`.

`Error::Decoding { offset, kind, message }` carries the `DecodingKind` of the
fault a decoder found, and `Error::reject_reason()` names the Reject reason
it draws when it refuses a confirmed request (#1446):

| Fault | Reported as | Reject reason |
|---|---|---|
| Encoding not valid for its datatype (wrong length, empty Unsigned, unknown character set) | `Decoding`, `InvalidEncoding` | INVALID_DATA_ENCODING |
| Value too large for its field, or outside its range | `Decoding`, `OutOfRange` | PARAMETER_OUT_OF_RANGE |
| More items than the decoder takes, or a tag length past its bound | `Decoding`, `Overflow` | BUFFER_OVERFLOW |
| A tag that doesn't fit, a closing tag closing nothing, nesting too deep | `Decoding`, `InvalidTag` | INVALID_TAG |
| The data or the frame ends where a member is due | `Decoding`, `Missing` | MISSING_REQUIRED_PARAMETER |
| A member's contents cut short | `BufferTooShort` | MISSING_REQUIRED_PARAMETER |
| Octets after the last member | `Decoding`, `Trailing` | TOO_MANY_ARGUMENTS |
| A character set the decoder doesn't convert | `Decoding`, `Unsupported` | OTHER |

`Error::into_request_reject()` turns a request's decode error into that
`Error::Reject`. The bundled server answers every confirmed request it can't
decode that way. A decoder that names a fault the table doesn't, such as
GetEnrollmentSummary's undefined enumerations, returns its `Error::Reject`
itself. A decoding error met once a service runs keeps its Error PDU.

`Error::UnsupportedTransport { required, actual }` reports an operation the
endpoint's data link cannot carry, such as a BBMD request through an
`AnyTransport` that is not B/IP. Both fields are a
`bacnet_types::data_link::DataLink` (`Bip`, `Bip6`, `Mstp`, `Sc`, `Ethernet`,
`Loopback`), whose `Display` is the short name, so the message reads
"operation requires BACnet/IP; this transport is MS/TP". Nothing was sent.

`Error::RoutedPathTooLong { dnet }` identifies the destination network from a
matching network-layer rejection; it does not claim an exact supported length.
`Error::RoutedPathCapacityExceeded { capacity }` reports that all bounded path
state is protected by a held/waiting gate or configured/learned evidence, so a
new path was rejected before transaction registration or frame emission.
`Error` is a public enum, so these variants can require new arms in downstream
exhaustive matches. Matchers with a wildcard arm are unaffected.

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

### Object identifiers

`ObjectIdentifier::new` validates the 10-bit object type (0..=1023) and 22-bit
instance (0..=4,194,303). `new_addressable` shares those checks and additionally
rejects the reserved wildcard instance 4,194,303. `ObjectType::from_raw` remains
an unrestricted selector; values above 1023 cannot form an object identifier.
Valid proprietary types and wire wildcard identifiers remain supported.

The safe `ObjectIdentifier::new_unchecked` constructor has been removed. Use
`new` or `new_addressable` and handle the error. The private fields and checked
construction keep encoding infallible without release-mode truncation; wire
decoding also establishes the field-width invariant.

### ValueSource CHOICE

`bacnet_types::constructed::BACnetValueSource` represents `None`,
`Object(BACnetDeviceObjectReference)`, or `Address(BACnetAddress)`.
The Object payload contains a required object identifier and an optional device
identifier; it replaces the earlier bare ObjectIdentifier payload.

`bacnet_encoding::constructed::encode_value_source(&mut BytesMut, &BACnetValueSource)`
returns `Result<(), Error>` and appends one framed CHOICE. Object-identifier widths
are validated at construction. An address MAC longer than
`BACnetAddress::MAX_MAC_LEN` (18) octets is refused with `Error::Encoding` before
the buffer changes, and the decoder refuses one as malformed (#1156).
`decode_value_source(&[u8], offset)` returns `Result<(BACnetValueSource, usize), Error>`;
the second value is the next absolute offset, and suffix bytes remain available.
A consumer decoding a complete property payload must check that this offset equals
the payload length. Array or stream consumers can decode subsequent choices.

This generic datatype preserves wire-valid object types and wildcard instances,
network zero and empty broadcast MAC addresses. A source claim does not establish
an actual or authorized command origin. Existing BACnetTimeStamp codecs remain
the timestamp encoding authority.

### Command-source tracking

Analog Output, Binary Output and Multi-state Output, plus commandable instances
of the corresponding Value families, implement `Value_Source`, the 16-element `Value_Source_Array`, and `Last_Command_Time`.
These paired properties are required while this mechanism is enabled, including
in Property_List, REQUIRED RPM selection and PICS. Sources and the timestamp are
returned as `PropertyValue::ApplicationData` containing their BACnet CHOICE bytes;
source-array index 0 returns Unsigned 16. `Command_Time_Array` is not implemented.

A standalone command uses `BACnetObject::write_property_from` with an explicit
`bacnet_objects::command_source::CommandOrigin`. Remote origins contain the actual
BACnet address and an Unknown, Unique(Device), or Ambiguous correlation snapshot.
Local origins contain a concrete owning Device and an optional concrete initiating
object. Standalone calls validate syntax and trust the caller's declaration;
they do not authenticate it or check database membership. Context-free
`write_property` denies Present_Value commands and Value_Source corrections on
these six commandable families. Noncommandable Value modes use the direct write
contract described below and do not require command provenance.
`AnalogValueObject::set_present_value` was removed: configure
`set_relinquish_default` for a fallback or submit a sourced priority command.
Input measurement setters keep their separate contract. Priority_Array stays
read-only; a sourced Present_Value NULL relinquishes the specified priority.

The full server derives remote origins from direct network 0/source MAC or routed
SNET/SADR, independently of Audit reporting. Address-to-Device correlation is a
snapshot, not authentication. Once the server knows its own network number, a
local binding, or a Device binding routed through that number, names a request
from its MAC with no SNET and one a router relays with that number and MAC as
SNET and SADR, for the command origin and the target Audit record alike (#1404).
WP, WPM and CreateObject initial commands use that origin. Schedule commands name
the initiating Schedule and preserve complete target references; Staging commands
name the actual plan source after its existing generation check. Failed CreateObject initialization rolls back the new object;
WPM retains its successful prefix and failed coordinate.

`BACnetServer::write_local` requires a final `LocalCommandSource` argument:
`ServerDevice` or `Object(oid)`. Both require a selected concrete local Device for
tracked commands; the object form also requires an existing concrete local
initiator under the database guard. Missing or wildcard-only Device identity and
missing initiators fail closed. Unrelated local writes retain their behavior
without a Device. The selected Device owns correction rights; changing the local
initiator changes the published source but not that owner. Custom objects retain
the default generic writer unless they implement the new hook. Writable
decorators must forward it, as the endpoint source-reporting decorator does.
The endpoint inbound write allowlist is unchanged.

`BACnetServer::write_local_encoded` takes the value as the octets a network
WriteProperty would carry, with the same `LocalCommandSource`. It applies that
handler's array-index check and per-property decoding, then the `write_local`
path, so a value from `read_local`, encoded, writes back (the Python
`write_property_local` uses it).

Each priority retains its original command owner separately from its correctable
source claim. Remote correction requires the same uniquely known Device at both
operations, or the same actual address without conflicting known identities or
ambiguity. An originally unknown command cannot gain cross-address rights through
a later binding. Expiry permits retained-address fallback; ambiguity denies
correction. Router hop changes alone do not change the original routed address.
Local correction requires the same owning Device, independently of initiator;
remote and local owners cannot correct one another. An authorized owner may
assert any complete, valid ValueSource CHOICE, including none or a forwarded
object/address. Payload claims do not prove ownership. Correction preserves the
original token; a new command replaces it.

Last_Command_Time is an object-owned u16 SequenceNumber, initially 0, incremented
with wraparound only when a successful Present_Value command or relinquishment
changes the effective `(value, active priority, source)` tuple. Noncurrent-only
writes, source corrections and fallback configuration do not increment it.
A NULL command retains the relinquishing writer in that slot's source (the
selected interpretation of the last command), while the visible source moves to
the next active slot or none.

Single and Multiple subscriptions to commandable `Value_Source` on these six
families report `Present_Value`, `Status_Flags`, `Value_Source`,
`Last_Command_Time`, and `Current_Command_Priority` together. The trigger uses
the object's PV criterion (its `COV_Increment` for analogs), flags, source, or
priority changes; time alone does not trigger. A Value_Source subscription's
increment does not replace the analog object's increment. Initial and renewal
reports contain the same five fields. A failed or malformed required companion
suppresses that reference without advancing its delivered baseline; valid
Multiple siblings continue. Overlapping Multiple selectors share captured values
and deduplicate report fields, while only qualifying references advance their
own baselines and contribute timestamps. A qualifying explicit property selector
controls its field's timestamp, including an explicit false choice, and an
explicit false selector keeps its field untimestamped even when it did not
qualify (#856). For implicit companions only, this implementation merges
timestamp intent from qualifying contributors; an unqualified explicit true
selector gains no authority from that overlap policy.
Existing delivery, lifetime and renewal
fences apply; same-generation concurrent completion ordering is separate (#826).

### Source of a noncommandable Present_Value

Clause 19.5 gives an object with no priority array one source to report: the
writer of its last Present_Value write (#1552). The noncommandable modes of
Analog, Binary and Multi-state Value, and the Color and Color Temperature
objects, track it once `set_value_source_tracking(true)` is called before
registration, as the audit policy is provisioned. Tracking is off by default, so
Property_List, the PICS and the wire stay as they were. On, the object serves
`Value_Source` as a required row (the tables' value-source footnote), starting
at NONE; `Value_Source_Array` and `Last_Command_Time` stay absent, since Clause
19.5.1.4 keeps the time to objects with a priority array.

- `write_property_from` records its origin as source and owner, published as
  for a commandable slot: the Device or local initiator, or the address of a
  writer the server can't tie to one Device. On a colour object a Color_Command
  that sets Present_Value (a fade, a ramp, a step, or a STOP that halts one)
  records its writer too; a STOP with nothing moving doesn't. A permitted NULL
  is a no-op and records nothing, nor does a refused write.
- Only the owner may write `Value_Source`, under the commandable path's
  ownership rules; the priority is ignored. Anyone else, or anyone before the
  first sourced write, gets WRITE_ACCESS_DENIED before the value is checked. A
  correction keeps the owner; the next write replaces it.
- Context-free `write_property` of Present_Value (or Color_Command) is refused
  with WRITE_ACCESS_DENIED while tracking, as a commandable command is, since
  the source it would leave can't be known.
- `BACnetServer::write_local` names this Device, or the `LocalCommandSource`
  object, and the Device owns the source. `set_present_value_local` calls the
  new `set_present_value_from_internal` hook with this Device. The typed
  setters (`set_present_value`, `set_color_command`, a `set_min_max` that moves
  a Color Temperature's value, and plain `set_present_value_internal`) name no
  writer, so they leave Value_Source NONE with no owner.

With tracking on and no local Device identity, a Schedule's or Command's write
to such an object falls back to a context-free write, which is refused: it fails
closed, as it does for a commandable object.

With source tracking enabled, Single and Multiple `Value_Source` subscriptions
on noncommandable Analog, Binary and Multi-state Value objects report
`Present_Value`, `Status_Flags`, and `Value_Source` together (Table 13-1a-2).
Initial and renewal reports include all three, including an initial NONE source
before Present_Value has been written (IC 135-2020-32). Notifications follow the
object's Present_Value criterion, any Status_Flags change, or any Value_Source
change. Analog uses its own `COV_Increment`, irrespective of the Value_Source
subscription's increment. A combined PV write and source correction reports the
final corrected source. The same capture and delivery fences described above
apply; command-only fields remain absent.

Color and Color Temperature retain their existing property-COV behavior. Their
Value_Source companion/trigger contract remains unresolved under #1582 because
the inspected object tables do not define the Status_Flags companion required
by Table 13-1a-2's fallback.

### APDU Types

```rust
use bacnet_encoding::apdu::*;

// Confirmed request, Complex ACK, Simple ACK, Error, Reject, Abort
// Segmentation: SegmentAck, segmented confirmed requests
```

### NPDU

```rust
use bacnet_encoding::npdu::{Npdu, NpduAddress, NpduDecodeError, encode_npdu, decode_npdu};

// Handles source/destination network addresses, hop count, priority
```

DADR and SADR are capped at `NpduAddress::MAX_MAC_LEN` (18) octets, the same
limit as `BACnetAddress::MAX_MAC_LEN` and the longest MAC any built-in data
link uses (B/IPv6) (#1141). `encode_npdu` refuses a longer address with
`Error::Encoding`. `decode_npdu` returns `NpduDecodeError`: a DLEN or SLEN past
the cap is `AddressTooLong { field, length, dnet, source }`, checked before the
address octets are read, and every other malformation is `Malformed(Error)`.
For an over-long DADR, `source` is the SNET/SADR behind it when the frame holds
a complete, valid one, which a router needs to address its reject (#1158). The
error converts into `Error` (an over-long address becomes `Error::OutOfRange`),
so `?` still works in functions that return `Result<_, Error>`.

---

## bacnet-services

23 BACnet service modules with request/response encoding and decoding.

### ReadProperty / WriteProperty

```rust
use bacnet_services::rp::{ReadPropertyRequest, ReadPropertyACK};
use bacnet_services::wp::WritePropertyRequest;
```

`WritePropertyRequest::encode` returns `Result<(), Error>` and validates its
optional priority before modifying the destination buffer. Only omission or
1–16 is accepted, including NULL and noncommandable writes. Direct and routed
client WP paths propagate invalid input as a local `Error::Encoding` before
transaction admission or traffic; device-based calls validate before lookup.
This outbound contract does not change inbound semantic-error responses or
remote commandability rules.


### ReadPropertyMultiple / WritePropertyMultiple

`WritePropertyMultipleRequest::validate` checks the whole outbound request;
`encode` returns `Result` and leaves an existing buffer unchanged on validation
failure. Requests and each object's write list must be nonempty, targets cannot
be ALL/REQUIRED/OPTIONAL, and supplied priorities must be 1–16. Omitted priority,
index zero, proprietary properties, NULL and empty list values remain legal.
Direct and device-directed clients reject invalid requests before admission or
discovery. Inbound cursor/no-op and ordered-prefix error semantics are separate.

For a Device wildcard request `(Device,4194303)`, the bundled server's
ReadPropertyMultiple result wrapper names the resolved local Device, including
wrappers containing per-property errors. Its Object_Identifier value names the
same Device. Without a matching Device, the wrapper retains the wildcard and
its references return UNKNOWN_OBJECT. Concrete requests remain unchanged.

ReadPropertyMultiple response indexes follow the effective object declaration:
requested indexes remain on known arrays, including index zero and inline array
errors; scalar results omit them. Unknown objects/properties or unavailable
legacy declarations conservatively omit the response index. This does not alter
read error precedence or add a property read. Target Audit records retain the
requested index independently.

```rust
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadAccessResult};
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};

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

The low-level `WritePropertyMultipleCursorError.kind` distinguishes
`WritePropertyMultipleFailureKind::Syntax(RejectReason)` from
`PriorityOutOfRange`. This replaces the former `reject_reason` field. The
bundled server returns a formal WPM Error with `SERVICES / PARAMETER_OUT_OF_RANGE`
for a valid Unsigned priority outside 1..16, retaining the failed coordinate and
any successful prefix. Syntax failures retain initial Reject and post-prefix
`INVALID_TAG` behavior. Whole-request `WritePropertyMultipleRequest::decode`
returns `Error::Decoding` for either failure.

### COV

```rust
use bacnet_services::cov::{
    SubscribeCOVRequest, COVNotificationRequest, UnsubscribeCOVRequest,
};
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
```

### Discovery

```rust
use bacnet_services::who_is::{DeviceInstanceRange, WhoIsRequest, IAmRequest};
use bacnet_services::who_has::{WhoHasRequest, WhoHasObject, IHaveRequest};
```

`WhoIsRequest` and `WhoHasRequest` carry their device-instance limits as one
`range: Option<DeviceInstanceRange>`, `None` asking every device (Clauses 16.9
and 16.10, #1483). A range holds both limits, so a request with one alone can't
be built, and `DeviceInstanceRange::new` refuses a low limit above the high one,
or a limit past `ObjectIdentifier::MAX_INSTANCE` (4194303), with
`Error::OutOfRange`. `DeviceInstanceRange::single(n)` asks one instance,
`DeviceInstanceRange::device(oid)` one device's instance, and
`DeviceInstanceRange::from_limits(low, high)` turns two optional limits into a
range, refusing one without the other. Both decoders refuse a request with one
limit, or with its low limit above its high one, and the server drops it
unanswered, counting it in `DiscoveryCounters::malformed_dropped`. A decoder
takes a limit past 4194303 as written, since some devices send one to mean
every device.

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
use bacnet_services::list_manipulation::ListElementRequest;
```

The shared AddListElement/RemoveListElement request exposes `validate()` and
transactional `encode(&mut BytesMut) -> Result<(), Error>`. A supplied array index
must be nonzero, and `list_of_elements` must contain at least one complete encoded
element. Validation checks tag framing with the shared parser limits (1 MiB per
primitive tag and 32 context levels including the service's outer `[3]`). It
honors application Boolean's no-payload encoding and preserves empty-valued,
constructed, context and vendor values. It does not validate every application
primitive or infer the remote property's datatype. Invalid requests leave the
output buffer unchanged; both client methods reject before transaction admission
or traffic. Inbound decoding and target property validation remain separate.

`ChangeListError` is the error body both services answer with (Clause 21): the
error class and code plus `first_failed_element_number`, the position from 1 of
the request element that failed, or 0 when the request failed for another
reason. `to_error_pdu` builds the Error PDU and `TryFrom<&ErrorPdu>` decodes and
checks one. `ErrorPdu` keeps the whole body in `error_data`; a device that sends
only a class and code still decodes, with no element number.


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
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
```

`WriteGroupRequest` follows the WriteGroup-Request production (Clause 21.3.2).
`group_number` is a `NonZeroU32` (group 0 is reserved) and `write_priority` is 1 to 16.
Each `GroupChannelValue` carries a `u16` channel number, an optional override priority
(1 to 16) and the already-encoded BACnetChannelValue in `value`: one
application-tagged primitive, a context-0 lighting command, or Addendum
135-2020ca's context-1 xy colour or context-2 colour command (#1474), with no
wrapper tag. A lighting command is the `encode_lighting_command` octets between
an opening and a closing context tag 0; its priority, when present, must be 1
to 16. The colour alternatives frame the `encode_xy_color` and
`encode_color_command` octets in tags 1 and 2 the same way.
`bacnet_encoding::constructed::constructed_channel_value` says which
constructed alternative some octets hold, as a `ConstructedChannelValue`
(`LightingCommand`, `XyColor` or `ColorCommand`), or `None` for anything else.
`encode` is fallible: it rejects priorities outside 1 to 16, an empty change list and
a value that is not a single BACnetChannelValue with `Error::Encoding`, leaving the
buffer unchanged. `decode` enforces the same rules and rejects trailing data.
The bundled server executes inbound WriteGroup on its Channel objects (see the
Channel paragraphs under Lighting & Color), and `BACnetClient::write_group` sends one.

### Who-Am-I and You-Are

```rust
use bacnet_services::who_am_i::{WhoAmIRequest, YouAreRequest};
```

`WhoAmIRequest` has three mandatory application-tagged fields: `vendor_id` (`u16`),
`model_name` and `serial_number`. `YouAreRequest` has the same three plus optional
`device_identifier` (which must name a Device object) and `device_mac_address`; at
least one of those two must be present. Both `encode` methods are fallible and both
`decode` methods reject missing fields, context-tagged layouts and trailing data.
`device_mac_address` is the MAC the matching device takes on the port the request
arrived on (Clauses 16.11.3.1.5 and 16.11.4), so it is held to
`BACnetAddress::MAX_MAC_LEN` (18 octets) in both directions (#1200): `decode`
refuses a longer one and `encode` returns `Error::Encoding` without writing.

### Virtual Terminal

```rust
use bacnet_services::virtual_terminal::{
    VTCloseRequest, VTDataAck, VTDataRequest, VTOpenAck, VTOpenRequest,
};
```

### Audit

```rust
use bacnet_services::audit::{
    AuditLogQueryAck, AuditLogQueryRequest, AuditNotificationRequest,
    AuditPropertyReference, BACnetAuditLogQueryParameters, BACnetAuditNotification,
};
```

These models encode the corrected 2020 baseline: ANSI/ASHRAE 135-2020 plus
the Errata Summary 2024-04-29 (v1) items 7-8 for the Audit query contract.
In particular, `AuditLogQueryRequest::start_at_sequence_number` is the
corrected `Option<u64>` cursor at unchanged tag [2], and each query
alternative contains `successful_actions_only: BACnetSuccessFilter`
(`ALL`/`SUCCESSES_ONLY`/`FAILURES_ONLY`) at unchanged tags [7]/[4]. Unsigned
values use the library's `u64` implementation limit (1-8 octet canonical
forms). Storage filtering enforces all three states, and the continuation
cursor is literal (only identities below the cursor match, newest-first
insertion order even across `u64::MAX`-to-1 wrap). These codecs are not an
unqualified Clause 13.19 support claim.

---

## bacnet-transport

Transport-layer implementations. All implement the `TransportPort` trait.

### Local receive capacity and outgoing limits

Every `TransportPort` implementation must
provide `local_receive_apdu_capacity() -> u16`, a stable receive declaration.
Transparent wrappers and `AnyTransport` delegate it. The former transport method
`max_apdu_length()` is now `egress_apdu_limit()` without an alias: it describes
the current outgoing path, and SC negotiation/reconnect/failover can change it.
Client budgets continue using egress limits and the client's existing canonical
configuration policy. Unrelated Device, client and configuration APIs retain
their names.

`ServerConfig.max_apdu_length` is a raw receive ceiling. Startup clamps it to the
transport's local capacity, rejects an effective value below 50, and requires the
current selected Device's `Max_APDU_Length_Accepted` to equal that effective
value before starting the transport. The server does not rewrite an
application-owned Device. No Device remains a valid startup configuration but
cannot emit I-Am. Both live I-Am paths recheck the selected Device under the
same database guard; queued spontaneous announcements check at execution before
encoding or limiter accounting. A mismatched replacement refuses announcement,
and a later matching replacement restores it. Database replacement does not
rebind the discovery limiter's startup identity.

Raw declarations need not be header codes: Device/I-Am 1474 stays 1474, while
originated confirmed COV, Audit and Event notifications advertise the floor 1024
in their Confirmed-Request header. The codec helper
`max_apdu_header_at_or_below(u32)` returns the largest code in
50/128/206/480/1024/1476 not exceeding its input, rejects values below 50 and
saturates larger unsigned values at 1476. This conversion does not shrink raw
byte budgets or I-Am values. It is separate from the exact-code encoder API.

Built-in B/IP, B/IPv6, Ethernet and SC declare local APDU 1476; MS/TP declares 480.
Both registered B/IP port snapshots use local capacity independently of the
Device/server ceiling. SC nodes advertise and enforce local NPDU 1478, including
the two-byte plain NPDU header, on Hub, accepted-direct and outbound-direct
intake. Complete BVLC bounds, remote/path limits, routed overhead and the Hub's
forwarding capacity remain independent. These APIs are new in
0.12.0; this bounded evidence is not a full Annex AB or hardware qualification.

### Feature Flags

| Feature | Platforms | Transport |
|---------|-----------|-----------|
| (default) | all | BIP (UDP/IPv4) |
| `ipv6` | all | BIP6 (UDP/IPv6 multicast) |
| `sc-tls` | all | BACnet/SC (WebSocket + TLS) + SC Hub |
| `serial` | all | MS/TP (serial token-passing via `tokio-serial`) |
| `serial-gpio` | Linux | MS/TP + GPIO direction control (adds `gpiocdev`) |
| `ethernet` | Linux | BACnet Ethernet (AF_PACKET raw sockets with a best-effort BPF filter) |

### BIP (IPv4)

```rust
use bacnet_transport::bip::BipTransport;

let transport = BipTransport::new(
    Ipv4Addr::new(0, 0, 0, 0),  // bind interface
    0xBAC0,                       // port (47808)
    Ipv4Addr::BROADCAST,          // broadcast address
);
```

Port zero asks for a private ephemeral port and never sets `SO_REUSEADDR`: on
Linux such a bind could otherwise be given a port another `SO_REUSEADDR`
socket already holds, and unicast to that port then reaches only one of them.
The choice is made at construction, so a restart that rebinds the remembered
actual port keeps it private. B/IPv6 applies the same port-zero and
explicit-port rule, but binds a fresh ephemeral port on each start instead of
remembering one.

By default the transport binds one socket on `0.0.0.0:port`, whatever the
interface: an explicitly requested port sets `SO_REUSEADDR`, and port zero is
private. The wildcard socket receives directed and limited broadcasts and
unicast alike, in the order they arrive, and the receive loop takes unicast
only to the interface address (or, for `0.0.0.0`, to one of the host's
addresses, below). Sends leave from it, so the OS picks their source address
by route. On an explicit port, Linux lets a second application bind the same
wildcard address, with the same single-receiver unicast caveat, so two
devices on two addresses of one host can't both rely on the standard port;
macOS and BSD refuse a second wildcard bind.

#### Sharing a port by address

`BipTransport::set_share_port_by_address(true)` (the B/IP builders'
`.share_port_by_address(true)`, Python's `share_port_by_address=True` on
`BACnetServer`, `BACnetClient` and `BipEndpoint`) binds the interface address
itself instead (#1538). Several devices on one host, each on its own address,
can then share one port such as 47808, and each receives only the unicast
sent to its own address. `start()` fails unless the interface is an explicit
address and the port nonzero.

What changes in this mode:

- **Sends** leave from the interface address and the shared port.
- **Receipt order:** on Linux, macOS and the BSDs, broadcasts arrive on
  separate receive-only listeners, read fairly against the address socket in
  no fixed order. A broadcast and a unicast that arrive together may be
  handled in either order, so a unicast that depends on a broadcast sent just
  before it, such as a query after a Network-Number-Is, can be handled first.
  A listener that fails is closed with a warning, and unicast goes on.
- **Broadcast address:** it must be the interface's subnet broadcast,
  judged by the netmask the host reports, or 255.255.255.255; anything else
  would lose this subnet's broadcasts, so `start()` fails. A loopback
  interface may also name itself, as loopback tests do; no other interface
  may. Where the host reports no netmask, the bind decides.
- **Arrival interface:** a listener on 255.255.255.255 (Linux) or `0.0.0.0`
  (macOS and the BSDs) hears every interface, so it keeps only broadcasts
  that arrived on the transport's own, by the index `IP_PKTINFO` (Linux) or
  `IP_RECVIF` (macOS and the BSDs) reports. On a host with several networks,
  such as a router with a port on each, one network's Who-Is doesn't reach
  the transport on another. The index is looked up when the transport
  starts, so after its interface changes (a re-plugged adapter, a rebuilt
  VLAN or bridge, a VPN that reconnects) restart the transport; the first
  broadcast dropped for arriving elsewhere is logged as a warning naming
  both indexes.
- **Linux** delivers a broadcast only to sockets bound to the wildcard address
  or to the broadcast address itself. The listeners bind the configured
  broadcast address and 255.255.255.255 with `SO_REUSEADDR`, which every
  device on the subnet shares, and each gets a copy. No listener sees a
  unicast, and the address socket shares
  nothing: no other socket can bind the same address and port, nor
  `0.0.0.0` on that port, so a default-mode transport can't share a port with
  devices sharing it by address.
- **macOS and the BSDs** refuse to bind 255.255.255.255, so one listener binds
  `0.0.0.0:port` with `SO_REUSEADDR` and `SO_REUSEPORT`, which several such
  listeners need, and each gets a copy of a broadcast. A unicast to a local
  address no socket on the port is bound to can reach a listener, which drops
  it. Another socket can't bind the same address and port without
  `SO_REUSEPORT` on both, which the address socket doesn't set. A default-mode
  transport's wildcard socket lacks `SO_REUSEPORT`, so it can't share a port
  with these listeners either.
- **Windows** delivers a broadcast arriving on an interface to a socket bound
  to that interface's address, so one socket is enough. Windows'
  `SO_REUSEADDR` would let another socket bind the same address and take its
  unicast, so the socket sets `SO_EXCLUSIVEADDRUSE` instead, and no other
  socket can bind that address and port. Other addresses can still share the
  port, and a socket already bound to `0.0.0.0` on it without
  `SO_EXCLUSIVEADDRUSE` doesn't stop the bind (it does stop a default-mode
  start). Under Windows' strong host model a socket bound to one interface
  sends only through it, so a multihomed BBMD in this mode reaches only the
  peers that interface can.

Tests run the shared port on every OS: on Linux on 127.0.0.2 and 127.0.0.3,
with broadcasts to 127.255.255.255 and 255.255.255.255, and on macOS and
Windows with 127.0.0.1 beside the default-route address. A subnet broadcast
on the default-route interface checks broadcast receipt on every OS, and is
skipped without a broadcast-capable default route; Windows runs it only in
CI.

With the `0.0.0.0` interface, `start()` lists the host's IPv4 addresses, with
`getifaddrs` on Linux, macOS and the BSDs and `GetAdaptersAddresses` on
Windows, and the transport accepts a unicast datagram only when the
destination the OS reports for it is one of them. The list holds every address
configured on the host, on any interface, up or down, loopback and link-local
included. On Windows that is every address except those duplicate address
detection marked as duplicate (in use by another host) or invalid, so a
tentative address, such as a static address on a disconnected adapter, counts.
The list is read at each start, so an address added later is accepted after
the next restart. If the addresses cannot be listed, or none is usable,
`start()` fails and suggests binding an explicit interface address.

A datagram whose UDP source is a group address (the limited broadcast, a
multicast address, or the configured broadcast IP unless it is one of the
node's own addresses) is dropped before its BVLC function is handled, and
counted in `group_source_drops()` (#1504). No node sends from one, and the
stack would answer it there, register it as a foreign device, or forward
from it as a BBMD. Linux discards most such datagrams itself; other systems
may not.

The stack takes a Forwarded-NPDU's originating address as the NPDU's source,
so an origin that is one of the link's group destinations
(`is_group_destination`: the limited broadcast, the configured broadcast IP
at any port, or a multicast address) makes the frame malformed. The receive
loop drops it before the network layer sees it, a BBMD forwards it nowhere,
and `forwarded_group_origin_drops()` counts it (#1493). Otherwise a forged
I-Am could bind a device to a group, and the answer to a request would go to
every node in it.

### BIP6 (IPv6)

```rust
use bacnet_transport::bip6::Bip6Transport;

let transport = Bip6Transport::new(
    Ipv6Addr::UNSPECIFIED,  // select one unambiguous local link/address
    0xBAC0,                 // port
    None,                   // device_instance (auto VMAC)
);
// 3-byte VMAC, 3 multicast scopes, collision detection
```

The transport selects one concrete local address and OS interface for normal
B/IPv6 operation. `::` requires one usable non-loopback multicast interface (or
loopback if none exists), then a unique non-link-local address on that interface,
otherwise a unique link-local address. Multiple interfaces or addresses in the
selected class fail startup; configure an existing concrete address to resolve
ambiguity. A concrete address must have one usable local owner. This is local
selection policy, not an Annex U requirement. It replaced, in 0.12.0, 0.11.0's
selection, which asked the routing table for an address and fell back to `::1`.

The selected address and actual UDP port form `local_mac()`. One wildcard socket
receives selected unicast and BACnet multicast traffic; packet metadata fences
other interfaces and destinations before collision handling, VMAC learning or
NPDU admission. Every normal data/control send retains the selected source and
interface. Required joins and random-VMAC collision probing finish before
publishing startup state. Failed or cancelled startup, stop, restart and drop
reclaim the socket/task lifetime; port zero selects a fresh ephemeral port on
restart. Link-local addresses retain their OS zone internally but cannot reach
other links; `::1` is node-local. FF05/FF08 group scope alone does not prove
cross-link reachability.

With `register_as_foreign_device`, `::` instead derives a concrete unicast source
usable for the configured BBMD; an explicit source is retained. The production
socket is bound to that source, so registration, DBTN and ordinary unicast agree
with `local_mac()`. This branch requires the existing configured Device instance
and preserves trusted-BBMD handling without normal multicast prerequisites.

A Forwarded-NPDU whose original source address is an IPv6 multicast group, a
group destination at any port, is malformed for the same reason as on B/IP:
it is dropped before the network layer sees it and counted in
`forwarded_group_origin_drops()` (#1493).

The selected-link wire fixtures qualify Linux on isolated ULA bridges and macOS
on loopback for multicast intake and unicast/control replies. Windows code is
compile-checked; its runtime remains unqualified. Unique link-local selection has
unit evidence, not physical-link wire qualification. This is bounded transport
evidence, not full Annex U conformance. External fixture commands and their
intentional exclusion from normal test runs are documented in
[the qualification guide](../crates/bacnet-transport/tests/ipv6_selected_link/README.md).

### BACnet/SC (Client Transport)

```rust
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};

let tls_config = ScNodeTlsConfig::from_der(ca_certs, node_cert_chain, node_key)?;
let ws = TlsWebSocket::connect("wss://hub:1234", tls_config).await?;
let transport = ScTransport::new(ws, vmac)
    .with_device_uuid(device_uuid) // caller's already-provisioned, durable [u8; 16]
    .with_heartbeat_interval_ms(30_000)
    .with_heartbeat_timeout_ms(60_000);
```

Production BACnet/SC transports validate heartbeat settings at `start()`: the interval must be
`3_000..=300_000` ms, and the disconnect timeout must be greater than the interval.

**Raw transport startup migration:** `new(ws, vmac)` remains two-argument and
infallible, with a zero UUID placeholder while unstarted. `with_device_uuid` is
required before `start()`: omitted/all-zero UUIDs and reserved all-zero/all-ff
local VMACs return clear `Error::Encoding` configuration errors. Error precedence
is reconnect configuration, heartbeat timing, then identity. No UUID version or
variant bits, EUI-48 shape, or Random-48 shape are enforced by this guard.

Identity failures precede transport-owned sends, receives, connector invocations,
socket consumption, channel/task allocation, and startup state changes. Sockets
are retained on repeated failure; correct a UUID with the existing consuming
`with_device_uuid` setter and retry on the **same owned WebSocket**. There is no
new VMAC repair setter. This cannot undo caller-owned WebSocket creation, dials,
or external work used to construct connector closures, nor does it promise
generic endpoint rollback or repairability of every configuration field.

The caller owns predeployment UUID generation and durable same-byte lifetime
reuse. Internal reconnect/failover/primary restore preserve the UUID, including
when a duplicate-VMAC NAK legitimately reselects the VMAC. This is **startup
enforcement, not lifetime immutability**. Since #956 `ScTransport` no longer
exposes its `ScConnection`, so applications cannot change the identity through
it and read the link state through `connection_state_changes()`. Pure
`ScConnection` codec/manual WebSocket use and later handshake validation are
outside this guard.

`with_advertised_uris` configures known direct-connection URIs; it does not enable
accepting connections. Address-Resolution requests receive an ACK (with a
possibly empty URI list) only while a registered direct listener is live, its
VMAC/UUID matches, and both NPDU intakes remain open. Otherwise the node returns
COMMUNICATION/OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (`7/45`). This current-live
availability policy is shared with Advertisement. Capability NAKs use the
existing rejection deadline and retirement behavior.


### Direct peer membership and limits

`DirectListener::start(config)` creates a standalone direct listener.
`ScTransport::with_direct_listener(config)` registers its intake and shares one
UUID/VMAC owner with opt-in `with_direct_discovery` / `with_direct_tls`.
The returned listener handle may stop the listener independently; transport
stop/abort/drop also seals that registered listener and its response writers.
Retaining the handle permits explicit cleanup/join, not continued acceptance or
responses after transport teardown. A standalone listener retains its own lifetime.
Changing discovery does not erase accepted membership. Accepted and outbound
peers share identity uniqueness while retaining separate numeric quotas.

`DirectAcceptConfig::with_max_established_peers(M)` sets the established accepted
peer limit; its default is `DIRECT_ACCEPT_MAX_ESTABLISHED_PEERS` (16). Zero
normalizes to one. At most M handshakes may be pending and at most 2M accepted
physical sockets may exist, including retiring replacements. Values above
`usize::MAX / 2` fail before binding. `active_connections()` counts physical
sockets and can therefore reach 2M. Pending/physical saturation drops TCP;
otherwise a distinct valid Connect at accepted capacity receives the local
`RESOURCES/OTHER` NAK. Saturation does not guarantee reconnect admission.

**Pre-1.0 API break:** `with_max_connections` becomes
`with_max_established_peers`; the former physical-cap semantics and name are
removed, without aliases. The default constant now describes established
peers. Callers that use `active_connections()` must allow pending and retiring
sockets in addition to established peers.

A known UUID can replace its direct connection with the same or a free changed
VMAC. Successful Connect-Accept transmission precedes incumbent retirement;
failed or cancelled admission preserves the incumbent. A contender claiming a
third peer's VMAC receives `COMMUNICATION/NODE_DUPLICATE_VMAC`, preserving both
incumbents. This compound-conflict precedence and the capacity error pair are
local policy. Replacing an outbound peer still needs an accepted slot.
Outbound sends verify the peer's Connect VMAC against the requested destination.
The outbound pool retains at most 16 peers, with 16 pending dials and 32 physical
sockets per enabled discovery owner; expiry, eviction and disable retire only
their own generation. An idle worker observes remote EOF/Close and answers a valid Disconnect request
before closing. Malformed Disconnect requests use the existing control validator.
Disable/drop cancels owned outbound socket workers; asynchronous stop also joins them.

Simultaneous replacements can select opposite sockets at the two endpoints and
leave no live direct connection. Membership guarantees at most one current
connection per peer; normal Hub fallback and bounded URI backoff/retry apply.
A successful local WebSocket write does not confirm remote NPDU delivery.

Private process-wide generations fence new work from old sockets and stale
cleanup. Already queued complete NPDUs retain their original values, including
the [direct TLS identity snapshot](#accepted-direct-tls-identity). UUID claims
are not certificate bindings. The bounded
[server response policy](#accepted-direct-server-responses) is separate from
[ordinary bidirectional routing](#bidirectional-direct-traffic).

### Bidirectional direct traffic

Current native source selects an established matching accepted or outbound direct
connection before optional URI discovery, even when discovery is disabled.
Broadcasts continue through the Hub. Configure `with_direct_tls(ScNodeTlsConfig)`
before start for built-in TLS dial-out with application intake and matching
original-response authority. This works independently of the Hub adapter type.
A valid Connect and membership publication precede application use. Ordinary
direct frames omit both VMAC fields, retain Data Options, and obey the peer's
independent NPDU and complete BVLC limits. The existing bounded intake applies to
both direct roles; stale generations cannot admit new frames, while already
admitted immutable envelopes retain their original identity and reply capability.

**Pre-1.0 API break:** `with_direct_dialer` is replaced by
`with_custom_direct_dialer`, with no alias. Arbitrary factories remain application
send-only, including closures returning `TlsWebSocket`; they cannot attest a TLS
leaf or mint a direct identity/capability. Use the built-in `with_direct_tls` path
for authenticated bidirectional intake. Verified leaf capture also works on TLS
resumption using rustls's authenticated session identity; a resumed handshake need
not retransmit the certificate. This does not attest that a remote server requested
or verified the local certificate, or bind its certificate to its UUID/VMAC claims.

The current direct queue is shared by ordinary and original-response writes:
64 queued operations plus at most one active write. Saturation returns an error
without Hub fallback. Queue cancellation, owner shutdown and retirement are checked
before writing, and reads (including Ping/Pong) alternate with bounded writes.
Only definitely unstarted work on a retired route permits a fresh route decision;
an uncertain started write is never duplicated through the Hub or another dial.
The existing original-response capability always fails closed after retirement.
Disabling discovery retires outbound workers but keeps accepted membership; stop,
abort and drop seal the transport lifetime irreversibly.

Outgoing client and native confirmed-notification transactions retain BACnet's
canonical peer-address/Invoke-ID correlation, with existing service, direction
and segment-phase checks. Responses may switch Hub/direct paths. A replacement
peer claiming the same address can complete or control an old pending outgoing
transaction; this is not proof of same-leaf continuity. Optional historical-route
filtering is separate from the selected original-socket policy for incoming replies.
No outgoing transaction/retry policy changes here. This behavior is new in
0.12.0 and adds no Python direct-entry API, Hub-relayed end-to-end identity,
full Annex AB or certification claim.

### Accepted direct TLS identity

The stack carries `TransportProvenance::direct_sc_identity()` through
accepted-direct and built-in outbound TLS ingress, the network queue, and server dispatch. It returns a
sealed, immutable `DirectScIdentity` with read-only `leaf_sha256()` and
`incarnation()` accessors. The fingerprint hashes the exact verified TLS leaf
DER; it does not hash PEM text, a public key, or claimed UUID/VMAC/SNET/SADR.
Same-leaf reconnects have different incarnations, and certificate rotation
changes the fingerprint. Incarnations are process-lifetime identifiers, not
persisted identity or an ordering API. Both values are `Copy + Eq + Hash`;
their `Debug` output omits fingerprint and incarnation.

Both built-in direct TLS roles capture the verified leaf before WebSocket upgrade and admit
NPDUs under the committed membership generation's fence. A missing verified
chain fails closed. Already admitted complete work may finish after close or
replacement under its original snapshot; it is not revoked or reinterpreted
using the new VMAC owner. Generic confirmed duplicate admission and the local
LSO replay store partition by this leaf/incarnation in addition to their existing
request keys. Generic duplicate detection now retains only pending operations;
LSO keeps its separate completed replay policy. Same-socket pending detection,
non-direct canonical keys, and capacity bounds remain unchanged. See
[confirmed transaction lifetimes](#confirmed-transaction-lifetimes).
Receive reassembly and client Abort cancellation also isolate direct incarnations.
Delayed, already-admitted A segments can finish A's context after replacement;
new frames from retired A cannot enter it.

Mutation and LifeSafetyOperation authorization contexts expose
`direct_sc_identity()`. Every WPM element retains the request's one snapshot,
with existing per-element ordering and authorized-prefix behavior. Application
policy still decides permission. `MutationTrust` and `ControlTrust` remain
scope labels; claimed addresses remain claims. **Pre-1.0 API change:**
`LifeSafetyOperationAuthorizationContext` now includes `provenance`, and its
`Debug` output redacts addresses and request inputs. Hub admission callbacks
report `is_hub_channel()` rather than `is_direct_peer()`; that scope-only value,
Hub-relayed ingress, and unverified transports return no direct identity.

These APIs are new in 0.12.0. This does not provide a Python
principal callback, certificate-to-claim binding, Hub-relayed end-to-end identity,
or a Python direct connection entry point. The narrower server response
capability below is separate from authentication provenance.

### Accepted direct server responses

Current native `BACnetServer` replies to verified direct confirmed requests only
through their original TLS connection, for accepted and built-in outbound peers. This covers SimpleACK, ComplexACK,
Error, Reject, server/overload Abort, LSO replay, segmented responses and retries,
and segmented-request SegmentACK/Abort. Replacement, closure or missing/mismatched
capability fails closed: there is no current-VMAC, replacement-socket, Hub or
new-dial fallback. Complete admitted work may still execute under its original
authorization; failure to reply does not roll it back or prove remote receipt.
Unconfirmed Who-Is/Who-Has discovery replies retain ordinary routing and are
outside this confinement guarantee.

**Pre-1.0 API change:** `ReceivedNpdu` and `ReceivedApdu` add
`direct_response: Option<DirectResponse>`; custom constructors use `None` for
unverified ingress and forwarding consumers preserve the original value.
`TransportProvenance` and `DirectScIdentity` remain `Copy + Eq + Hash`.
The separately sealed, cloneable `DirectResponse` exposes its read-only identity
and has redacted `Debug`; clones retain neither socket nor membership.
`ReceivedApdu::response_route()` saves a `ResponseRoute`. Server response helpers
use `NetworkLayer::send_response_apdu_on_issuance`, retaining routed DNET/DADR
encoding while selecting only the original direct writer. Verified direct
provenance without matching capability is an error; non-direct requests retain
ordinary routing and MS/TP reply handoff. Generic pending ownership still ends
at local encoded operation issuance, not the eventual send result.

Each direct connection has one socket writer and a shared 64-item ordinary/reply
queue, plus at most one active write. Ordinary saturation can reject a reply.
Queue saturation fails immediately; queue wait and each write are bounded by
the configured Connect timeout. The writer checks original membership and the
network/server's irreversible `DirectResponseScope` before starting queued work.
Network stop/drop and server stop/drop seal that scope synchronously. Low-level
capability callers must retain a scope for their owner and seal it at shutdown;
a sealed scope never reopens. Cancelled queued work is skipped. Already-started
writes cannot be recalled; timeout or a failed/retired write closes the worker.
The peer's negotiated Max-NPDU-Length and complete Max-BVLC-Length are checked.
`DirectResponse::max_npdu_length()` exposes that immutable payload budget;
`ResponseRoute::max_apdu_length(cap, destination)` subtracts the same encoded
local/routed NPDU header used for issuance. Following Clause 5.2.1.2, server
response selection uses the minimum of this path budget, the requester's APDU
acceptance and the server's configured APDU cap. ComplexACK segments fill this
budget subject to the existing segment-count and capability limits. If no
segment fits, the existing Abort path applies; if even that cannot fit, the
bounded send fails without fallback. Sizing never requires current membership
and does not revoke admitted execution; invalid authority still fails at send.
Response and received WebSocket frames alternate preference: a ready binary,
Ping or Pong frame can precede a queued response by at most one read turn, and
a full response queue can precede input by at most one bounded write. Ignored
controls yield a scheduling turn without extending the binary-activity idle
deadline; handshake control filtering retains the absolute Connect timeout.

Segmented-response ACK/Abort admission includes the original direct leaf and
incarnation. A reconnect cannot advance or cancel an old response child.
Receive reassembly saves segment zero's response capability separately from
its authorization snapshot; final completion uses that saved route.

This is selected local confinement policy, not a Standard requirement to deliver
on a historical socket. It is new in 0.12.0 and qualifies only the
native server consumer described here. The [client and endpoint supplement](#accepted-direct-client-and-endpoint-replies)
qualifies those additional inbound reply consumers. Outgoing client transaction
correlation retains the [standard path-switching behavior](#bidirectional-direct-traffic). No full
Annex AB, external interoperability or certification claim follows.

### Accepted-direct client and endpoint replies

The selected original-socket response policy extends to standalone
`BACnetClient` handling inbound confirmed COV/Event notifications and unsupported
or segmented confirmed requests, and to `EndpointSession`'s existing narrow
ReadProperty/authorized Device WriteProperty responder. This is new in
0.12.0. It does not change outgoing client transactions, their retries or their
terminal/segment-control admission. Ordinary direct routing now applies as
described [above](#bidirectional-direct-traffic). BACnet permits response path switching; this is a
local confinement policy for these incoming-request consumers, not a universal
protocol correlation requirement.

Direct provenance **or any supplied direct capability** selects checked response
issuance before `reply_tx`. Missing, mismatched, retired or sealed authority fails
without a prompt-channel, address, replacement, Hub or new-dial fallback. Ordinary
non-direct/no-capability ingress keeps existing MS/TP behavior: the standalone
client falls back to ordinary routing after a failed prompt handoff, while the
endpoint completes that prompt attempt even if its receiver closed. Endpoint
reply suspension only consumes ordinary prompt work. Group requests and the
client's COV `NoResponse` policy remain silent.

The endpoint carries the saved `ResponseRoute` through its existing bounded
egress queue (`SessionConfig.queue_capacity`). The hidden composition method
`EndpointEgress::admit_response_apdu` returns caller-owned completion: dropping
it retracts queued work. Stop/drop closes admission and cancels queued work;
retained role handles cannot extend that lifetime. The direct socket still has
the shared 64-item ordinary/reply writer queue and its existing bounded I/O. An already-started
write cannot be recalled, and local completion does not prove peer receipt.

After service execution, the endpoint caps a ComplexACK by the requester APDU
limit and saved peer NPDU/BVLC limits using the actual local/routed NPDU header
(Clause 5.2.1.2). It has no segmented response sender: an oversized ACK selects
its existing `SEGMENTATION_NOT_SUPPORTED` Abort. If that Abort cannot fit, checked
send fails without fallback. Retirement or invalid reply authority does not
revoke the original authorization or roll back an admitted Device write.
Direct responses use empty outgoing Data Options; ordinary endpoint egress keeps
its existing data-attribute and destination behavior.

Real-TLS and lifecycle tests use public client/session paths, held A envelopes
and distinct-leaf B replacements, mixed prompt channels, exact response budgets
and cancellation barriers. This is neither a full Annex AB claim nor Python
direct-listener/API support.

### Hub certificate bindings

`ScHubCertificateBinding::new(uuid, allowed_vmacs, leaf_sha256)` creates one
immutable installation group. The UUID is a nonzero 16-byte value; VMACs are
distinct nonreserved six-byte values, and fingerprints are distinct 32-byte
SHA-256 digests of the **exact leaf certificate DER**. Both lists must be nonempty.
`ScHubCertificateBindings::new(Vec<ScHubCertificateBinding>)` rejects empty maps
and any UUID, VMAC or digest shared by groups. Multiple digests in one group
authorize explicit certificate rotation; multiple VMACs authorize only those ports.
All inputs are owned. Debug and errors omit certificate and policy contents.

Install the map with `ScHubTlsConfig::with_certificate_bindings(bindings)`.
An absent map retains CA-valid admission. A configured map admits only listed
verified leaves with the group's exact UUID and one allowed VMAC; even unreserved
claims from an unmapped leaf are denied. UUID/VMAC reservations exist while the
node is offline. Local Hub VMAC overlap fails before binding; there is no extra
restriction on a group's UUID matching the hosting device UUID.

The Hub hashes the verified leaf after TLS acceptance. Under the registry lock,
it checks the map before the existing admin callback, deadline commit, insertion
or incumbent replacement. Callback Allow cannot override a binding denial; Deny
or panic still refuses a matching leaf. Existing collision, capacity, deadline
and same-UUID replacement rules then apply, including to listed renewals.
Denied attempts leave incumbent membership/relay intact and increment the existing
redacted `admin_denied` counter. Cloned configurations share immutable policy,
with independent live registries and counters.

This is opt-in installation policy allowed by 135-2020 Annex AB.7.4, not its
default authentication requirement. Configure every Hub feeding a trusted router
ingress consistently. It does not convey a certificate principal in relayed BVLC
frames, authorize BACnet operations, bind direct-peer requests, or complete the
Annex AB security profile (#518/#524 remain separate; accepted-direct identity
is described [above](#accepted-direct-tls-identity)). Runtime tests use
distinct real same-CA leaves, native registration/relay and joined shutdown.

### BACnet/SC Hub

```rust
use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};

// Owned, already loaded DER: Vec<CertificateDer<'static>> for both lists,
// and PrivateKeyDer<'static> for hub_key. File loading belongs to the caller.
let tls = ScHubTlsConfig::from_der(ca_certs, hub_cert_chain, hub_key)?;
let mut hub = ScHub::start_with_tls_config(
    listen_addr, tls, hub_vmac, hub_uuid, ScHubHandshakeTimeouts::default(),
).await?;
let addr = hub.local_addr().expect("started hub has a bound address");
// ... use the hub ...
hub.stop().await;
```

The SC hub is a TLS WebSocket relay. Both clients and servers connect to it as spoke nodes. Messages are routed by VMAC address.

`ScHubTlsConfig::with_admission_policy` receives `ScHubAdmissionInput::registration`
as one fixed `ScHubRegistrationKind`: `Initial`, `SameUuidSameVmac`,
`SameUuidMovedVmac`, or `ConflictingVmac`. Classification and policy run under the
same registry lock before registration commit; Allow still applies ordinary
collision and capacity rules. The labels reveal no incumbent identity fields.
A known UUID moving onto another UUID's VMAC is classified as `ConflictingVmac`.
UUID equality is a payload claim, not certificate-principal authentication.
The default accepts/replaces a known UUID as Annex AB requires; an operator may
explicitly deny the two same-UUID kinds as local security policy before protocol
acceptance. Denial leaves the incumbent untouched and retains the existing
RESOURCES/OTHER NAK and admin-denial counter. Policies remain synchronous,
nonblocking and panic-deny; they must not perform I/O or reenter the registry.


Every admitted transit relay attempt has one configurable local budget (five
seconds by default), including destination sink acquisition and WebSocket send.
It applies to NPDU/opaque unicast, each concurrent broadcast recipient, and
forwarded BVLC-Result. A timeout
lets that source process its next frame; it does not retry, fabricate a Result,
or retire the destination solely for timing out. Terminal send errors retain the
existing captured-connection retirement rules, and heartbeat liveness is separate.
Cancellation cannot retract bytes already buffered by the WebSocket. Broadcast
fanout remains concurrent; a healthy recipient need not wait for a blocked one.
Probe, control, cleanup and graceful-shutdown policies remain separate; shutdown
may force cleanup before a blocked relay's send deadline.

Peer-initiated WebSocket Close is answered through the connection lease, including
upgraded peers that have not sent Connect and peers closing while the Hub awaits
Disconnect-Ack. Cleanup flushes the queued reciprocal frame within the local
five-second sink acquisition/I/O bound after retiring the matching registration.
Close without Disconnect-Ack still yields a forced graceful-shutdown outcome;
forceful abort may forgo the reply. Tests cover the WebSocket replies, not TLS
`close_notify` behavior.

`ScHubTlsConfig::with_relay_send_budget(Duration)` validates this separate
transit budget; `validate_relay_send_budget` supports preflight before
loading TLS files. This replaces the pre-1.0 unicast-only setting without an
alias; update callers to `with_relay_send_budget`, `relay_send_budget` and
`validate_relay_send_budget`. `ScHubProbePolicy::new(scan_interval, idle_age, ack_age,
send_budget)` configures the optional accepting-Hub probe through
`with_probe_policy`. Defaults are 30s/60s/5s/5s. Both policies require positive
whole milliseconds, at most `i64::MAX` milliseconds to reserve tick headroom,
and a future instant representable by the platform monotonic clock.

Each Hub owns one monotonic origin. A scan probes only after idle age strictly
exceeds its threshold; pending ACK age starts at reservation, before sink
acquisition, and retirement requires a later scan to observe strictly exceeded
ACK age. Serial send work or scheduling delays can postpone that scan; ACK age
is not a hard closure deadline. Missed ticks are skipped. Only a matching valid
ACK clears pending and refreshes activity. Wrong-ID or malformed ACKs do not.
The probe send budget includes acquisition and send. These are local Hub
policies, separate from the initiating node's normative 3–300s heartbeat range.

`ScHubBroadcastRatePolicy::new(sender_burst, sender_per_second, global_burst,
global_per_second)` checks the existing continuously refilled broadcast policy
before I/O. `with_broadcast_rate_policy` applies it unchanged; rates and bursts
must be in `1..=u64::MAX / 1_000_000_000`. Existing sender/global exhaustion
counters and silent-drop semantics remain.

`ScHub::status().await.outcomes` is a fixed `ScHubOutcomeCounts` snapshot:
committed UUID replacements, selected VMAC/capacity refusals, ordered accept
limit drops, TLS/WebSocket/Connect timeouts, eligible NPDU/opaque unicast
missing-target/length-limit/send-timeout/send-error outcomes, and actual
matching-generation heartbeat retirements. Each `u64` saturates, starts at zero
for each Hub, and never controls policy. Refusals count decisions, not delivered
NAKs; a later Connect timeout can also count. Snapshots are not transactional.
Malformed, pre-registration, stale-source, self/local, broadcast, and forwarded
Result paths are excluded from unicast counters. Retirement skips do not imply
send success. Existing admin/broadcast counters retain their meanings.
Rust status remains available after stop; a new Hub using the same address and
config owns fresh counters.

With `sc-tls`, every public hub startup requires `ScHubTlsConfig`:
explicit nonempty CA trust anchors, mandatory WebPKI client verification, and
TLS 1.3-only local policy. Its fallible `from_der` constructor performs no file or
network I/O. Empty CA/chain, malformed DER (including a bad entry in an otherwise
valid list), unusable keys, and certificate/key mismatch return `Error::Encoding`
before startup can bind. The hub chain is leaf first. Configuration is private;
clones share the same policy, with no mutable/raw accessor or unchecked conversion.
The constructor uses the built-in aws-lc provider, not a caller-installed provider.

**Rust source-breaking migration:** the second parameter of `start`,
`start_with_uuid`, and `start_with_uuid_and_timeouts` is now `ScHubTlsConfig`, not
`TlsAcceptor`. Construct it with `from_der`; raw hub injection, including custom
verifiers/providers, TLS-version selection and arbitrary configuration knobs, is
retired with no public unchecked escape.

**Hub identity source/runtime break:** `ScHub::start(bind, tls, vmac, uuid)` now
requires the fourth UUID argument. All four public starts reject an all-zero
16-byte UUID or reserved UNKNOWN (all-zero)/BROADCAST (all-ff) local VMAC with
`Error::Encoding` before `TcpListener::bind`, through one shared enforcement point.
The other three method names, argument order and return contracts are unchanged.
No UUID version/variant or general VMAC bit-shape policy is added. Provision the
hosting device UUID before deployment and durably reuse it for its lifetime
(AB.1.5.3). Connect-Accept carries that device UUID and the hosting port's VMAC
unchanged (AB.2.11, AB.6), not an identity generated per connection. Persistence,
generation and detecting changed stored values belong outside this identity
argument. Peer registration bindings are configured separately on `ScHubTlsConfig`. Default or explicitly validated handshake budgets and lifecycle
are preserved. `start_with_tls_config` remains a compatible full-control alias for
`start_with_uuid_and_timeouts`. Built-in node APIs separately require
`ScNodeTlsConfig`, as described below; generic custom transports remain available.
Python hub startup uses the typed path internally. The standalone benchmark
hub/device and Docker SC pair now require explicit mTLS PEM files; see
[Secure Docker migration](../examples/docker/README.md).

Local configuration checks do not certify certificate dates or issuer
relationships: peers verify certificates at handshake time using rustls trust
anchors. Base Standard 135-2020 AB.7.4/AB.7.4.1.1 provides the mutual operational
authentication and installation-credential context; TLS 1.3-*only* is local policy,
not the Standard's TLS 1.3-*support* requirement. This does not add direct-issuer,
revocation or SAN policy, or close the full security
profile gap (#513 remains open for final acceptance assessment and the remaining
policy limits, not for a public raw hub startup path).

Evidence includes executable/compile-fail rustdoc, native preflight rejection,
TLS 1.3 mutual authentication with Connect-Accept and relay barriers after missing,
wrong-issuer, expired, not-yet-valid client and TLS 1.2 denials, custom phase
deadlines, and explicit stop. All three formerly raw startup methods have live
positive/negative coverage and wrong-argument-type compile-fail examples; the old
TLS 1.2/server-auth-only characterization is intentionally retired. Installed Python tests separately exercise OpenSSL
peers and ReadProperty; these are not hardware or full-profile certification.

The already-mTLS benchmark hub launcher and the CLI ReadProperty and server SC-DCC
test fixtures also use the validated hub path with explicit test UUIDs, retaining timeouts,
authentication modes and cleanup. The benchmark PEM loader has focused empty,
malformed, mixed-valid/invalid DER and mismatched-key tests. Independent raw TLS
peer helpers (including TLS-version negative controls) retain their existing
signatures for independent peers, not public hub startup. The separate
standalone/Docker migration did not change production CLI or node APIs; the
subsequent built-in node API migration follows.
Benchmark compilation and functional TLS tests are not new performance qualification.
The server-auth-only `sc_latency`/`sc_throughput` targets are retired; the original
mTLS targets remain, with historical results and limits in [Benchmarks](../Benchmarks.md).

#### Strict local node TLS configuration

**Rust source-breaking migration:** `TlsWebSocket::connect(url, config)`,
`ScClientBuilder::tls_config(config)`, and `ScServerBuilder::tls_config(config)`
now require `bacnet_transport::sc_tls::ScNodeTlsConfig`, not
`Arc<rustls::ClientConfig>`. Names, argument order, return types, identity defaults,
and lifecycle behavior are unchanged. Load owned DER at your application boundary:

```rust
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
let tls = ScNodeTlsConfig::from_der(ca_certs, node_cert_chain, node_private_key)?;
let ws = TlsWebSocket::connect(hub_url, tls.clone()).await?;
// Or pass tls to BACnetClient::sc_builder().tls_config(tls), or
// BACnetServer::sc_builder().tls_config(tls), then finish that builder.
```

The factory accepts nonempty explicit CA and leaf-first operational chains plus a
matching usable key. It checks every DER entry before any I/O, uses the fixed
built-in aws-lc provider, normal WebPKI server CA/name verification, and TLS 1.3
only. It does not preflight local certificate dates, issuer relationships, EKU,
or authorization. No raw constructor, getter, mutable access, or unchecked
conversion exposes the underlying configuration. Clones share one configuration,
including its verifier, identity resolver and normal resumption cache; reconnects
do not rebuild it or disable tickets. Early-data policy is unchanged.

**Local contract limit:** the node offers credentials when requested and compatible.
A trusted TLS 1.3 server that sends no CertificateRequest can complete; resumed
connections may not retransmit certificates. This does not attest that every
connection presents an identity or that an arbitrary remote hub verifies it.
The independent server-side no-request and Full→Resumed tests characterize this
limit; strict hub and reconnect/failover tests cover the configured mTLS paths.
`WebSocketPort`, `ScTransport`, and generic builders remain public and generic;
other WebSocket implementations are outside this built-in driver guarantee.
Python constructors, CLI flags, file-I/O/error phases, and Docker provisioning stay
compatible; their existing node policy is consolidated internally. The local raw
configuration gap is closed, not the full Annex AB profile or issue #513.

### MS/TP (Serial RS-485)

MS/TP is a token-passing protocol over RS-485 serial, commonly used for field-level BACnet devices. The serial I/O is abstracted behind the `SerialPort` trait, with three RS-485 direction control modes.

#### Host Diagnostics and Qualification

Call `MstpTransport::diagnostics()` before moving the transport into its owner.
The cloneable `mstp::MstpDiagnostics` handle returns owned
`MstpDiagnosticsSnapshot` counts during operation and after stop/drop, without
retaining serial ownership. Counts start at zero, saturate at `u64::MAX`, and have
no reset. Relaxed loads are individually atomic, not a globally coherent snapshot.
These are redacted host events, not wire timestamps or proof of peer delivery.
See the [qualification method](mstp-qualification.md) and
[unrun result template](mstp-qualification-result.json) for before/after deltas,
the #707 rerun matrix, independent capture requirements and explicit non-claims.
No hardware qualification, timer change, Python/generic status API, or expanded
MS/TP routing/conformance claim is included.

#### Optional Dedicated Execution

Execution placement is configured separately from `MstpConfig`, preserving
existing master-node configuration literals and `SerialPort` implementations:

```rust
use bacnet_transport::mstp::{MstpConfig, MstpExecutionMode, MstpTransport};

let transport = MstpTransport::new(serial, MstpConfig {
    this_station: 1,
    baud_rate: 76800,
    ..MstpConfig::default()
})
.with_execution_mode(MstpExecutionMode::DedicatedThread);
```

Call the builder before `start()`. Omitting it (or selecting `Tokio`) keeps the
existing Tokio-spawn path; changing the builder setting does not migrate an
already-running loop. Dedicated mode runs the same MAC loop on an OS thread with
its own current-thread runtime. Read/write polling and blocking backend work are
isolated from application workers; native drain offloads use the isolated
runtime's blocking pool. This applies equally to UART and USB serial backends,
without changing frame order, drain boundaries, turnaround deadlines or the
64-entry NPDU receive channel. Async serial resources opened on another reactor
still require that original reactor to remain running.

`start()` reports thread/runtime creation failures without falling back to the
application pool. `stop()` waits for task and isolated-runtime teardown, clears
the transmit queue and sets the node to Idle. `abort()` and drop request
cancellation and release transport-owned state without waiting. Blocking calls
must return before their resources can be released; cancellation is not a drain
or rollback of driver-accepted bytes. As before, stopping does not make a
consumed serial transport restartable: construct a new transport to restart.

Isolation is an execution option, not a real-time guarantee or measured timing
claim: fast/efficient does not mean deterministic. RT policy/priority
(`SCHED_FIFO`), CPU affinity/pinning and reporting RT setup success/failure are
**deferred, not implemented**. PREEMPT_RT, IRQ, mlock and buffer-tuning deployment
guidance beyond this note, plus on-wire hardware qualification (#502), remain
out of scope. This is only the thread-isolation subset of #501; that issue remains
open for the residual RT work and full documentation.

#### Auto-Direction (USB RS-485 Adapters)

Most USB RS-485 adapters (FTDI, CH340, CP2102) handle direction switching in hardware — no configuration needed.

```rust
use bacnet_transport::mstp_serial::{TokioSerialPort, SerialConfig};

let serial = TokioSerialPort::open(&SerialConfig {
    port_name: "/dev/ttyUSB0".into(),   // Linux
    // port_name: "/dev/cu.usbserial-xxx".into(),  // macOS
    baud_rate: 76800,
})?;

// Use with BACnetClient or BACnetServer via generic_builder
let client = BACnetClient::generic_builder()
    .transport(MstpTransport::new(serial, 1, 127))  // station 1, max_master 127
    .build()
    .await?;
```

To find the port name, `available_ports()` returns the names of the serial ports
the operating system reports: macOS lists them through IOKit, Windows through
SetupAPI and the registry, and Linux from sysfs (`/sys/class/tty`). A port that
another program has open is listed too, and an empty list means none. It returns
`Error::Transport` with the `std::io::ErrorKind` of the failure if the operating
system can't be asked.

```rust
use bacnet_transport::mstp_serial::available_ports;

for name in available_ports()? {
    println!("{name}"); // /dev/ttyUSB0, /dev/cu.usbserial-1410, COM3, ...
}
```

#### Kernel RS-485 Mode (Linux, RTS-based)

When DE/RE is wired to the UART's RTS pin, the Linux kernel can toggle it automatically via the `TIOCSRS485` ioctl. Zero userspace overhead.

```rust
let serial = TokioSerialPort::open(&config)?;
serial.enable_kernel_rs485(
    false,  // invert_rts: false = RTS HIGH during TX
    0,      // delay_before_send_us
    0,      // delay_after_send_us
)?;
```

Both delay arguments remain in microseconds but must be exact multiples of 1000:
zero is valid, and `1000` requests one millisecond. The Linux ABI stores whole
milliseconds, so fractional-millisecond requests return an error before any ioctl
instead of being rounded.

After applying the configuration, the method reads it back with `TIOCGRS485` and
checks that RS-485 is enabled with the requested RTS polarity and delays. Drivers
may reject or sanitize unsupported settings; set/readback failures and mismatches
return an error. A mismatch reports the effective flags and millisecond delays.
A readback or verification error can occur after the hardware configuration has
changed; the method does not roll back or retry. Success logs the verified effective
settings. See the [Linux RS-485 userspace ABI](https://cdn.kernel.org/doc/html/latest/driver-api/serial/serial-rs485.html).

#### GPIO Direction Control (RS-485 Hats)

For RS-485 hats where DE/RE is wired to a GPIO pin (e.g., Seeed Studio RS-485 Shield on Raspberry Pi with GPIO18), use `GpioDirectionPort` to wrap a `SerialPort` that implements transmit-complete `drain()`. Requires the `serial-gpio` feature.

```rust
use bacnet_transport::mstp_serial::{GpioDirectionPort, TokioSerialPort, SerialConfig};

let serial = TokioSerialPort::open(&SerialConfig {
    port_name: "/dev/ttyS0".into(),
    baud_rate: 76800,
})?;

// Wrap with GPIO direction control: gpiochip0, line 18, active-high
let port = GpioDirectionPort::new(serial, "/dev/gpiochip0", 18, true)?;

// Or with an additional guard interval after drain (microseconds):
let port = GpioDirectionPort::with_post_tx_delay(
    serial, "/dev/gpiochip0", 18, true, 200,
)?;
```

The `GpioDirectionPort` wrapper:
- Sets GPIO to receive mode (DE deasserted) on creation
- Switches to TX mode before each `write()`
- Waits for transmit-complete drain, including after a partial write error
- Starts the optional transceiver guard interval only after drain succeeds; the delay is not a substitute for drain and must fit the link's driver-release timing budget
- Switches back to RX only after completion and the guard interval
- Serializes I/O with direction changes; after a failed drain or cancelled write, the next read/write must finish draining before restoring RX
- Uses the Linux GPIO character device (`/dev/gpiochipN`) via `gpiocdev` — not deprecated sysfs

A drain error leaves transmit completion unknown and DE asserted. If recovery is
not possible, the caller must handle the failed port; dropping the wrapper is not
an asynchronous drain. Unix `TokioSerialPort` uses the native serial backend's
`tcdrain`-backed synchronous flush on a blocking worker, keeping the stream alive
and exclusive until the syscall returns. This does not qualify adapter or driver
on-wire timing. Auto-direction and kernel RS-485 writes remain unchanged.

#### SerialPort Trait

The MS/TP state machine is hardware-agnostic. Custom serial implementations (e.g., for testing) can implement:

```rust
pub trait SerialPort: Send + Sync + 'static {
    fn write(&self, data: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
    fn drain(&self) -> impl Future<Output = Result<(), Error>> + Send;
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = Result<usize, Error>> + Send;
}
```

`write()` may report driver acceptance before transmission finishes. `drain()`
reports completion of all accepted output, including the UART shift register.
Its default implementation returns an unsupported-operation error, preserving
existing custom backends without falsely claiming completion. Custom backends
used for software direction control must implement this operation. The loopback
backend completes writes in memory and drains immediately.

### Loopback Transport

```rust
use bacnet_transport::loopback::LoopbackTransport;

let (side_a, side_b) = LoopbackTransport::pair(
    vec![0x00, 0x01],  // MAC for side A
    vec![0x00, 0x02],  // MAC for side B
);
```

In-process channel-based transport for composing a client and server without real network sockets (e.g. inside an HTTP gateway). `LoopbackTransport::pair()` creates two connected transports backed by `tokio::sync::mpsc` channels — sending on one delivers to the other. Available as `AnyTransport::Loopback` for use with the enum dispatch wrapper.

The peer receives every frame, whatever MAC a unicast was sent to. For tests that need to know, `record_unicast_destinations()` returns a receiver of those MACs, in the order the peer receives the unicast frames (#1243). Call it before handing the transport to a router or network layer; dropping the receiver stops the record.

By default the transport drops the data attributes given to `send_unicast_with_data_attributes` and `send_broadcast_with_data_attributes`, like a data link that cannot carry them. `carry_data_attributes()` makes that side hand them to the peer in `ReceivedNpdu::data_attributes`, so a test can feed a router or network layer frames that carry attributes and check the ones it sends back (#1289).

### AnyTransport (enum dispatch)

```rust
use bacnet_transport::any::AnyTransport;
use bacnet_transport::mstp::NoSerial; // placeholder when serial feature is off

let transport: AnyTransport<NoSerial> = AnyTransport::Bip(Box::new(bip_transport));
```

Variants: `Bip` (boxed), `Bip6`, `Mstp`, `Sc` (boxed), `Loopback`.

`bacnet_transport::bip::AsBip` lends the `BipTransport` underneath a transport
that may carry BACnet/IP. `BipTransport` always lends itself; `AnyTransport`
lends its `Bip` variant and returns `Error::UnsupportedTransport` naming the
data link for every other variant. The client's BBMD helpers take any transport
with `AsBip`, and a wrapper transport can implement it by delegating.

### BBMD

```rust
use std::net::Ipv4Addr;
use std::path::PathBuf;

use bacnet_transport::bbmd::BdtEntry;
use bacnet_transport::bip::{BipTransport, DEFAULT_BACNET_PORT};

// Bind the BBMD's own interface address and its subnet's broadcast address.
let mut transport = BipTransport::new(
    Ipv4Addr::new(192, 168, 1, 10),
    DEFAULT_BACNET_PORT,
    Ipv4Addr::new(192, 168, 1, 255),
);

transport.enable_bbmd(vec![
    // This BBMD's own row (added automatically when it is missing).
    BdtEntry {
        ip: [192, 168, 1, 10],
        port: DEFAULT_BACNET_PORT,
        broadcast_mask: [255, 255, 255, 255],
    },
    // A peer BBMD on another subnet, reached by unicast.
    BdtEntry {
        ip: [10, 0, 5, 2],
        port: DEFAULT_BACNET_PORT,
        broadcast_mask: [255, 255, 255, 255],
    },
]);

// Optional: load a BDT saved in this file at startup, falling back to the
// configured table if the file is missing or invalid. Write-BDT from the
// network is always refused and never changes the table or the file.
transport.set_bdt_persist_path(PathBuf::from("/var/lib/rusty-bacnet/bdt.bin"));

// Optional: sources allowed to send Delete-Foreign-Device-Table-Entry.
// An empty ACL denies every source.
transport.set_bbmd_management_acl(vec![[192, 168, 1, 100]]);
```

The BBMD's own B/IP address is the originating address of the broadcasts it
forwards for itself, the BDT row it never forwards to, and the source it drops
as the echo of its own broadcasts. With an interface address, it is that
address and the bound port. Bound to `0.0.0.0`, the BBMD reads it from the BDT
it starts with (the persisted BDT when that loads, otherwise the configured
one):

- the one row whose IP is a local IPv4 address of the host and whose port is
  the bound port;
- if several rows qualify, `start()` fails and asks for an explicit interface;
- if none does, the host's local address toward its default route, but only
  when it is one of the host's addresses and not loopback; otherwise `start()`
  fails and asks for an explicit interface or the BBMD's own row in the BDT.

A persisted BDT that loads is authoritative here: if no own address can be
chosen from it, `start()` fails rather than falling back to the configured
BDT. Only a self row that would push the persisted BDT past 128 entries still
falls back to the configured BDT, with a warning.

Each `start()` of a `0.0.0.0` BBMD repeats this, so a restart follows a
changed address. The self row the BBMD appended moves with it, and rows listed
in the BDT stay. A failed `start()` keeps the BBMD configuration, and a failed
restart leaves its BDT and FDT as they were. The host's IPv4 addresses are
the list described under [BIP (IPv4)](#bip-ipv4), the same on every platform,
so these rules are too.

On a multihomed host, prefer an explicit interface and that subnet's broadcast
address, so the echo of each broadcast comes back from the BBMD's own address.
A `0.0.0.0` BBMD whose broadcast address is 255.255.255.255 and whose own
address is not the default-route address logs a warning at start: the kernel
may send its broadcasts from another interface, and their echo is then not
recognised as its own. Whatever its own address, a BBMD never rebroadcasts on
its subnet a Forwarded-NPDU that arrived by broadcast (to the configured
broadcast address or 255.255.255.255), since the subnet already received it;
it still sends it to its foreign devices. This also stops a BDT row that is
the BBMD itself under another address from looping a broadcast.

A BBMD forwards its own broadcasts as well as those of other devices on its
subnet (Annex J.4.5). Each `send_broadcast` in BBMD mode queues a
Forwarded-NPDU, with the BBMD's own B/IP address as the originating address,
for every BDT entry except its own and for its registered foreign devices, then
sends the local Original-Broadcast-NPDU. At most
`ForeignDevicePolicy::max_fdt_fanout` foreign devices (default 32) and
`FanoutPolicy::max_fanout_per_input` targets in all (default 64) receive each
broadcast. So remote devices and foreign devices hear the BBMD's own Who-Is,
I-Am and Network-Number-Is, and the broadcasts it routes.

This fanout uses the same `FanoutPolicy` queue and rate limits as forwarded
input and shows in `fanout_counters()`. The per-origin limit is keyed on the
origin's IP only. The BBMD's own broadcasts, including those it routes, share
that one budget with everything else it forwards with its own IP as origin:
`max_packets_per_sec_per_origin`, 128 forwarded packets per second by default,
or about 128/T complete broadcasts per second with T targets. Past that, a
broadcast reaches only some of its targets: BDT targets come first, so foreign
devices are cut first. The global packet and byte limits also apply.

Throttled targets, a full queue and failed sends are counted in
`fanout_counters()` and logged; a stopped fanout worker or an encoding failure
is only logged. None of them fail the local broadcast. The forward is queued
before the local send and does not depend on it, so in BBMD mode an `Err` from
`send_broadcast` can follow a forward that was already queued.

---

## bacnet-network

Network layer routing, router tables, and the multi-port router.

```rust
use bacnet_network::layer::NetworkLayer;
use bacnet_network::router::BACnetRouter;
```

`BACnetRouter::start` takes the ports and a `RouterOptions` (#1220).
`RouterOptions::new()` is the plain router: a raw `mpsc::Receiver` for local
APDUs, the permissive wire-control policy and no network-control receiver.
The builder methods turn options on, in any combination: `track_admission()`
makes the local receiver an `AdmissionReceiver` with queue snapshots and a
per-source quota, `control_policy()` and `control_authorizer()` set the RB-09
gate for routing controls, and `network_control_receiver()` adds the receiver
described below (`network_control_receiver_with_admission()` for a tracked
one). The start returns a `StartedRouter` holding the router, the
local APDU receiver and, when asked for, the network-control receiver.

```rust
use bacnet_network::router::{BACnetRouter, RouterOptions, StartedRouter};

let StartedRouter { router, apdus, network_control } =
    BACnetRouter::start(ports, RouterOptions::new().track_admission()).await?;
```

An inbound NPDU whose DLEN or SLEN is past `NpduAddress::MAX_MAC_LEN` is
refused before anything else happens to it (#1141). `NetworkLayer` discards it
and counts it in `address_length_drops()`; a non-router has no reject message
to send. `BACnetRouter` neither forwards nor delivers it, counts it in its own
`address_length_drops()`, and, when the NPDU names a specific DNET, rejects it
with Reject-Message-To-Network reason 6 (`ADDRESSING_ERROR`, Clause 6.4.4) for
that DNET, as it does a DNET it cannot reach. A global broadcast or an NPDU
without a DNET is dropped without a reject.

Both also drop a frame whose link-layer source MAC, as the transport reports
it, is longer than `NpduAddress::MAX_MAC_LEN`, before decoding it, and count it
in the same `address_length_drops()` (#1198). Nothing answers such a frame, and
a router learns no route from it. No built-in transport reports a MAC that long
(B/IP, BACnet/SC and Ethernet use 6 octets, MS/TP 1, B/IPv6 18), so only a
custom `TransportPort` can, and every address the stack learns off the network
fits a `BACnetAddress`.

Both also drop an NPDU whose DNET is 0xFFFF and that carries a DADR (#1379).
DNET 0xFFFF already names every device on every network (Clauses 6.2.2 and
6.3.2), so a DADR beside it contradicts it. `NetworkLayer` hands such an NPDU
to neither receiver. `BACnetRouter` neither forwards nor delivers it, acts on
no network message in it and sends no reject, since a global broadcast never
draws one. Each counts it in `global_broadcast_dadr_drops()`, apart from
`address_length_drops()`: the lengths are fine, and the count points at the
peer that sent it. A global broadcast with DLEN 0 is unaffected.

Both also drop an NPDU addressed to a broadcast, global (DNET 0xFFFF) or
remote (a DNET with DLEN 0), whose APDU isn't an Unconfirmed-Request (#1491).
Only that PDU type may use a broadcast network address (Clause 6.3); a
Confirmed-Request, an ACK, an Error, a Reject or an Abort belongs to one peer's
transaction, so every device reached would get one that names no one.
`BACnetRouter` passes such an NPDU on to no network, delivers it nowhere and
sends no reject; `NetworkLayer` hands it to no receiver. Each counts it in
`broadcast_pdu_type_drops()`. The PDU type is the high nibble of the first
APDU octet, so nothing is decoded, and an empty APDU counts too. Network
messages, an Unconfirmed-Request, and an APDU to one device (a DADR, or no
DNET at all) are not affected; the server judges a confirmed request that
arrives by link broadcast itself.

`BACnetRouter` sends each Reject-Message-To-Network it originates to whoever
first sent the refused NPDU (Clause 6.4.4, #1158). An NPDU that arrived
with SNET/SADR came through another router: the reject carries that SNET/SADR
as its DNET/DADR, with a hop count of 255, and goes back out the arrival port
to the router that relayed the NPDU. An NPDU without SNET/SADR draws a local
unicast to its sender. An SNET equal to one of the router's own networks puts
the originator on a directly connected link, so the reject leaves by the port
attached to that network as a local unicast to the SADR, with no DNET (Clause
6.5.4). That is the arrival port when SNET is the arrival network (#1174), and
another port when the NPDU looped back to the router through some other path
(#1219). An SNET/SADR that is the router's own address on that network means
the NPDU came back to the router that sent it, and no reject goes out
(#1219). A reason 6 reject for an over-long SADR has no originator to name, so
it falls back to that local unicast. A received reject is relayed by its
DNET/DADR like any routed NPDU (Clause 6.6.3.5).

A received reject with no DNET, or whose DADR is the router's own MAC on the
port attached to its DNET, is addressed to the router itself (#1175). It
updates the routing table and goes no further. Start the router with
`RouterOptions::network_control_receiver()` to also get these rejects as
`ReceivedNetworkControl` records, the same type a non-router
`NetworkLayer` control receiver yields. That receiver is a raw
`mpsc::Receiver`. `RouterOptions::network_control_receiver_with_admission()`
returns the same stream as an `AdmissionReceiver` instead, as
`NetworkLayer::enable_network_control_receiver_with_admission()` does
(#1242): its `counters()` give the queue's depth, high-water mark and full and
closed drop totals. The two options are alternatives, and the later call
decides the receiver type.

```rust
let StartedRouter { network_control, .. } = BACnetRouter::start(
    ports,
    RouterOptions::new().network_control_receiver_with_admission(),
)
.await?;
let controls = network_control.expect("asked for");
let snapshot = controls.counters().snapshot();
```

A client or server attached to a router through a `LoopbackTransport` port
does not need either receiver: it is an ordinary node on that port's network,
and rejects for its requests reach its own `NetworkLayer`.

`NetworkLayer` is a non-router and frames each NPDU with the destination its
caller names: `send_apdu_routed` and `broadcast_to_network` put that DNET in
the NPDU even when it is the number `local_network_number()` holds. The
choice between local and routed traffic belongs to the caller, because a
confirmed request's transaction is keyed to the peer it was sent to, and an
answer goes back by the route its request arrived on. The full server and the
standalone client make that choice on every path they start, and send a
destination naming their own network's number as local traffic (#1358).
`BACnetRouter` keeps its own per-port networks and is not affected.

The routed sends (`send_apdu_routed`, `send_apdu_routed_via_local_broadcast`
and their `_with_data_attributes` forms), `send_apdu_on_issuance` and
`broadcast_to_network` refuse DNET 0 and DNET 0xFFFF with `Error::Encoding`
before anything is sent (#1314, #1340, #1380). A global broadcast goes out only
through `broadcast_global_apdu`, with DLEN 0 and the broadcast MAC, so that
every router on the network can pass it on (Clause 6.3.2); a unicast would
reach a single router.

A broadcast carries only an Unconfirmed-Request APDU (Clause 6.3). Any other
PDU type fails with an `Error::Encoding` naming it, before anything is sent,
from `broadcast_apdu`, `broadcast_global_apdu` and `broadcast_to_network`, and
from a routed or on-issuance send with an empty DADR (a remote broadcast), in
every `_with_data_attributes` form too (#1479).
`send_apdu_routed_via_local_broadcast` names one device, so it refuses an
empty DADR for every PDU type: a remote network's broadcast goes through
`broadcast_to_network`. A send naming one device takes any PDU type, even when
its link DA is the broadcast MAC. `send_apdu` to the MAC the transport reports
as its broadcast, or any other group address the medium carries
(`TransportPort::is_group_destination`: on B/IP the limited broadcast, the
configured broadcast IP or a multicast address at any port, on B/IPv6 any
multicast group, on Ethernet any MAC with the group bit set), is a local
broadcast too, but the layer doesn't ask the transport on every unicast:
`BACnetClient`'s confirmed requests, the endpoint's egress and its requester,
which take caller-chosen MACs, refuse anything but an Unconfirmed-Request there
themselves. The server refuses one for the confirmed requests it starts
itself, to event recipients, Channel and Command targets, audit recipients and
bound devices, and binds no device to one (#1493); its replies and COV
notifications go to the source a request came from. `is_broadcast_mac` keeps
its narrower meaning, this link's own broadcast, which routing relies on.

A reply goes back to the link-layer MAC its request came from, so a
confirmed request from a group address would get its answer, any segment
ACK, and the confirmed COV notifications of a subscription it makes, sent to
every node in the group. The server, the client and the endpoint ignore such
a request (#1504). The server counts it in
`BACnetServer::group_source_request_drops()` and the client in
`BACnetClient::group_source_request_drops()`; the endpoint's ingress hands it
to policy as `PolicyReason::GroupSource`, which the session counts with its
other policy outcomes. No built-in transport hands up a group source: B/IP
and Ethernet drop one themselves, MS/TP refuses a broadcast source station,
and IPv6 stacks discard a datagram from a multicast address. A nonzero count
points at a custom transport.

`BACnetRouter` also drops a routed NPDU that it would deliver, on a directly
connected port, to a DADR that is a group destination there
(`TransportPort::group_destinations`), unless its APDU is an
Unconfirmed-Request (#1504). As one unicast it would reach every node in the
group without the broadcast network addresses #1491 filters. It is delivered
nowhere, draws no reject, and counts in `group_dadr_drops()`. Network
messages, and a group DADR on a network behind another router, which that
router judges, are not affected. A routed request's SADR names a node on
another network, which the ingress rule above can't judge, so a reply to a
group SADR is dropped only where the final router is a `BACnetRouter`.

---

## bacnet-objects

BACnet object model: trait, database, and object implementations.

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

The pre-1.0 object API removes `WritePropertyRollback` and the
`capture_write_property_rollback` / `restore_write_property_rollback` hooks.
Implementors validate writes and preserve their own state on failure. The bundled
WritePropertyMultiple service retains every successful prefix write, reports the
first failure, and leaves the remaining suffix unprocessed; it does not restore
previous values. Built-in persistence and File resize candidates retain their
existing commit boundaries. No replacement token API is needed.

For unindexed writes that reach a built-in object's final property dispatch,
absent properties return `PROPERTY/UNKNOWN_PROPERTY`, including NULL writes.
Present read-only properties return `PROPERTY/WRITE_ACCESS_DENIED`. Presence is
based on that instance's effective metadata, including optional properties and
`PROPERTY_LIST`. Unprovisioned Staging names and stream File `RECORD_COUNT`
also report absence; present read-only record File counts remain denied. Earlier
state, command-source, authorization and indexed-write
guards retain their precedence; custom object implementations retain their own
write dispatch. WPM reports the failed coordinate and retains its successful
prefix.

At the indexed WP/WPM service gate, nonempty effective metadata that omits the
property produces `PROPERTY/UNKNOWN_PROPERTY` before value decoding. A served
scalar or BACnetLIST still produces `PROPERTY/PROPERTY_IS_NOT_AN_ARRAY`; a served
array retains its object-owned element, count and write-access rules. This
absence-first ordering is a local error-precedence policy. Empty custom metadata
does not prove absence: those objects keep their array classifier and writer
delegation. For this early absence failure, WPM keeps its successful prefix and
reports the exact failed object/property/index, without authorizing or observing
the failing element or suffix. The rejected indexed attempt produces no execution
Audit record; present read-only arrays and unindexed absence retain their existing
Audit handling. The outer WP authorization check and direct object writes are
unchanged.

A NULL written to a property that isn't commandable and has no NULL in its
datatype succeeds and leaves the property as it is (Clauses 15.9.2 and 15.10.2,
#1396). The server applies this once, for WriteProperty, WritePropertyMultiple,
`write_local`, `write_local_encoded`, a Command's local writes, CreateObject's
initial values and a Schedule's writes to its targets (#1416): the write goes
to the object as usual, and when the object refuses the NULL with
`PROPERTY/INVALID_DATA_TYPE` the server answers success instead. Every check the
object makes first still answers, so an unknown property, a read-only one, one
not writable in the object's state, or an array index out of range is refused as
before. An object's own Present_Value relinquish, and a property that stores a
NULL, never reach the rule. The rule covers an array element too, judged against
the element's datatype once the index checks out. Nothing follows such a write as
a change (no COV report, event pass or save). Over WriteProperty and
WritePropertyMultiple an Audit Reporter records it as a successful write; a
CreateObject is audited once, as its CREATE, and a Schedule's target writes
aren't audited. A CreateObject goes on to its next initial value, and a
Schedule counts the target as one that took its write. A custom object should
therefore check access and state before the value's datatype, as the built-in
objects do.

WriteProperty and WritePropertyMultiple give a property that
`BACnetObject::is_list_property` reports as a BACnetLIST, written whole, to the
object as a `PropertyValue::List` of any length: a value with no octets is the
empty list (Clause 20.2.17), so Alarm_Values can be cleared, and one element is
a list of one. The object judges the list, so an empty value on a read-only list
is its `PROPERTY/WRITE_ACCESS_DENIED`, and on a list it doesn't serve
`PROPERTY/UNKNOWN_PROPERTY`. A few properties the server decodes for the object
arrive as their raw octets in `PropertyValue::ApplicationData` instead, lists
among them: Recipient_List, Subscribed_Recipients,
List_Of_Object_Property_References, and the Loop and Pulse Converter references.
The object decides what an empty value means there (an empty
Setpoint_Reference holds no reference). For any other property, no octets is
`PROPERTY/INVALID_DATA_ENCODING` and one element arrives alone.
`write_local_encoded`, a Command's writes and CreateObject's initial values
decode the same way (#1328, #1389). CreateObject checks an initial value's array
index against the new object first, as WriteProperty does, and names the first
initial value it can't apply by its position; one that doesn't decode is
`PROPERTY/INVALID_DATA_ENCODING` there.

A new object's Object_Name, until an initial value renames it, is its type
and instance (`BINARY_VALUE-2`). When another object already holds that
name, the server takes the first free `BINARY_VALUE-2 (n)` from n = 2, so a
client that renamed an object to the next default name doesn't make the
create fail (#1437). A few read-only properties take a CreateObject initial
value: `BACnetObject::creation_only_properties` lists the ones an object sets
whole only at creation, and the server gives such a value, sent without an
array index, to `BACnetObject::initialize_property` instead of the write
route (#1429). The built-in Analog Input and Analog Output take Units (an
Enumerated up to 65535). The Multi-state Input, Output and Value take
Number_Of_States (1 to `multistate::MAX_NUMBER_OF_STATES`, 1024),
which resizes State_Text. A count is refused with `PROPERTY/VALUE_OUT_OF_RANGE`
if a value the object holds would name a state past it. WriteProperty still
answers `PROPERTY/WRITE_ACCESS_DENIED` for each. The PICS lists each
createable type's set.

State_Text written whole, by WriteProperty, WritePropertyMultiple or a
CreateObject initial value, sets Number_Of_States to its number of labels
(#1443), with the same checks: 1 to 1024 labels, and a shrink that would
leave Present_Value, Relinquish_Default, a Priority_Array command or an
Alarm_Values entry past the new count is `PROPERTY/VALUE_OUT_OF_RANGE` and
changes nothing. A Multi-state Output's Feedback_Value doesn't block a
shrink; past the count it shows as CONFIGURATION_ERROR. Since a whole
write can resize State_Text, its size at index 0 takes a write as well
(Clause 12.1.5.1): an Unsigned count with the same checks, which truncates
State_Text on a shrink and, on a grow, appends the `State {n}` labels a new
object starts with. A WriteProperty naming Number_Of_States is still
refused. The metadata gives Number_Of_States
`PropertyWriteCapability::Through(STATE_TEXT)`, which doesn't count as
writable, and the PICS keeps its row read-only, marks it "resized through
STATE_TEXT: a whole write or its size at index 0", and leaves it off the
creation-only line.

On these objects the order of the initial values follows one rule: the
values that give the state count are applied before every other initial
value, and everything else, including a count value that fails its own
checks (an array index, its datatype, its range), is applied in request
order. Those values are the request's Number_Of_States, or, when it has
none, State_Text written whole; with a Number_Of_States, a whole State_Text
has to label exactly that many states. So Present_Value,
Relinquish_Default, Alarm_Values and State_Text are judged against the
requested count wherever it stands, and a bad value earlier in the list than
a bad count is the one named. A refusal always names the value's own
position: `[Relinquish_Default 2, Number_Of_States 1]` is refused at 1, the
default being past the one state. With several good counts, the last one
sets the states.

A Multi-state Input or Value refuses an Alarm_Values entry past its
Number_Of_States with `PROPERTY/VALUE_OUT_OF_RANGE` naming the element, over
WriteProperty, WritePropertyMultiple, the list services and CreateObject
alike (#1429). `set_alarm_values` stays unchecked for local configuration;
on a Multi-state Value an entry past the count shows as CONFIGURATION_ERROR.
A list holding such an entry has to lose it before AddListElement or
RemoveListElement can change anything else, since each writes the whole list
back.

AddListElement and RemoveListElement edit only properties that
`BACnetObject::is_list_property` reports as a BACnetLIST. The default follows the
Clause 12 datatypes, including identifiers whose type depends on the object type
(Alarm_Values, Present_Value, Member_Of, List_Of_Object_Property_References);
custom objects with vendor lists override it. Every other target, whether a scalar,
a constructed single value, a whole array or an indexed array element, returns
`SERVICES/PROPERTY_IS_NOT_A_LIST` before any element is decoded. Unknown object,
unknown property and array-index errors come first, and element datatype errors
after. Only a BACnetLIST of BACnetDestination (Recipient_List) uses the
destination codec, only Schedule's List_Of_Object_Property_References the
reference codec (#1121), and only a Notification Forwarder's
Subscribed_Recipients the subscription codec (#1049). A list the object holds
framed with no element codec,
such as the standalone Device's COV subscription lists, returns
`PROPERTY/WRITE_ACCESS_DENIED`.

Elements compare whole (Clauses 15.1.2 and 15.2.2): two elements are the same
when their encodings are, so a destination that differs in one field is a
different destination. AddListElement leaves an element that is already present
as it is, including a repeat within the request; that is not a failure.
Subscribed_Recipients is the exception (Clause 12.51.9): an element names the
entry with the same recipient and process identifier, so AddListElement renews
that entry in place with the element's confirmation flag and Time Remaining,
and RemoveListElement removes it whatever those two members say.
RemoveListElement checks every element first and removes nothing if one is
refused: an element that does not decode as the property's element, or whose
datatype differs from the stored elements', returns
`PROPERTY/INVALID_DATA_TYPE`, and one not in the list
`SERVICES/LIST_ELEMENT_NOT_FOUND`. Both services always answer errors with a
ChangeList-Error. A refusal of an element (decode, datatype, not found) names
its position. The object judges the edited list, the stored elements followed
by the new ones in request order, through `write_property`. A refusal there
that names an element, as `Error::Structured` with
`ErrorDetail::FirstFailedElementNumber` holding its position in that list,
goes out naming the request element at that position; the built-in Alarm_Values,
Fault_Signals, Date_List and Schedule reference-list writers all name it. A
stored element named that
way is no element of the request, so the response names element 0. A refusal
that names no element, for an element's datatype, encoding, range or space,
names the first element the list would have gained, which is exact when one
element is new. Refusals of the request or target (authorization, object,
property, array index, list kind, write access) name element 0, as do the
object's refusals of what a removal leaves.

On a running server a successful edit runs the object's intrinsic-reporting
evaluation, as a WriteProperty does. An Alarm_Values edit that puts the
watched value in or out of alarm, with Time_Delay 0, moves Event_State before
the next periodic tick, and the transition's Status_Flags change goes to the
object's COV subscribers.

ReadRange reads only the same BACnetLIST properties. A scalar, a constructed
single value, a whole array (Object_List, Priority_Array) or an indexed array
element returns `SERVICES/PROPERTY_IS_NOT_A_LIST`, after the unknown object,
unknown property and array-index errors and before any By Sequence Number or By
Time error. A list the object holds framed in one `PropertyValue::ApplicationData`
is split into its elements first, so By Position counts destinations in
Recipient_List, references in Schedule's List_Of_Object_Property_References,
entries in a Notification Forwarder's Subscribed_Recipients, and subscriptions
and COV-multiple contexts in the Device's Active_COV_Subscriptions and
Active_COV_Multiple_Subscriptions. A running
server pages those two Device lists from the live COV table, through the same
Device view as ReadProperty and from one snapshot per request, so a page's
items joined in order are a run of the ReadProperty value. The standalone
`handle_read_range` pages the Device object's empty lists, as standalone
`handle_read_property` reads them. A list it cannot split (a framed list with
no element codec, such as a vendor list, or a value of another shape) returns
`SERVICES/OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`; ReadProperty still reads it
whole. Custom objects that hold a vendor list should return
`PropertyValue::List`, one value per item.

Intrinsic reporting uses one proposal/commit contract. The
`evaluate_intrinsic_reporting` and `tick_intrinsic_reporting` hooks return a
fire-ready `TransitionOutcome` while leaving event state, acknowledgment bits,
history, and the ready proposal unchanged until commit succeeds. Implement
`commit_event_transition_internal` to validate the supplied
`EventTransitionCommit`, atomically apply all object-owned transition state, and
return `EventTransitionCommitError` without mutation on failure. Custom objects
can use these public types directly; the built-in private commit kernel is not
required. Both server paths commit before distribution, even when Event_Enable,
DCC, or an empty recipient list suppresses sending. The default commit hook
returns `Unsupported`; it never authorizes an intrinsic notification.

The pre-1.0 API removes `intrinsic_reporting_requires_atomic_commit` and the
exported `impl_intrinsic_reporting!` macro. Migrate custom objects to the hooks
above; there is no alternate immediate-commit path. Standalone detector
`probe`/`tick` methods retain their own detector-local behavior.

### Constructed property framing

Built-in objects read and write these constructed values in their Clause 21
framing, through the shared `bacnet-encoding` codecs.

- **Notification Class `Recipient_List`** is a BACnetLIST of BACnetDestination
  (Clause 12.21). Each element is a seven-member sequence in the Clause 21
  order: a days-of-week set and a time window, then the recipient, process
  identifier, confirmation flag and event transitions. The recipient is a
  CHOICE: a device identifier or a network address. Decoding is strict, and a
  malformed stored list fails closed with no partial delivery. Indexed writes
  are refused. The list holds at most `MAX_RECIPIENT_LIST_DESTINATIONS` (32)
  destinations (#1098): a write that would leave more fails with RESOURCES /
  NO_SPACE_TO_WRITE_PROPERTY naming the first destination past the cap, which
  AddListElement reports as NO_SPACE_TO_ADD_LIST_ELEMENT at the request element
  that brought it. `add_destination` returns `Result` and refuses past the cap
  too, and `recipient_list()` reads the list. An address recipient's MAC is at
  most `BACnetAddress::MAX_MAC_LEN` (18) octets, the B/IPv6 form (#1124):
  `decode_destination` reads the recipient with `decode_recipient`, which
  refuses a longer one, so a write fails with PROPERTY / INVALID_DATA_TYPE and
  `add_destination` refuses it with the same code. The
  Audit_Notification_Recipient has the same bound, refused there with PROPERTY /
  INVALID_DATA_ENCODING. Every `BACnetAddress` codec holds to it (#1156):
  `decode_recipient` wherever a recipient travels (COV subscription lists,
  audit notifications and records, the GetEnrollmentSummary filter), the
  ValueSource codec and the AuditLogQuery address filters, sharing
  `check_decoded_mac_len`. Their encoders refuse a longer MAC with
  `Error::Encoding` before writing, through `check_encoded_mac_len`, so
  `encode_recipient`, `encode_destination(_list)` and the
  `encode_cov_(multiple_)subscription(_list)` family return `Result`. The
  stack stores nothing it could not encode: COV admission refuses a subscriber
  whose address is longer, and so does a remote command origin. Only the
  framed form in `PropertyValue::ApplicationData` is a
  Recipient_List value; the flat `PropertyValue::List` layout from before #152
  is refused (#1125). Routing holds every Notification Class, a custom object
  included, to the same cap: a class serving a longer list gets
  `RecipientLookupOutcome::RecipientListTooLong`, and the transition reaches
  none of its destinations. A Notification Forwarder's Recipient_List takes the
  same writes, with the same cap.
- **Notification Forwarder `Subscribed_Recipients`** is a BACnetLIST of
  BACnetEventNotificationSubscription (Clause 12.51.9): a recipient, a process
  identifier, a confirmation flag and the minutes the entry has left, under
  context tags 0 to 3 (`encode_event_notification_subscription`,
  `decode_event_notification_subscription`). The bundled
  `NotificationForwarderObject` holds the list in
  `bacnet_objects::subscribed_recipients::SubscribedRecipients`, and an
  application's own forwarder type can do the same, routing the property's
  read and write and the `*_monotonic_*_internal` clock hooks to it. The
  server forwards notifications to every live entry (see
  [Notification forwarding](#notification-forwarding)). The store keeps at most
  `MAX_SUBSCRIBED_RECIPIENTS` (32) entries and takes 1 to
  `MAX_SUBSCRIPTION_MINUTES` (1,440) minutes, refusing anything else by
  position, as a Recipient_List write does. It serves whole minutes left,
  rounded up, and the server's monotonic operation task drops an entry at its
  deadline. A rewrite keeps the deadline of each entry written exactly as it
  reads. The server's list services and ReadRange edit and page the list of any
  NOTIFICATION_FORWARDER object held this way.
- **`Event_Parameters` and `Fault_Parameters`** (Clause 12.12) use the
  BACnetEventParameter and BACnetFaultParameter CHOICE framing. Modeled
  alternatives round-trip. An alternative the stack does not model is kept as
  opaque bytes, and omitted, deprecated and reserved choices, or trailing bytes
  after a framed element, are rejected. Fault_Parameters without a fault
  algorithm reads as the context-tagged `none` choice (`08`), and writing that
  clears it. The CHOICE has no application NULL, so a NULL written to it
  succeeds and changes nothing (#1417).
- **BACnetTimeStamp** (Clause 21) has one codec for every producer and consumer.
  The time form is a primitive tag holding raw Time octets, the sequence number
  must fit 0..=65535 on both encode and decode, and the date-and-time form is an
  opening and closing tag pair around an application-tagged Date and Time.
- **Global Group `Group_Members` and `Present_Value`** (Clause 12.50) are
  arrays. A Group_Members element is a BACnetDeviceObjectPropertyReference. A
  Present_Value element is a BACnetPropertyAccessResult: the member's
  reference, then the value read or the error the read failed with. The
  application sets the members with `GlobalGroupObject::set_group_members` or
  `add_group_member` (read back with `group_members()`), which return `Result`
  and run the device reference check below, and stores the results in
  `GlobalGroupObject::present_value` as `AccessResult` values, by member
  position; a member without one reads PROPERTY / VALUE_NOT_INITIALIZED.
  Index 0 reads the array size and each index from 1 one element.
- **Group `List_Of_Group_Members` and `Present_Value`** (Clause 12.14) are
  lists, so an array index is refused. A member is a
  `bacnet_types::constructed::ReadAccessSpecification`: an object in this device
  and the properties the group reports, encoded by
  `bacnet_encoding::constructed::encode_read_access_specification`.
  `GroupObject::add_member` refuses one
  with no properties and one that would report a Group's or Global Group's
  Present_Value, returning a `GroupMemberRefusal` that names the rule and
  converts to PROPERTY / VALUE_OUT_OF_RANGE. Any property identifier is taken,
  including those ASHRAE assigns above 4194303. The object stores no Present_Value: the server rebuilds it on
  every ReadProperty, ReadPropertyMultiple and ReadRange as one
  ReadAccessResult per member, reading each member as ReadPropertyMultiple
  would, so a failed read carries its error and an object that isn't in the
  database reads OBJECT / UNKNOWN_OBJECT. Read directly from the object alone,
  Present_Value is an empty list. Every member row counts against
  `ReadPropertyMultipleBudget::max_result_elements` along with the request's
  own rows, so several Groups in one request share it; ReadProperty, ReadRange
  and `read_local` get the limit of a ReadPropertyMultiple naming only that
  Present_Value. The shared endpoint's ReadProperty uses
  `SessionConfig::read_work_limit` (default 256) instead. A request that would
  pass the limit is aborted with OUT_OF_RESOURCES (`read_local` returns
  `Error::Abort`).
- **Structured View `Subordinate_List` and Command `Action`** (Clauses 12.29
  and 12.10) are arrays too, with the same per-index reads, as is
  `Subordinate_Annotations`. A Subordinate_List element is a
  BACnetDeviceObjectReference; `add_subordinate` takes one, or an
  `ObjectIdentifier` for an object in this device, and returns `Result` (see
  device references below). `set_subordinates` replaces every subordinate
  with (reference, annotation) pairs, so both arrays keep one size, and
  `subordinates()` reads them back. An Action element is a
  BACnetActionList, the BACnetActionCommand writes that Present_Value N
  selects, framed in `[0]`. `CommandObject::set_action` takes
  `BACnetActionList` values and refuses, with VALUE_OUT_OF_RANGE, a command
  whose device identifier isn't a Device, whose priority is outside 1 to 16 or
  whose value can't be encoded. All three arrays are read-only on the
  network. Writing the Command's Present_Value runs the list it selects; see
  [Building Control](#building-control-7).
- **Load Control shed levels** (Clause 12.28): Requested_Shed_Level,
  Expected_Shed_Level and Actual_Shed_Level are `BACnetShedLevel` values, one
  context tag each: percent `[0]` or level `[1]` (Unsigned, `u64` in Rust) or
  amount `[2]` (REAL). They start at level 0, the LEVEL choice's no-shed value.
  A WriteProperty of Requested_Shed_Level must carry one of those choices;
  anything else fails with INVALID_DATA_TYPE, and a percent above 100 or an
  amount that is negative or not finite with VALUE_OUT_OF_RANGE.
  `LoadControlObject::set_requested_shed_level` applies the same checks and
  returns `Result`. Present_Value stays SHED_INACTIVE (the shed state machine
  isn't modeled), so a new requested level also resets Expected_Shed_Level and
  Actual_Shed_Level to its choice's Table 12-33 default: 100, 0 or 0.0.
  `set_actual_shed_level` refuses a level of another choice than the requested
  one.
- **Access Point `Access_Event_Time` and Credential Data Input `Update_Time`**
  are `BACnetTimeStamp` values, the unspecified date and time in the datetime
  form until the first update. Credential Data Input `Present_Value` is a
  `BACnetAuthenticationFactor`, the UNDEFINED factor until the first read.
  `CredentialDataInputObject::set_present_value(factor, update_time)` records
  a read and its time together and returns `Result`: a factor whose format
  type and class the reader doesn't declare, other than UNDEFINED or ERROR
  with class 0, is VALUE_OUT_OF_RANGE.
- **Access-control arrays**: Credential Data Input `Supported_Formats` (each
  element a `BACnetAuthenticationFactorFormat`: format type `[0]`, optional
  vendor id `[1]` and vendor format `[2]`) and `Supported_Format_Classes`
  (Unsigned), Access Door `Door_Members` and Access Point `Access_Doors` (each
  element a `BACnetDeviceObjectReference`) are BACnetARRAYs: index 0 reads the
  size, 1 to N one element, and past N fails with INVALID_ARRAY_INDEX. All four
  are read-only on the network. The application sets them with
  `CredentialDataInputObject::set_supported_formats` (format and class pairs,
  so both arrays keep one size; a format outside the closed production, a
  CUSTOM format without both vendor members or another format with a nonzero
  one is VALUE_OUT_OF_RANGE), `AccessDoorObject::set_door_members` and
  `AccessPointObject::set_access_doors` (Access Door references only, else
  VALUE_OUT_OF_RANGE), both returning `Result`. A format list that stops
  declaring Present_Value's format and class puts Present_Value back to
  UNDEFINED, with Update_Time stamped from the Device clock; out of service
  that covers the simulated factor and the reader's factor put aside.
- **Access Point `Authentication_Policy_List` and
  `Authentication_Policy_Names`** (#1325) are BACnetARRAYs of
  `bacnet_types::constructed::BACnetAuthenticationPolicy` (codec
  `bacnet_encoding::constructed::{encode_authentication_policy,
  decode_authentication_policy}`) and of CharacterString, served once
  `AccessPointObject::set_authentication_policies` sets them and read-only on
  the network. Their size is Number_Of_Authentication_Policies, capped at
  `MAX_AUTHENTICATION_POLICIES` (256) while they are served: a longer list, or
  a larger count, is VALUE_OUT_OF_RANGE. The rules for valid policies are
  under the Access Point below.
- **Access Zone `Entry_Points` and `Exit_Points`** (Clauses 12.32.23 and
  12.32.24) are BACnetLISTs of `BACnetDeviceObjectReference`, read-only on the
  network. `AccessZoneObject::set_entry_points` and `set_exit_points` set
  them and return `Result`: a reference to anything but an Access Point is
  VALUE_OUT_OF_RANGE, and the points set before are kept.
- **Access User `Credentials`, `Members` and `Member_Of`** (Clauses 12.33.12
  to 12.33.14) are BACnetLISTs of `BACnetDeviceObjectReference`, read-only on
  the network, so an index is PROPERTY_IS_NOT_AN_ARRAY. Credentials names the
  user's Access Credentials, and Members and Member_Of the Access Users one
  level below and above it. `AccessUserObject::set_credentials`,
  `set_members` and `set_member_of` set them and return `Result`: a reference
  to another object type (anything but an Access Credential for
  Credentials, or an Access User for the other two) is VALUE_OUT_OF_RANGE,
  and the list set before is kept.
- **Access Rights rules**: `Positive_Access_Rules` and `Negative_Access_Rules`
  are BACnetARRAYs of `bacnet_types::constructed::BACnetAccessRule` (codec
  `bacnet_encoding::constructed::{encode_access_rule, decode_access_rule}`),
  read whole, by index and at index 0 for the size like the arrays above.
  `BACnetAccessRule::new(time_range, location, enable)` sets each specifier
  from its reference: SPECIFIED when given, ALWAYS or ALL when `None`.
  `AccessRightsObject::set_positive_access_rules` and
  `set_negative_access_rules` return `Result` and keep the old rules on
  VALUE_OUT_OF_RANGE: a device member that isn't a Device, a specifier
  outside its two values, SPECIFIED without its reference, ALWAYS or ALL with
  a reference that isn't unspecified (instance 4194303), or a location that is
  neither an Access Point, an Access Zone nor unspecified. A list longer than
  `MAX_ACCESS_RULES` (1024) is NO_SPACE_TO_WRITE_PROPERTY. Peers write both
  arrays with WriteProperty and WritePropertyMultiple: the whole array (the
  rules' octets back to back), one rule at an index, or the size at index 0.
  Each write gets the setters' checks, and a refused one leaves the array
  as it was. Growing at index 0 appends SPECIFIED rules with unspecified
  references (Schedule 4194303's Present_Value, Access Point 4194303) and
  the enable flag FALSE (Clause 12.34.9.3); shrinking drops rules from the
  end. Enable (property 133, `PropertyIdentifier::LOG_ENABLE`) is a BOOLEAN,
  TRUE by default, set with `set_enable` or written by peers; FALSE disables
  every rule in both arrays (Clause 12.34.8) without touching each rule's own
  flag. The object stores and serves the rules and the flag, and
  `evaluate_access_rights` checks a credential against them. An object built
  with `with_persistence` keeps what peers write to the arrays and Enable
  across a restart. [Access Control](#access-control-7) covers both.
- **Access Rights Accompaniment**: the optional row (Clause 12.34.11) is one
  `BACnetDeviceObjectReference`, served only once the application sets it
  with `AccessRightsObject::set_accompaniment(Some(reference))`; until then,
  and after `set_accompaniment(None)`, it is out of Property_List and a read
  or write gets UNKNOWN_PROPERTY. It names an Access Rights, Access Credential
  or Access User object, here or in another device; a reference whose object
  and device instances are 4194303 asks for no accompaniment. The setter, and
  a peer's WriteProperty or WritePropertyMultiple of the reference's octets
  once the row is served, refuse with VALUE_OUT_OF_RANGE a device member that
  isn't a Device or any other object type (unless unspecified), keeping the
  old value. `accompaniment()` returns it. Nothing in the stack evaluates it.
- **Access Credential Authorization_Exemptions**: the optional row (Clause
  12.35.25, #1331) is a BACnetLIST of BACnetAuthorizationExemption, each an
  Enumerated, served only once the application sets it with
  `AccessCredentialObject::set_authorization_exemptions(Some(list))`, an
  empty list included; until then, and after `None`, it is out of
  Property_List and a read gets UNKNOWN_PROPERTY. It is read-only on the
  network. A value that is neither one of the seven named checks nor in the
  vendor range 64 to 255 is VALUE_OUT_OF_RANGE, keeping the old list.
  `authorization_exemptions()` returns it. With ACCESS_RIGHTS listed,
  `evaluate_access_rights` reports the credential exempt.
- **Device references**: a `BACnetDeviceObjectReference` or
  `BACnetDeviceObjectPropertyReference` whose device identifier is present
  must name a Device object (Clause 21); each type's
  `device_identifier_is_device` tells, as does
  `bacnet_types::constructed::device_identifier_is_device` for a bare
  optional identifier. Every setter that stores one, and every network write
  path, refuses one that breaks the rule with VALUE_OUT_OF_RANGE and keeps
  what it held, whatever the instance number: `set_door_members`,
  `set_access_doors`, `set_access_event`'s credential,
  `AccessZoneObject::set_entry_points` and `set_exit_points`,
  `AccessUserObject::set_credentials`, `set_members` and `set_member_of`,
  `AccessCredentialObject::set_assigned_access_rights`, the Access Rights
  rules, `StructuredViewObject::add_subordinate` and `set_subordinates`,
  `set_energy_meter_ref`, the Life Safety `add_member` and `add_zone_member`,
  the Staging configuration, `ChannelObject::set_members`,
  `GlobalGroupObject::set_group_members` and `add_group_member`,
  `EventEnrollmentObject::set_object_property_reference`,
  `TrendLogObject::set_log_device_object_property`,
  `TrendLogMultipleObject::add_property_reference` and the device identifier
  of each command `CommandObject::set_action` takes. One module in
  bacnet-objects serves every one of these properties and decodes every
  writable one (Averaging and Trend Log references, Trend Log Multiple
  Log_DeviceObjectProperty, Channel and Schedule
  List_Of_Object_Property_References, Staging Target_References), so a
  written value gets one answer whichever object takes it: another datatype,
  or a list or array element that can't open a reference, INVALID_DATA_TYPE;
  a reference that doesn't decode, or anything at all after the reference
  where the property (or an indexed element) holds one, INVALID_DATA_ENCODING;
  a non-Device member VALUE_OUT_OF_RANGE, ahead of any refusal of a remote
  device.

### ObjectDatabase

```rust
use bacnet_objects::database::ObjectDatabase;

let mut db = ObjectDatabase::new();
db.add(Box::new(analog_input));

let obj = db.get(&oid);                // Option<&dyn BACnetObject>
let obj = db.get_mut(&oid);            // Option<&mut dyn BACnetObject>
```

The pre-1.0 database API now returns `&mut dyn BACnetObject` from `get_mut`.
Structural replacement goes through `add`/`remove`; the scoped
`with_object_adapter` callback supports trusted identity-preserving wrappers
without rebinding clocks or changing indexes. It retires polling ownership before
invocation, including callback errors and unwinds, and cannot return a slot borrow.

Trend polling state now belongs to the database; the separate `TrendLogState`
argument is removed. A synchronous database poll selects, reads and appends under
one caller-owned exclusive guard. `Log_Interval` is in hundredths of a second
(raw 1 is 10 ms, raw 50 is 500 ms). Successful attempts anchor the next interval
to actual completion, with no catch-up burst. The server sleeps until the earliest
due time, capped at 100 ms to reconcile changed configuration. Invalid clocks and
insertion errors retry after 100 ms without advancing the last success. An overdue
entry after slow synchronous work yields for 1 ms instead of spinning. These are
local scheduling policies, not hard real-time guarantees. Custom polling callers
must bind the database's monotonic clock as well as its Device clock. The bundled
server binds both. Existing disabled/count-only accepted outcomes still advance
the schedule. The poll reads only this database. A `Log_DeviceObjectProperty`
naming another device logs a failure record, PROPERTY /
OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, and never a same-numbered local object;
one naming this device reads locally, as one without a Device does. A local
read that fails logs a failure record carrying its error (OBJECT /
UNKNOWN_OBJECT for a missing object) where it used to log a null value (#1183).
Indexed reference execution is not added.

`ObjectDatabase::selected_device` is the Device the database represents (the
lowest instance when it holds several). `ObjectDatabase::local_device` returns
a `LocalDevice`, whose `is_local` tells whether a reference's optional Device
member keeps it inside this device: no member, or the selected Device when its
instance isn't the wildcard. The Trend Log poller and the server's Event
Enrollment evaluation (monitored, setpoint and fault references) both resolve
references through it, so a FLOATING_LIMIT setpoint naming this device is read
and reported like an unqualified one (#1184).

#### Databases with several Devices

A BACnet device has one Device object (Clause 12.11), but a database may hold
several. The stack then treats the lowest instance as this device everywhere,
whatever order the Devices were added in (#1204):

- **This device's Device object** is `selected_device`: wildcard Device reads,
  Who-Is and Who-Has answers, I-Am, the identity in COV and event
  notifications, the startup APDU capacity check, the APDU_Timeout an Audit Log
  sink uses on receipt, and the services the standalone PICS reports.
- **Device-qualified references** go through `local_device().is_local`: Trend
  Log polling, Event Enrollment references, Command action lists, the
  Schedule, Channel, Staging and Averaging reference rewrites, and an Audit
  Log's forwarding parent. A reference naming another Device in the same
  database still points at another device.
- **Audit and endpoint identity** need `local_device` to be a concrete built-in
  Device: target Audit Reporters, the endpoint's source Audit Reporter and
  endpoint Device writes. It names this device in audit records, owns the Audit
  recipient, and is the only Device endpoint writes reach; a session identity
  must match it. While an Audit runtime is active the database refuses a new
  Device and keeps this one, so the choice holds for the runtime's life.
- **Other Devices** take no part. Their recipient, APDU_Timeout and declared
  services are plain object data.

When the only Device has the wildcard instance there is a selected Device but
no concrete identity, so Audit, endpoint Device writes, local command sources
and Audit Log forwarding refuse to start or stay unconfigured.

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

#### Value Present_Value access

The three Value families also offer `with_access`: Analog Value takes
`(instance, name, units, access)`, Binary Value takes `(instance, name, access)`,
and Multi-state Value takes `(instance, name, number_of_states, access)`.
The final argument is `bacnet_objects::present_value_access::PresentValueAccess`:

| Mode | Network-equivalent Present_Value writes | Local application updates |
| --- | --- | --- |
| `Commandable` (the `new` default) | Sourced priority commands; NULL relinquishes a slot | Use sourced commands through `write_local` |
| `Writable` | Direct replacement; supplied valid priority is ignored | Accepted while in service |
| `ReadOnly` | Denied in service; accepted while Out_Of_Service | Accepted while in service |

For the two noncommandable modes, both `BACnetObject::write_property` and
`write_property_from` enforce the same access and type/range checks. A permitted
NULL write succeeds without changing Present_Value (§19.2); an array index still
fails because Present_Value is not an array. Read-only in-service writes remain
denied, including NULL. Priority_Array, Relinquish_Default, Current_Command_Priority,
Value_Source_Array, Last_Command_Time and commandable-only Audit_Priority_Filter
are absent from these modes' projected metadata, and so is Value_Source unless
`set_value_source_tracking(true)` provisions it (see
[Source of a noncommandable Present_Value](#source-of-a-noncommandable-present_value)).
This does not disable the remaining supported AV/BV target Audit policy or add
MSV target Audit reporting. Each Value type also accepts `set_profile(ObjectProfile)`
for optional Tags and profile text rows in all three modes; Present_Value access
does not control those rows. See [Object profile rows](#object-profile-rows) for
the access-explicit Tags persistence constructors.

`BACnetServer::set_present_value_local` supplies a logical application value to
Analog/Binary/Multi-state Inputs, noncommandable Values, Loop (the control
algorithm's output) and Life Safety Point and Zone, then runs the existing event
and COV path after releasing the database lock. The corresponding low-level
`set_present_value_internal` hook bypasses those server notifications. For the
Inputs, Values and Loop both deny updates while Out_Of_Service to preserve
simulation ownership: this is local policy for Inputs and the object-clause rule
for these Values and Loop, whose Present_Value peers may write only while
Out_Of_Service is TRUE. Peers never write a Life Safety Present_Value, so those
objects take the update in either state (see
[Life Safety execution and COV](#life-safety-execution-and-cov)). Application NULL
is an invalid datatype, not a relinquishment. For network-equivalent writes use
`write_local`; noncommandable writes remain available without resolved command
identity. Commandable writes still require a valid source. These access modes are
Rust construction APIs; Python constructors retain their current defaults. A
Loop's measured input has its own route,
`BACnetServer::set_controlled_variable_value_local` (see
[Building Control](#building-control-7)).

#### Schedule & Notification (6)

| Type | Constructor |
|------|-------------|
| `CalendarObject` | `::new(instance, name)` |
| `ScheduleObject` | `::new(instance, name, default_value)` |
| `NotificationClass` | `::new(instance, name)`, `::with_persistence(instance, name, persistence)` |
| `NotificationForwarderObject` | `::new(instance, name)`, `::with_persistence(instance, name, persistence)` |
| `AlertEnrollmentObject` | `::new(instance, name, initial_source)` |
| `EventEnrollmentObject` | `::new(instance, name, event_type)` |

A `NotificationClass` built with `with_persistence` keeps a written
`Recipient_List` across a restart (Clause 12.21.8, #1315). The storage is an
application-owned `NotificationClassPersistence` that loads and saves a
`NotificationClassSnapshot`; `FileNotificationClassPersistence` keeps it in
one file, replaced whole the same way as the forwarder's file (a format of
its own, tagged `RBNNCL01`, with the same 64 KiB and 32-destination caps on
load). A class built with `new` keeps the list in memory only.

Saves follow the forwarder's rules (see
[Notification forwarding](#notification-forwarding)), and the two objects
share the code: the save runs on the class's own writer thread, and the
bundled server stages every network or `write_local` Recipient_List write
(WriteProperty, WritePropertyMultiple, AddListElement and RemoveListElement)
and waits for its save with the database guard dropped. A list that cannot be
saved is refused with DEVICE / OPERATIONAL_PROBLEM, and the class keeps the
old one. A WritePropertyMultiple that writes the list more than once stages
one save of the last (#1423). Application code writing through the database
saves in place, and a write it makes that the class refuses leaves a staged
write alone (#1424). A staged write
its request releases without making (an earlier WritePropertyMultiple attempt
failed, say) is dropped, and the class saves the list it serves at once. A
staged write whose request vanished without releasing it (`stop()` aborted
the request, or an application dropped a `write_local` future) is dropped the
same way once 10 s have passed since its save finished: by the next write
that stages, or within a further second by the server's once-a-second
operation task, which measures the time on its own monotonic clock. The
forwarder's operation task applies the same bound. Neither check runs once
the server has stopped, so `stop()`, after joining its requests, drops a
staged write still held and waits until storage holds the served list again,
and a class dropped with one still held saves the served list as it goes,
unless the staged save failed (#1363). A request already running when
`stop()` begins can still make its write, unanswered; the class then serves
that list and storage holds it (#1457). `wait_for_saves()` blocks until queued
saves have run, and dropping the class waits for them too.

A written list wins over `add_destination`, as on the forwarder:
`NotificationClassSnapshot::recipient_list` stays `None` until a write sets
the list, configured destinations are never saved and apply at every start
until then, and once a written list was saved, `recipient_list_saved()` is
true and `add_destination` checks a destination without adding it. An
AddListElement edits the list the class serves, configured destinations
included, so its result is the written list from then on. Loading a saved
list that a write would refuse (past the cap, or an address MAC past 18
octets) fails `with_persistence`.

An Event Enrollment's Object_Property_Reference, set with
`set_object_property_reference` (which returns `Result`, refusing a device
identifier that isn't a Device with VALUE_OUT_OF_RANGE), reads as the
context-tagged
`BACnetDeviceObjectPropertyReference` in one `PropertyValue::ApplicationData`,
its array index and Device members present only when set (#1182), and it
stays read-only over the network. Without a reference it reads as the unset
form, Analog Input 4194303's Present_Value (#1417), and a reference whose
object or Device is at instance 4194303 given to the setter leaves it unset.
The server's evaluation and CHANGE_OF_RELIABILITY notifications decode that
encoding, so a notification's property values carry the reference in it; the
evaluation treats the unset form as no reference.

`ScheduleObject::add_object_property_reference` retains a complete local
`BACnetObjectPropertyReference`, including its optional target array index;
`set_object_property_references` replaces the whole list. Both return `Result`
and refuse a list past 1,024 references (RESOURCES /
NO_SPACE_TO_WRITE_PROPERTY).
`ScheduleObject::evaluate(today, time, calendar_active)` calculates
Present_Value as Clause 12.24.4 orders it (#1028): within Effective_Period, the
best-priority special event in effect whose current value is not NULL (an
inline calendar entry matching `today`, or a referenced Calendar that is TRUE),
then today's weekly entry if not NULL, then Schedule_Default; outside the
period it returns `None`. Time-values are typed (`BACnetTimeValue::value` is a
primitive `PropertyValue`), so Present_Value and the target writes carry the
scheduled value's own datatype. The public `BACnetObject::tick_schedule(today,
time, calendar_active)` hook returns `Option<ScheduleWrite>` (value, priority,
references, retry): a changed value, or any value on entering the
Effective_Period (start-up included); with neither, the current value for the
references that refused their last write, flagged `retry` (#1436). The server writes it to every reference at
`Priority_For_Writing`, set with `set_priority_for_writing` (1 to 16, default
16); a NULL relinquishes that slot, and leaves a target property that isn't
commandable and has no NULL in its datatype as it is, the target counting as
one that took the write (#1416). A failed target write
does not prevent subsequent target writes. `set_weekly_schedule`,
`add_exception` and `set_effective_period` return `Result` and refuse
non-primitive values, non-specific or repeated times, out-of-range priorities
and calendar entries.

Weekly_Schedule, Exception_Schedule and Effective_Period are network-writable
(#1057): whole, or one element of either array by index. A write is decoded
with the shared codecs in `bacnet_encoding::constructed` and refused, unchanged,
wherever the setters would refuse it, with the same errors (DUPLICATE_ENTRY for
a time given twice in one list). Writing Exception_Schedule's index 0 resizes
it, appending empty events (a wholly unspecified date, priority 16); it holds at
most 1,024 events, from writes or `add_exception` (RESOURCES /
NO_SPACE_TO_WRITE_PROPERTY). After a WriteProperty, WritePropertyMultiple or
`write_local` commits to a Schedule, the server runs that Schedule's
evaluation at once, as the tick would, and fans COV out for the targets it
writes. Reliability is CONFIGURATION_ERROR, with FAULT in Status_Flags, while
the non-NULL values in Weekly_Schedule, Exception_Schedule and Schedule_Default
are not all of one datatype (#1056), or while a referenced property refused a
value of that datatype at its last write (#1086), or the reference itself: a
missing object or property, or an array index the property can't take (#1433).
The Schedule still writes its references. The server reports each write's
per-target result through the public
`BACnetObject::complete_schedule_write(write, outcomes)` hook, one
`ScheduleTargetOutcome` (`Accepted`, `DatatypeRefused` for INVALID_DATA_TYPE or
DATATYPE_NOT_SUPPORTED, `ReferenceRefused` for UNKNOWN_OBJECT,
UNKNOWN_PROPERTY, PROPERTY_IS_NOT_AN_ARRAY or INVALID_ARRAY_INDEX, `Failed`
otherwise, WRITE_ACCESS_DENIED included) per reference. A refusal clears when
that target later takes a value or leaves the list; a NULL, or an
out-of-service value of another datatype, counts for nothing. While a refusal
stands, each pass with nothing else to send offers the current value again to
the refused references alone (#1436): the 60-second tick, or the pass any
committed write to the Schedule runs. So an array grown to take the index gets
the value and clears the fault within one tick. A target object created later
doesn't wait (#1440): `ObjectDatabase::add` asks each Schedule through the
public `BACnetObject::retry_refusals_naming(target)` hook whether it holds a
refusal naming the new object, and queues those that do; the server then
writes that retry to the references naming the object alone and fans COV out,
under the CreateObject's own guard or, after the application's own `add`, from
its Schedule task. A retry that fails otherwise (an out-of-range value, a denied write) ends
the refusal as well, as that failure on a first write would never have raised
it, and warns once; one still refused logs at debug. Retries skip a NULL value
and a Schedule out of service or outside its period.

List_Of_Object_Property_References and Priority_For_Writing are
network-writable too (#1088), through the setters' checks. The list is written
whole, as the bytes a read returns, or edited by AddListElement and
RemoveListElement, which write the result back whole through the same check
(#1121). The Schedule writes only local targets, so a member naming another
device is refused with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, and one whose
device identifier isn't a Device with VALUE_OUT_OF_RANGE. A member naming the
Device the server answers for is the local reference it stands for: the server
drops that Device member before the Schedule decodes the value, and it reads
back without it (#1122). `ScheduleObject` itself can't tell which Device holds
it, so written directly it refuses every Device member. Each refusal of a
member names its position, as `Error::Structured` with
`ErrorDetail::FirstFailedElementNumber`, which AddListElement reports as the
request element. After a change the next pass sends the current Present_Value
to the new list at the new priority (in service only inside Effective_Period;
out of service at once), and relinquishes, with a NULL at the old priority,
each slot the Schedule holds that the change leaves behind: a dropped
reference, or every reference when the priority moves. A Schedule holds slots
from a write of a non-NULL value until it leaves its Effective_Period, so one
out of season clears nothing another Schedule may own.

While Out_Of_Service is TRUE, Present_Value is writable (#1055) with any
primitive value, NULL included (INVALID_DATA_TYPE otherwise, and
WRITE_ACCESS_DENIED in service), and the tick leaves it alone. Every accepted
write goes on to the references at `Priority_For_Writing`, a NULL
relinquishing, in the pass the committed write triggers. The public
`BACnetObject::take_owed_schedule_writes()` hook hands the pass what a Schedule
owes outside its calculation, once and before `tick_schedule`: the
relinquishing NULLs a change of references or priority owes, then that
written value. It needs no clock, so a value written on the object directly
goes out at the next tick. When Out_Of_Service returns to
FALSE the evaluation runs at once and takes over. A special event's priority is
a `u64` (`BACnetSpecialEvent::event_priority`): the shared codec decodes any
Unsigned there, and the object refuses one outside 1 to 16 with
VALUE_OUT_OF_RANGE, over the network and from `add_exception` alike (#1087).

`CalendarObject` evaluates Present_Value from the bound Device clock's local
date on every read (#1029): TRUE when any Date_List entry matches, FALSE
without a clock. `set_present_value` is gone; `is_active_on(day)` answers for
any `bacnet_types::calendar::SpecificDate`. `add_date_entry` returns `Result`
and refuses out-of-range entries, as Date_List writes do. Date matching for
both objects lives in `bacnet_types::calendar`.

`List_Of_Object_Property_References` now reads as `PropertyValue::ApplicationData`
containing concatenated bare context-tagged local DeviceObjectPropertyReference
bodies: object `[0]`, property `[1]`, optional target index `[2]`, and no Device
member. RP and RPM emit these bytes unchanged; an empty list has an empty payload.
The list property itself is not an array, so its own indexed requests still fail
with `PROPERTY_IS_NOT_AN_ARRAY`. This correction adds no source-origin hooks.

`AlertEnrollmentObject::new` now requires the initial
`bacnet_types::primitives::ObjectIdentifier` reported by `Present_Value`.
This is an intentional breaking correction: migrate two-argument callers by
passing the identifier of the latest alert source. Use
`record_alert_source(source)` to update only that source identity; the helper
does not evaluate an alert or update event, timestamp, acknowledgement, or
notification state. The served Table 12-61 surface no longer includes the
previous compatibility-only `Status_Flags`, `Out_Of_Service`, or `Reliability`
properties.

`bacnet_server::event_enrollment::evaluate_event_enrollments_report` returns
one `EventEnrollmentEvaluationReport` containing committed `transitions`,
`reliability_results`, and typed `diagnostics`. `ObservationUnavailable` remains
distinct from an ordinary `NoTransition`, and Reliability commit diagnostics keep
their own stage. `evaluate_event_enrollments` deliberately returns only event
transitions. The pre-1.0 duplicate detailed report API has been removed; use the
unqualified types and complete report entrypoint. The public report contract is
covered by [the external-crate tests](../crates/bacnet-server/tests/event_enrollment_report.rs).

#### Logging & Trending (5)

| Type | Constructor |
|------|-------------|
| `TrendLogObject` | `::new(instance, name, buffer_size)` |
| `TrendLogMultipleObject` | `::new(instance, name, buffer_size)` |
| `EventLogObject` | `::new(instance, name, buffer_size)` |
| `AuditLogObject` | `::new(instance, name, buffer_size, persistence)` |
| `AuditReporterObject` | `::new(instance, name)` |

`TrendLogObject::add_record`, `TrendLogMultipleObject::add_record`, and
`EventLogObject::add_record` return `Result<(), Error>`. Handle or propagate that
result: a required stop-before-full status transition fails atomically with
`DEVICE / OPERATIONAL_PROBLEM` when its clock is missing or invalid. `Ok(())`
means the operation was accepted; disabled logging can ignore the ordinary
record, zero-capacity logging can count without storing it, and a status
transition can replace it.

A Trend Log Multiple record is a `BACnetLogMultipleRecord`: a timestamp and a
`LogData` holding one `LogValue` per Log_DeviceObjectProperty member, a log
status, or a time change. `TrendLogMultipleObject::add_record` takes one and
`records()` returns them.

Each of the three logs restores its buffer with
`restore_log_buffer(total_record_count, records)` (#1537), and
`total_record_count()` and `records()` give what to save for it. The records,
oldest first, become the buffer; the newest is numbered `total_record_count`
and each earlier one one less, from 1 back to 2^32 - 1, so a count below the
number of records is a count that has wrapped (Clause 12.25.16). With no
records it seeds the count, which lets a test start a log just short of the
wrap. The restore records no status and keeps the records whatever Enable
and the window say; BUFFER_READY counts from the restored count, as when
detection starts. It fails with `Error::OutOfRange`, or a record's encoding
error, and changes nothing when there are more records than Buffer_Size,
records with a count of zero, a full buffer while Stop_When_Full and Enable
are both TRUE, or a record that would not encode. It is for start-up, before
the log goes into an `ObjectDatabase`: a running server reaches its objects
only through `BACnetObject`, which has no restore, because renumbering a log
peers are reading would change what their sequence numbers mean with no
BUFFER_PURGED record to tell them. Durable storage of a log, as the Audit
Log has, is not built yet.

A device restoring saved records after a restart should then call
`record_interruption(date, time)` with the time it came back. Clause
12.25.14 gives a log a LOG_INTERRUPTED status when a power failure or reset
broke its collection, so readers know samples before it may be missing. The
status record counts toward Total_Record_Count like any other, numbered one
past the restored count, and pushes the oldest record out of a full buffer.
It goes in whatever Enable and the window say, carrying LOG_DISABLED while
collection is off (and turning Enable FALSE when it fills a Stop_When_Full
buffer). No clock is bound before the log joins a database, so the caller
gives the time; one that isn't an actual moment fails with
`Error::OutOfRange`.

Log_DeviceObjectProperty reads as the context-tagged
`BACnetDeviceObjectPropertyReference` (#1234): one
`PropertyValue::ApplicationData` on a Trend Log, and on a Trend Log Multiple a
BACnetARRAY with one such value per element, which an array index reads singly
(index 0 is the count). A Trend Log without a reference reads as the unset
form, Analog Input 4194303's Present_Value, the empty element a Trend Log
Multiple grows by (#1417). Both are writable over WriteProperty,
WritePropertyMultiple and `write_local`, in that encoding: a Trend Log takes
one reference, any whose object or Device is at instance 4194303 unsetting it,
and a NULL succeeds and changes nothing; a Trend Log Multiple takes
the whole array, at any length up to `trend::MAX_LOG_DEVICE_OBJECT_PROPERTIES`
(64, RESOURCES / NO_SPACE_TO_WRITE_PROPERTY past it), or one element by index.
An Unsigned written to index 0 resizes it: a smaller size drops the trailing
elements, a larger one appends empty elements (Analog Input 4194303's
Present_Value), a size past 64 is NO_SPACE_TO_WRITE_PROPERTY and another
datatype INVALID_DATA_TYPE. A reference naming this server's Device
is stored without the Device member; one naming another device is
OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, except that a Trend Log Multiple element
naming instance 4194303 is an empty element and kept. A Device member that
isn't a Device identifier is VALUE_OUT_OF_RANGE, another datatype (the old flat
application-tagged form included) INVALID_DATA_TYPE, and a malformed reference,
or anything after one written alone, INVALID_DATA_ENCODING. A write that
changes the value purges the log, leaving a
BUFFER_PURGED status record; without a valid clock it fails with DEVICE /
OPERATIONAL_PROBLEM and changes nothing. The local
`TrendLogObject::set_log_device_object_property` and
`TrendLogMultipleObject::add_property_reference` return `Result`: they leave
the buffer alone and may name another device (the poller logs a failure for
it), but refuse a Device member that isn't a Device identifier, and
`add_property_reference` a 65th reference.

Trend Log and Trend Log Multiple both serve Start_Time, Stop_Time,
Align_Intervals, Interval_Offset and Trigger, and their Logging_Type is
writable (#1235, #1353, #1354); Event Log serves Start_Time and Stop_Time
(#1353). Each row has a local setter on the object, returning `Result` where
a write can be refused:

- **Logging_Type** is POLLED or TRIGGERED, through
  `set_logging_type(LoggingType)` as over the wire. A Trend Log Multiple
  never logs by COV, so COV and any other value are PROPERTY /
  VALUE_OUT_OF_RANGE (Clause 12.30.12). A Trend Log could, but this stack has
  no COV acquisition yet (#1480), so it refuses COV rather than serve a mode
  it doesn't carry out: COV and any other value are PROPERTY /
  OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, the answer Clause 12.25.26 gives for a
  value the object doesn't support. POLLED with a zero Log_Interval sets
  `trend::DEFAULT_LOG_INTERVAL` (6000 hundredths, one minute); TRIGGERED sets
  Log_Interval to 0 and makes it read-only, so a write or
  `set_log_interval` then is WRITE_ACCESS_DENIED. On a POLLED Trend Log, a
  nonzero Log_Interval written to 0 is the older way to ask for COV logging
  (Clause 12.25.9) and gets the same OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED; a
  Trend Log Multiple just stops polling at 0.
- **Trigger** written TRUE (or `trigger()`) asks a TRIGGERED log for one
  acquisition; it reads TRUE until the poller's record is accepted, and a
  successful `add_record` clears it, even when the log ignores the record
  (Enable FALSE, or outside the window). TRUE on a POLLED log is PROPERTY /
  NOT_CONFIGURED_FOR_TRIGGERED_LOGGING; FALSE is accepted and changes nothing.
- **Start_Time / Stop_Time** (`set_start_time`, `set_stop_time`) are
  BACnetDateTime values, served as an application Date then Time. Every field
  unspecified leaves that side open. Any other value has to name an actual
  date and time or it is VALUE_OUT_OF_RANGE: the weekday may be unspecified,
  and unspecified seconds or hundredths count as zero, as workstations often
  send them. Records are kept while Enable is TRUE and the local time is on
  or after the start and before the stop; each record is judged at its own
  timestamp. When the window opens or closes while Enable is TRUE the log
  records it, LOG_DISABLED on closing and a clear status on opening; a write
  records it at once, and the poller's next pass records a change that time
  brings. Enable changes while the window is shut leave logging off, so they
  add no record. The local setters are configuration: the log notes where
  the window stands without a record, so a client's write that then opens or
  shuts it is recorded. The poller looks at
  every log's window on each pass, an Event Log's included, so an opening or
  closing is recorded even when no record arrives.
- **Align_Intervals / Interval_Offset** (`set_align_intervals`,
  `set_interval_offset`) make a POLLED log acquire when the Device clock's
  time of day is Interval_Offset (modulo Log_Interval) past a multiple of
  Log_Interval, when Log_Interval divides a day. The poller checks the Device
  clock on every pass, so a clock change moves the plan with it: a boundary a
  forward jump skips isn't made up off the grid, and one a backward jump
  repeats, a daylight-saving fall-back say, is logged again when reached.

An Event Log record is a `BACnetEventLogRecord`: a timestamp and an
`EventLogDatum` holding a log status, a time change, or a notification as a
typed `EventNotificationRequest`, the parameters of a ConfirmedEventNotification
request. `EventLogObject::add_record` takes one. The request, its
`NotificationParameters` event values and `BACnetPropertyValue` live in
`bacnet_types::constructed` (bacnet-services re-exports them). Their codecs
are functions in `bacnet_encoding::constructed`: `encode_event_notification` /
`decode_event_notification`, `encode_notification_parameters` /
`decode_notification_parameters`, and `encode_bacnet_property_value` /
`decode_bacnet_property_value`. `decode_event_notification_tolerant` reads a
request whose message text doesn't decode (a character set the stack doesn't
support, for one) with no message text; the client uses it for received
notifications and `decode_event_log_record` for a record's notification, which
otherwise has to be a valid request. The request codec writes an
ACK_NOTIFICATION without its ack-required, from-state and event values, so
ReadRange serves such a record without them. A Trend Log record stays a `BACnetLogRecord`; its optional
`status_flags` is a `StatusFlags`.

A running server records its own event notifications in every Event Log. Each
notification it builds for an event or acknowledgment transition, intrinsic or
from an Event Enrollment, goes through `ObjectDatabase::log_event_notification`,
which adds a notification record stamped with the Device clock's local date and
time to each Event Log through `BACnetObject::add_event_log_record` (the
built-in `EventLogObject` takes it like `add_record`; the trait default refuses
with `OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`). The rules:

- The record holds the notification as recipients get it, with Process
  Identifier 0 in place of a recipient's own.
- It is logged when the Notification Class reads fine but selects nobody (an
  empty Recipient_List, or no destination open for that day, time or
  transition): the Recipient_List picks network recipients, not local objects
  (Clause 13.2.5).
- It is not logged when the recipient lookup fails closed: the Notification
  Class is missing, or its Recipient_List can't be read, is invalid or is past
  the cap. The server refuses that transition whole, and a record would carry a
  priority and ack policy the class never gave.
- A transition whose Event_Enable bit is off, or one made while
  DeviceCommunicationControl stops initiation, builds no notification and
  leaves no record. Both are local choices: the logs hold what the device's
  notification distribution produced.
- Notifications the server receives are logged only by the Event Logs that
  opt in (below).
- No log takes a BUFFER_READY report whose Buffer_Property names an Event Log,
  nor any report from an Event Enrollment of this device monitoring a property
  of an Event Log here. Logging either would add a record that changes what it
  watches, so reports could prompt each other without end, directly or
  crosswise between two logs. The second rule is wider because an enrollment
  running any algorithm on, say, Total_Record_Count answers each record. An
  acknowledgment of a log's report, and a Trend Log's or Trend Log Multiple's
  report, are logged like any other notification.
- Each log applies its own Enable, Start_Time / Stop_Time window,
  Buffer_Size and Stop_When_Full handling.
- Without a valid Device clock nothing is logged, since a record needs a
  timestamp.

The record is added under the database write guard that built the
notification, before the network send.

An Event Log also records the Confirmed and UnconfirmedEventNotifications the
server receives once `EventLogObject::set_log_received_notifications(true)`
opts it in (#1346; off by default, as Clause 12.27 leaves the choice to the
device). Both receive paths hand each one that decodes in full, unicast or
broadcast, to `ObjectDatabase::log_received_event_notification` before the
Notification Forwarders see it, holding no other lock; a confirmed one is
logged at its first receipt only. The record holds the notification as it
decoded, Process Identifier included; a message text in a character set the
stack can't read is left out, as `decode_event_notification_tolerant` reads
it, and a notification whose event values use a choice the codec doesn't know
isn't logged. The record goes through the log's lifecycle like any other:
Enable, the window, Buffer_Size and Stop_When_Full apply, and it counts toward
Notification_Threshold. A received BUFFER_READY report whose Buffer_Property
names an Event Log isn't logged, whichever object made it (another vendor's
Event Enrollment may run the algorithm on its log), so an opted-in log here and
a log in another device can't keep prompting each other's reports. Any other
notification about a remote Event Log is logged. Nor is one whose Initiating
Device Identifier names this device logged. A remote enrollment watching some
other property of a log here could still answer each record it causes, at most
at its source's rate.

The server holds each source to `RECEIVED_EVENT_LOG_RATE` (5) records in each
one-second window, shared by every opted-in log. A source is its network
address as `CanonicalPeer::from_source` reads it, not the Initiating Device
Identifier the sender writes, which a flooding node could vary freely.
`RECEIVED_EVENT_LOG_SOURCES` (32) sources hold an allowance of their own at a
time. A new one takes the place of one that has sent nothing for a whole
window since its last notification, and while none has been that quiet,
every other source shares one more allowance. So however many sources send,
the logs take at most 33 x 5 = 165 received records a window (up to twice
that in a second that straddles two windows), and a flood can't push the
device's own records out faster than that. Each notification past its
source's allowance is counted in
`EventNotificationCounters::received_not_logged`. With no Event Log opted in,
a received notification isn't even decoded for logging; the opt-in lives on
each log, so the check still takes the database read lock.

Event Log, Trend Log and Trend Log Multiple report intrinsically with the
BUFFER_READY algorithm (Clause 13.3.7, #1347). Each serves Notification_Threshold,
Records_Since_Notification, Last_Notify_Record, Notification_Class,
Event_Enable, Acked_Transitions, Notify_Type, Event_Time_Stamps,
Event_Message_Texts and Event_Detection_Enable, with local setters for the
first four of the writable ones (`set_notification_threshold`,
`set_notification_class`, `set_event_enable`, `set_notify_type`). Once
Notification_Threshold is set (0, the default, reports nothing), the server's
one-second intrinsic pass, or a write to the log, commits a NORMAL to NORMAL
transition each time that many records have been collected since
Last_Notify_Record, and the recipients of the log's Notification Class get a
BUFFER_READY notification naming the log's Log_Buffer in this device with the
previous and current Total_Record_Count. Records that arrive within one pass
make one report spanning them all. Event_Enable's TO_NORMAL flag (set by
default) decides whether it goes out; Notify_Type defaults to EVENT. An
Event Log's report goes in no log, its own included, so it never counts
toward its next one. A Trend Log's or Trend Log Multiple's report is logged in
the Event Logs like any other notification; those logs take no notifications,
so it can't count toward their own next report. A purge restarts Records_Since_Notification at the BUFFER_PURGED
record but leaves the threshold counting from Last_Notify_Record;
Event_Detection_Enable TRUE again restarts both from the current count.
While Event_Algorithm_Inhibit is TRUE no report goes out; the records keep
counting, so one falls due as soon as it clears.

Every object here that reports intrinsically (the analog, binary and
multi-state families, Access Door, Access Zone and the three logs) also
serves Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair
(#1329), all three writable:

- Event_Message_Texts_Config holds one CharacterString per transition,
  TO_OFFNORMAL, TO_FAULT and TO_NORMAL. A non-empty entry replaces the
  server's own Message Text for that transition, in the notification and in
  Event_Message_Texts; an empty one, the default, leaves it. The text goes
  out as written: the stack defines no substitution codes.
- Event_Algorithm_Inhibit TRUE stops the event algorithm but not fault
  detection (Clause 13.2.2.1): no offnormal or normal transition of its own,
  any time delay under way dropped, and an offnormal object back to NORMAL at
  once. Once it is FALSE, a condition has to last its whole Time_Delay again.
  A client writes it while Event_Detection_Enable is TRUE and there is no
  reference.
- Event_Algorithm_Inhibit_Ref names a Boolean or BinaryPV property of this
  device (the datatype has no device member) for the inhibit to follow, and
  the inhibit is then read-only. The server reads the property, through
  `ObjectDatabase::follow_event_algorithm_inhibit`, each time it evaluates
  the object: on a write to it and on the one-second tick, so a change
  reaches the inhibit within a second. A Boolean TRUE inhibits, and so does
  ACTIVE read from a property known to hold a BinaryPV: Present_Value,
  Relinquish_Default, a Priority_Array element, Alarm_Value and
  Feedback_Value of the binary types, and an Access Credential's
  Credential_Status. Anything else doesn't, an Event_State of FAULT or a
  Reliability that reads as Enumerated 1 included, nor does a missing
  property. Unset, it reads as Binary Value 4194303's Present_Value, and
  writing that clears it and puts the inhibit back to FALSE.
- Event_Message_Texts_Config is always three entries: the tables fix its
  size, so a write at index 0 is `PROPERTY/WRITE_ACCESS_DENIED` (Clause
  12.1.5.1).

Every Trend Log samples a BACnet property, so its Start_Time, Stop_Time,
Log_Interval and Log_DeviceObjectProperty are classed required (Table 12-29
footnotes 1 and 8, #1481): RPM with REQUIRED returns them, and the PICS lists
them as required. Log_Interval is classed as on a Trend Log Multiple: a
required readable row whose write capability follows Logging_Type.

Every record kind, the Audit Log's included, carries a log status as the
typed `bacnet_types::bitstring::LogStatus` flags (`LOG_DISABLED`,
`BUFFER_PURGED`, `LOG_INTERRUPTED`). The codecs send bit 0 first, as for
every BACnet bit string: log-disabled is `05 80`, buffer-purged `05 40`,
log-interrupted `05 20`. `LogDatum` and `LogValue` keep INTEGER values as
`i64` and ENUMERATED and Unsigned values as `u64`. Clause 21 lets a logging
device hold these to 32 bits but doesn't require it, so a record read from a
peer may carry wider values, and the decoders accept up to eight octets.

`add_record` and the trend hooks refuse a record that would not encode (an
any-value, or a notification's raw event values, whose tags don't balance; a
complex event's property priority outside 1 to 16; a bit string with
impossible padding) with its encoding error, before anything changes. So
`LogBufferRecords::encode_record` cannot fail, and one bad record can't break
every ReadRange window over the log.

Trend Log Multiple, Trend Log and Event Log objects list `Log_Buffer` in their
Property_List, but ReadProperty and ReadPropertyMultiple answer it with
`PROPERTY / READ_ACCESS_DENIED` (also inside RPM `ALL` and `REQUIRED`):
Clauses 12.25.14, 12.27.13 and 12.30.19 make the buffer reachable only
through ReadRange. ReadRange reads each object's records through
`BACnetObject::log_buffer_internal`, a `LogBufferRecords` view, and returns
each item as one record framed as its Clause 21 production:

| Object | Record | Codec in `bacnet_encoding::constructed` |
|--------|--------|------------------------------------------|
| Trend Log | BACnetLogRecord | `encode_log_record` / `decode_log_record` |
| Event Log | BACnetEventLogRecord | `encode_event_log_record` / `decode_event_log_record` |
| Trend Log Multiple | BACnetLogMultipleRecord | `encode_log_multiple_record` / `decode_log_multiple_record` |
| Audit Log | BACnetAuditLogRecord | `encode_audit_log_record` / `decode_audit_log_record_at` |

Each decoder returns the offset after the record, so a client walks a
ReadRange ACK's `item_data` record by record.

A ReadRange no longer builds every record's identity (#1536): only the
returned window's records are encoded and given identities.
`LogBufferRecords::record_identity` numbers any record from
Total_Record_Count, and `record_position` computes where a sequence number
sits, so on a Trend Log, Event Log or Trend Log Multiple By Position and By
Sequence Number cost the same over a full log as over a short one. By Time
bisects the timestamps while `timestamp_order` reports them `Ascending`,
which the buffer keeps current as records come and go. Some reads still walk
records:

- By Time on a log whose clock has been set back, so that an earlier stamp
  follows a later one (`Unordered`), walks the log until it finds its anchor.
- By Time on an Audit Log walks the whole ring, validating every timestamp.
- By Sequence Number on an Audit Log computes the number's place in the ring,
  which numbers on by one from its oldest record, and checks the record
  there. When that record has another number (the number isn't resident, or
  the store skips numbers), it walks the ring twice: once for a record
  numbered zero, once to search.

A timestamp that isn't an actual moment (`Unkeyed`) refuses By Time reads
with `LIST_ITEM_NOT_TIMESTAMPED` until that record leaves the buffer. A
custom `LogBufferRecords` must supply `record_identity`; the other two
lookups default to walking the records.

The poller logs a value whose
datatype has no alternative of its own (a CharacterString, Double, Date,
ObjectIdentifier, whole array and so on) as `AnyValue` holding the value's
own encoding, the bytes a ReadProperty of it carries; NULL is logged only
for a NULL value. An any-value holds at most
`bacnet_objects::log_buffer::ANY_VALUE_MAX_OCTETS` (256) octets of encoding.
A longer value is logged as a `PROPERTY / VALUE_TOO_LONG` failure, so a Trend
Log record stays small enough for a ReadRange page on a 480-octet APDU. A
value no record could carry is logged as `SERVICES / OTHER`. Records an
application adds itself are not capped.

The pre-1.0 `BACnetObject` contract has two fallible trend hooks:
`add_trend_record` for Trend Log records and `add_trend_multiple_record` for
Trend Log Multiple records. The void hook and `try_add_trend_record_internal`
adapter have been replaced. Custom implementations return their insertion result
directly; the default returns `OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`.
The server poller (`ObjectDatabase::poll_trend_logs`) samples both object types
and retries failed insertions without advancing its last-log time. It also
makes one acquisition for each Trigger of a TRIGGERED log (a Trend Log
Multiple with no members records an empty set of values, and a Trend Log
with no reference a PROPERTY / NO_PROPERTY_SPECIFIED failure, so Trigger
never stays TRUE), waits for each clock-aligned boundary of an aligned POLLED
log, and on every pass calls the hidden `refresh_log_window_internal` hook on
each Trend Log, Trend Log Multiple and Event Log, so each records its window
opening or closing. Wrappers forward that hook, as `SourceReporter` does.
`TrendLogObject::set_logging_type` returns `Result`, as Trend Log
Multiple's does. Only POLLED and TRIGGERED logs are polled. Complete log-family
conformance is not claimed.

#### Audit Reporter configuration and send delay

Trusted local configuration through `dyn BACnetObject` uses one atomic
`configure_audit_reporter_internal(level, operations, confirmed, selectors, priorities, maximum_send_delay)`
contract. It replaces all six settings once; invalid or resource-denied changes
leave every field unchanged. The final argument is `Option<AuditSendDelay>`:
`None` omits both delay/control properties, while `Some(AuditSendDelay::new(0)?)`
exposes the pair with immediate delivery. Positive values enable bounded target
batching; see [delay controls and limits](delayed-target-audit.md). Built-in live
setters and Description writes share that boundary and return Result. `None` selectors remove Monitored_Objects and
select all nominal targets; `Some(vec![])` retains an empty property and selects
none. Runtime ownership prepares mandatory change notifications before committing.
The private endpoint source adapter forwards the contract while rejecting a present
delay capability and preserving exactly-one source-role ownership. See
[target Reporter ownership and live changes](target-audit-reporters.md).

#### Building Control (7)

| Type | Constructor |
|------|-------------|
| `LoopObject` | `::new(instance, name, output_units)` |
| `CommandObject` | `::new(instance, name)` |
| `TimerObject` | `::new(instance, name)` |
| `LoadControlObject` | `::new(instance, name)` |
| `ProgramObject` | `::new(instance, name)` |
| `AveragingObject` | `::new(instance, name)` |
| `StagingObject` | `::new(instance, name, StagingConfig { ... })` |

The Loop serves every required Table 12-20 row. Setpoint, the gain constants,
Update_Interval and Action (DIRECT or REVERSE) take network writes.
Controlled_Variable_Units, the three gain units rows and Priority_For_Writing
are read-only over the network; set them before adding the Loop with
`set_controlled_variable_units`, `set_proportional_constant_units`,
`set_integral_constant_units`, `set_derivative_constant_units` and
`set_priority_for_writing`. The object stores the loop's configuration and
output for the application's algorithm: it neither computes Present_Value nor
writes it to the Manipulated_Variable_Reference target.

Controlled_Variable_Reference and Manipulated_Variable_Reference read as the
context-tagged `BACnetObjectPropertyReference` in one
`PropertyValue::ApplicationData`. While unset they read as the unset form, the
reserved instance 4194303's Present_Value of an Analog Input and an Analog
Output respectively (#1417). Setpoint_Reference reads as the
`BACnetSetpointReference`: the same members inside opening and closing tag 0,
or an empty `ApplicationData` while unset, since the sequence's only member is
optional (#1312). All three take writes in those encodings over
WriteProperty, WritePropertyMultiple and `write_local`, so a value read writes
back unchanged; any reference to instance 4194303 clears a variable reference
(so do the setters given one), and the empty value clears Setpoint_Reference.
Another datatype is INVALID_DATA_TYPE: the flat
`[ObjectIdentifier, Enumerated, Unsigned?]` list these used to read as, Null,
or the setpoint frame on a variable reference. The server turns that refusal
of a NULL into the success that changes nothing (#1396, #1417).
Malformed octets, such as a Device member `[3]` the production lacks or an
empty frame `0E 0F`, are INVALID_DATA_ENCODING. These refusals are the device
references' single-reference codes (#1395): anything after the one reference
is INVALID_DATA_ENCODING whatever its tag, and a value passed to
`write_property` as a list mixing raw chunks with decoded values is
INVALID_DATA_TYPE. An empty list passed that way is the empty value: it is
INVALID_DATA_ENCODING on a variable reference and clears Setpoint_Reference.
The `set_*_reference` setters still take a `BACnetObjectPropertyReference`.
An application that follows the references decodes what it reads with
`bacnet_encoding::constructed::decode_object_property_reference`, or
`decode_setpoint_reference`, which gives `None` for the empty value.

While the application runs the algorithm, it also feeds Controlled_Variable_Value,
the measurement the algorithm compares with Setpoint. The server doesn't follow
Controlled_Variable_Reference. In a running server the application calls
`BACnetServer::set_controlled_variable_value_local(&loop_id, PropertyValue::Real(v))`
alongside `set_present_value_local` for the output. The property stays
read-only over the network. The call takes a finite REAL and refuses any other
object with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Unlike Present_Value, it is
accepted while Out_Of_Service is TRUE, because Out_Of_Service decouples only the
output and Reliability. The change goes through the server's COV path: a
SubscribeCOVProperty on Controlled_Variable_Value is notified, while a SubscribeCOV
on the Loop carries the value in its next report without being triggered by it.
Before the Loop is added, `LoopObject::set_controlled_variable_value` sets the
starting value.

A Command object runs one of its Action lists each time its Present_Value is
written (Clause 12.10). Give it the lists with `CommandObject::set_action`
before adding it, and optionally one description per list with
`set_action_text`, which serves Action_Text and needs exactly one text per
list; both arrays are read-only on the network. Present_Value N selects list
N. A number above the list count is VALUE_OUT_OF_RANGE, and zero or an empty
list writes nothing and sets All_Writes_Successful TRUE. Otherwise In_Process
turns TRUE and All_Writes_Successful FALSE, and every Present_Value write is
OBJECT / BUSY until the run ends. A running server makes the commands in order
through the `write_local` path, each at its own priority with the Command as
the initiating object, and waits out a command's `post_delay` in the run's own
task, so the write that started the run is answered at once. Each command's
`write_successful` flag records its outcome; a failure with `quit_on_failure`
set stops the list and marks the rest unsuccessful. At the end In_Process
returns to FALSE, with All_Writes_Successful TRUE only if every write
succeeded. A Schedule writing a Command's Present_Value starts the run as
well. Command takes SubscribeCOVProperty but not SubscribeCOV, so a client can
follow In_Process.

A command whose `device_identifier` names another device goes there as a
confirmed WriteProperty (#1180). The address comes from the server's device
bindings: a `DeviceBinding` registered on the builder, or an I-Am the server
heard in the last ten minutes. A binding never takes a group address of the
link (`TransportPort::is_group_destination`), as the device's own MAC or as its
router's: one registered so stops the server from starting, with an error
naming the device and the address, and an I-Am from one binds nothing (#1493). For a device with neither, the server first
broadcasts one Who-Is whose low and high limits are both that device's
instance (#1322). A device it has never heard from is asked on every network
(a global broadcast, DNET 65535). One whose stale I-Am is still held is asked
where that I-Am came from: the local network, or the remote network it was
routed from, where a remote network numbered as this device's own counts as
local. If that Who-Is draws nothing, the stale I-Am is dropped, so the
device's next Who-Is goes global. The server then waits
`ServerConfig::cov_retry_timeout_ms`, but never more than a minute (#1368),
counted from the send, for the I-Am, which binds the device as any I-Am does,
and the write goes ahead; with no I-Am by then the command fails and no
WriteProperty is sent. Writes that miss
while that Who-Is is out share it and its wait. A device gets at most one
Who-Is a minute, counted from when it went out, so a command naming it within
a minute of one that drew nothing fails at once. At most 256 devices with a
Who-Is out or held off are tracked, and a command needing another fails
unsent; a device that answers frees its place at once and stays bound for
ten minutes, so the cap limits unanswered Who-Is requests to 256 a minute.
The wildcard instance 4194303 is never looked for. A binding routed through
the network numbered as this device's own, once the server knows that number
(see [Local Network Number controls](#local-network-number-controls)), is a
device on the local network: the WriteProperty goes to its final MAC with no
DNET, not through its router, and the answer is awaited from that MAC
(#1358). Each attempt waits `ServerConfig::cov_retry_timeout_ms` (3
seconds by default) for the answer, and only silence earns another attempt,
up to three retries under the one invoke ID. An Error (BUSY included), Reject
or Abort fails the command at once. Nothing is sent while
DeviceCommunicationControl restricts initiation, a Who-Is included, and the
run holds no database guard while the write is outstanding or waits for an
I-Am. Naming this server's own Device is the same as naming none.

A run that `stop()` cuts short isn't resumed. It ends where it stood
(#1252): In_Process returns to FALSE, each command it hadn't made reads
unsuccessful, and All_Writes_Successful is TRUE only if every write had
already been made and succeeded, as when the stop falls in the last command's
post delay. A Channel's distribution with members left unwritten ends FAILED,
with Reliability PROCESS_ERROR unless a member had already failed. Once the
server's own work has stopped, `stop()` also ends any run still in progress on
the database, such as one a write made straight into the database queued.
`stop()` doesn't wait for a database the application holds: those runs end as
soon as it lets go.

A `write_local` dropped after its write committed, by a timeout, a `select!`
or a cancelled Python task, skips nothing (#1367): the commit hands its
database guard, and the event pass, COV fanout, Schedule fanout, Staging plan
and runs the write owes, to a task of its own in the server's request task
set, which the call only waits for. The run the write started goes ahead and
reports its end. `stop()` aborts that task with the other request tasks, and
a run it hadn't started then ends as if none of its writes were made (#1324):
In_Process FALSE with every command unsuccessful, or a Channel's Write_Status
FAILED with Reliability PROCESS_ERROR. The call still returns `Ok(())` then,
since the write was made. `set_life_safety_operation_expected_local` hands
its timestamped capture and COV fanout to such a task the same way, so a
caller dropped once Operation_Expected has changed still notifies its
subscribers (#1520). Every local write (`write_local`,
`write_local_encoded`, `set_present_value_local` and the other `*_local`
setters) must therefore be awaited inside a Tokio runtime: outside one it
fails with `Error::Encoding` before anything is written.

Whatever commits a Present_Value write owns the run it starts and finishes it,
so no path leaves a Command in process (#1178). Without a server,
`tick_schedules` runs the lists its Schedule writes start before it returns,
post delays included, making each command as the bare WriteProperty handler
would with the Command as the initiating object; it has no network, so a
command naming another device fails. Dropping its future first ends each
unfinished run as unsuccessful. The bare `handle_write_property` and
`handle_write_property_multiple` handlers are synchronous and make no writes
for a Command: the run a Present_Value write starts ends at once, with
In_Process FALSE, All_Writes_Successful FALSE and every command's
`write_successful` FALSE. The endpoint responder refuses a Command's
Present_Value write with WRITE_ACCESS_DENIED, as it does every write other
than the Device's Description and Audit recipient.

Load Control supports COV (Table 13-1). Its SubscribeCOV report carries
Present_Value, Status_Flags, Requested_Shed_Level, Start_Time and
Shed_Duration, and a change of any of them sends one. Duty_Window, which the
row also names, isn't served yet.

A running server samples an Averaging object's Object_Property_Reference
itself, every Window_Interval / Window_Samples seconds (#1144). The first sample
comes one spacing after the server starts, and each write that empties the
window (below) starts the spacing over. The spacing never drops below
`averaging::MIN_SAMPLE_PERIOD` (100 ms); a shorter configured one is stretched,
so that window spans more than Window_Interval. A referenced object or property
that doesn't exist, an array index on a property that isn't an array, a failed
read, or a value of a datatype the object can't average counts as a missed
attempt. References are always local: a written reference naming another
device is refused (see below). The server's monotonic operation task does this through
`ObjectDatabase::sample_due_averaging_objects`, which an application driving
its own database can call as well.

The application feeds an object that has no reference. In a running server it
passes each result to
`BACnetServer::add_averaging_sample_local(&averaging_id, Some(value))`, or
`None` for an attempt that produced no value (a failed read, say). On an object
the server samples, such a call is one more attempt and leaves the server's
spacing alone. Before the object is added, `AveragingObject::add_sample(v)` does
the same for an `f32` and `add_missed_sample()` for a miss. The value may be a
BOOLEAN (FALSE and TRUE count as 0 and 1), Signed, Unsigned, Enumerated or
finite REAL, since the object computes in REAL. Another datatype, Double
included, fails with INVALID_DATA_TYPE and NaN or an infinity with
VALUE_OUT_OF_RANGE, and a refused sample counts as neither attempted nor valid.
Any object other than an Averaging object refuses the call with
OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.

The object keeps the most recent Window_Samples attempts (15 by default, at most
`MAX_WINDOW_SAMPLES` = 1440), and each sample, the server's or the
application's, fills the next slot. Window_Interval is 900 s by default.
Minimum_Value, Maximum_Value and Average_Value cover the valid samples in the
window, Attempted_Samples counts the attempts in it and Valid_Samples the valid
ones, so a miss shows up as the difference. With no valid sample in the window,
the statistics read positive infinity, negative infinity and NaN.
Window_Interval and Window_Samples are writable over the network and through
`set_window_interval` and `set_window_samples`; a write of either, of
Object_Property_Reference, or of zero to Attempted_Samples empties the window. A
zero interval, a sample count of zero or above the bound, and a nonzero
Attempted_Samples fail with VALUE_OUT_OF_RANGE and change nothing.

Each sample changes the statistics and counts together, then the server's COV
path runs, as it does after a write. Averaging has no Table 13-1 row, so
SubscribeCOV on it is refused (`supports_cov` is false), but it takes
SubscribeCOVProperty and SubscribeCOVPropertyMultiple
(`supports_subscribe_cov_property` is true): a numeric property is reported
when it moves by the subscription's COV increment, or on any change if the
subscription gives none, and the report carries no Status_Flags because the
object has none. A move to or from the NaN or an infinity of an empty window is
always reported, whatever the increment, and staying at one never is.

An Averaging object samples only properties in its own device. A written
Object_Property_Reference naming another device is refused with
OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. One naming the Device the server answers
for is the local reference it stands for: the server drops that Device member
before the object decodes the value, on WriteProperty, WritePropertyMultiple
and `write_local`, and it reads back without it (#1153). `AveragingObject`
itself can't tell which Device holds it, so written directly it refuses every
Device member.

Object_Property_Reference reads as the context-tagged
`BACnetDeviceObjectPropertyReference`, a `PropertyValue::ApplicationData` with
no Device member, and a write takes that encoding back (#1182). While unset it
reads as Analog Input 4194303's Present_Value, and writing a reference whose
object or Device is at instance 4194303 unsets it; a NULL succeeds and changes
nothing (#1417). The flat application-tagged list reads used to serve is now
INVALID_DATA_TYPE, as are octets that don't open with the object
identifier's context tag 0 (#1312); anything after the one reference is
INVALID_DATA_ENCODING, and a Device member that isn't a Device identifier
VALUE_OUT_OF_RANGE. The server hands the written octets over whole, as for a
Trend Log (#1313).

Staging uses an explicit atomic configuration; the former stage-count-only
constructor is intentionally removed because it could not create a valid
ladder or target mapping. To migrate to 0.11.0, replace that argument with a
`StagingConfig` containing the initial value, minimum, units, priority, at least
two ordered stages, and local target references. Each stage's `values` must
have one entry per target; optional `stage_names` must have one name per stage.
Construction returns an error for invalid configuration, so preserve the
fallible result handling:

```rust
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

let target = BACnetDeviceObjectReference {
    device_identifier: None,
    object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1)?,
};
let staging = StagingObject::new(
    1,
    "Two-stage fan",
    StagingConfig {
        present_value: 5.0,
        min_present_value: 0.0,
        units: 62,
        priority_for_writing: 8,
        stages: vec![
            BACnetStageLimitValue {
                limit: 10.0,
                values: vec![false],
                deadband: 1.0,
            },
            BACnetStageLimitValue {
                limit: 20.0,
                values: vec![true],
                deadband: 1.0,
            },
        ],
        target_references: vec![target],
        stage_names: Some(vec!["Off".into(), "On".into()]),
    },
)?;
# Ok::<(), bacnet_types::error::Error>(())
```

Staging targets are local-only Binary Output, Binary Value, or Binary Lighting
Output objects. A written target naming another device is refused with
OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. One naming the Device the server answers
for is the local reference it stands for: the server drops that Device member
before the Staging object decodes the value, on WriteProperty,
WritePropertyMultiple and `write_local`, and it reads back without it (#1136).
`StagingObject` itself can't tell which Device holds it, so written directly
or configured through `StagingConfig` it refuses every Device member. The
server applies stage changes through its ordinary local
write notification path at `priority_for_writing`, completing the bounded local
plan during write handling without remote I/O. A target failure sets source
`Reliability` to `UNRELIABLE_OTHER`; a later fully successful current plan
clears it. `Out_Of_Service` decouples targets while preserving PV/stage
evaluation, and returning to service reapplies the selected stage. Network
writes may replace individual or whole `Stages`, `Target_References`, and
configured `Stage_Names` arrays, but array lengths are fixed after construction
so coupled configuration cannot pass through an invalid intermediate shape.
`Target_References` reaches the object as the written octets and is decoded
by the shared device reference helpers, so an element of another datatype is
INVALID_DATA_TYPE (#1313).
`Max_Pres_Value` is derived from the final stage limit. Staging does not
advertise intrinsic reporting. It supports COV (Table 13-1): a SubscribeCOV
notification carries Present_Value, Status_Flags and Present_Stage, and fires
when Present_Value moves by the writable `COV_Increment` (default 0), when
Status_Flags changes (including a target-plan completion that changes
Reliability), or when Present_Stage changes.

#### Lighting & Color (5)

| Type | Constructor |
|------|-------------|
| `ChannelObject` | `::new(instance, name, channel_number)` |
| `LightingOutputObject` | `::new(instance, name)` |
| `BinaryLightingOutputObject` | `::new(instance, name)` |
| `ColorObject` | `::new(instance, name)` |
| `ColorTemperatureObject` | `::new(instance, name)` |

Lighting Output's `Default_Fade_Time`, `Default_Ramp_Rate` and
`Default_Step_Increment` are writable over the network and through
`set_default_fade_time`, `set_default_ramp_rate` and
`set_default_step_increment`. A new object uses 100 ms, 100.0 %/s and 1.0 %.
A fade time outside 100 to 86,400,000 ms, or a rate or increment outside 0.1
to 100.0, is refused with VALUE_OUT_OF_RANGE (Clauses 12.54.16 to 12.54.18).
Both lighting objects serve `Current_Command_Priority`.

Lighting Output's Present_Value and Relinquish_Default take a level from 0.0
(off) to 100.0 percent. A level above 0.0 and below 1.0 is stored as 1.0, the
dimmest on level (Clause 12.54.4), so the priority slot, Present_Value,
Tracking_Value and COV reports all carry 1.0 (#1385). A level below 0.0 or
above 100.0, NaN included, is refused with VALUE_OUT_OF_RANGE, except
Present_Value's warn values -1.0, -2.0 and -3.0 (Table 12-65), which act as
WARN, WARN_RELINQUISH and WARN_OFF at the write's priority and never enter the
priority array (#1384).

Lighting Output's `Lighting_Command` holds a `BACnetLightingCommand`
(`bacnet_types::constructed`): an operation plus an optional target level, ramp
rate, step increment, fade time and priority (#1263). It reads operation NONE
until written. Over the network it travels as the command's context-tagged
fields, which `bacnet_encoding::constructed::encode_lighting_command` writes and
`decode_lighting_command_value` reads (`decode_lighting_command` reads one at
an offset inside a larger value); locally the object reads as
`PropertyValue::ApplicationData` holding those octets, and
`set_lighting_command` and `lighting_command` take and return the typed value.
Each command is checked against its operation (Clause 12.54, Table 12-67):

- NONE, the reserved operations 11 to 255 and anything past 65,535 are refused.
- FADE_TO and RAMP_TO need a target level.
- A field the operation uses must be in range: target level 0.0 to 100.0, fade
  time (FADE_TO) 100 to 86,400,000 ms, ramp rate (RAMP_TO) and step increment
  (the four step operations) 0.1 to 100.0, and priority 1 to 16.
- A field the operation doesn't use is kept as written without a check. A
  proprietary operation (256 to 65,535) has only its priority checked, the
  same check a Channel makes of a lighting command written to it.

A refused command is VALUE_OUT_OF_RANGE. Any other datatype, an OCTET STRING
included, is INVALID_DATA_TYPE, and octets that aren't exactly one command are
INVALID_DATA_ENCODING, even when a field is also too wide for its type. An
Unsigned or ENUMERATED field may open with zero octets only up to four contents
octets.

The object carries out each command it takes (#1384), at the command's
priority or `Lighting_Command_Default_Priority`, and `Lighting_Command` keeps
reporting it as written. `Lighting_Command_Default_Priority` takes 1 to 16 but
not 6, Minimum On/Off's priority (Clause 12.54.27). A level it puts in a slot is normalized as a
commanded Present_Value is, so a FADE_TO 0.5 puts 1.0 in the slot.

- FADE_TO and RAMP_TO put the target level in the slot. When that slot is then
  the highest in use, Tracking_Value moves in a straight line from where it
  stood to the level, over the fade time (or `Default_Fade_Time`) or at the
  ramp rate (or `Default_Ramp_Rate`), and In_Progress reads FADE_ACTIVE or
  RAMP_ACTIVE until it arrives.
- The step operations put Tracking_Value plus or minus the step increment (or
  `Default_Step_Increment`) in the slot, kept within 1.0 to 100.0. STEP_UP and
  STEP_DOWN do nothing while off; STEP_ON turns off into 1.0, STEP_OFF turns
  1.0 into off.
- With `Blink_Warn_Enable` FALSE (the default) the warn operations act at once:
  WARN changes nothing, WARN_RELINQUISH relinquishes the slot and WARN_OFF
  writes 0.0 to it. With it TRUE they blink and, where the table's conditions
  call for one, hold the level for `Egress_Time` seconds with `Egress_Active`
  TRUE before relinquishing the slot or writing 0.0.
- STOP ends a fade or ramp at its priority with Tracking_Value in the slot, or
  cancels an egress timer there; elsewhere it is ignored.
- A command other than STOP, or a Present_Value write, at the priority of the
  command in progress or a higher one halts it (Clause 12.54.6.1): a fade or
  ramp stops with its slot as it is, and an egress takes effect at once.
- A proprietary operation is stored and does nothing else.

Lighting Output takes the trims of Addendum 135-2020ca part 5 (#1528).
`High_End_Trim` and `Low_End_Trim` are absent until `set_high_end_trim` or
`set_low_end_trim` sets them (`None` takes one away), and either brings
`Trim_Fade_Time` (0 to 86,400,000 ms, initially 0; `set_trim_fade_time`),
which the table's footnote then makes required. All three take writes; a
trim outside 1.0 to 100.0, or a fade time past a day, is refused with
VALUE_OUT_OF_RANGE.

- Tracking_Value is held between the trims: an on level below the low trim
  tracks at it, a level above the high trim tracks at that, and off stays
  off. Present_Value keeps the level as commanded.
- In_Progress reads TRIM_ACTIVE (5, `LightingInProgress::TRIM_ACTIVE`),
  ahead of FADE_ACTIVE or RAMP_ACTIVE, while Present_Value lies outside the
  trims, and while moving trims (below) hold Tracking_Value back; the second
  case is this implementation's, so that IDLE always means Tracking_Value
  equals Present_Value.
- The trims stand aside while Present_Value comes from slot 1 or 2.
- A trim change moves the trims themselves, in a straight line over
  Trim_Fade_Time, and Tracking_Value follows them, sampled for COV as a fade
  is. A fade or ramp under way runs on behind the moving trims.
- Commands work from Tracking_Value as reported, so a step starts from the
  held level, STOP leaves the held level in the slot, and a fade to a level
  past a trim reaches the trim early and stays there. An egress holds the
  trimmed level. A STEP_UP while Present_Value is above the high trim writes
  the trim plus the increment, so it lowers Present_Value, as Table 12-67
  has it.
- The levels between off and the low trim are outside the operating range,
  so a fade or ramp doesn't crawl through them: one up from off starts at
  the low trim and runs to its target over its whole time, and one down to
  off runs to the low trim over its whole time and goes off as it ends, both
  reading FADE_ACTIVE or RAMP_ACTIVE. The addendum leaves this to the
  implementation.
- STEP_OFF turns the light off from the low trim while one applies, rather
  than only from 1.0; STEP_DOWN still stops at 1.0, so Present_Value can go
  below the trim while Tracking_Value stays at it.
- A high trim below the low one sets Reliability to CONFIGURATION_ERROR (with
  Status_Flags' FAULT), and the trims hold nothing until it's put right,
  which gives back the Reliability the error replaced.

Both lighting outputs take the colour links of Addendum 135-2020ca part 4
(#1527). `set_color_link` takes a `bacnet_objects::lighting::ColorLink`: a
`reference` (Color_Reference) and, where the output supports colour override,
a `ColorOverride` with its `active` flag (Color_Override) and `reference`
(Override_Color_Reference). The rows are absent until a link is set, then
required while present, as the tables' footnotes say, and all of them take
writes. A reference must name a colour object, Color or Color Temperature,
or a write is refused with VALUE_OUT_OF_RANGE. Instance 4194303 names none,
leaving the colour to the application, and is taken with any object type:
the clauses give the instance alone that meaning.

The outputs only store the references, and the override never writes a
colour object: it switches which object the colour comes from, so a fade on
either colour object runs on, and once the override ends the colour is
wherever the Color_Reference object has got to. `ObjectDatabase::lighting_color`
follows the link: it returns the `LightingColor` an output shows now, the
Tracking_Value of the referenced colour object (Override_Color_Reference's
while Color_Override is TRUE) as an `OutputColor::Xy` or `OutputColor::Kelvin`,
with the object it came from. It returns `None` for a reference to an object
the database doesn't hold: such a reference is stored and served but not
followed, since the clauses put the companion in the same device and an
object identifier can't name another one.

Fades, ramps and egress timers run on the server's monotonic task, which a
write that starts one wakes. Tracking_Value is worked out from the clock when
read. While it moves, the task samples it for COV each time it has moved by
`COV_Increment` (1.0 percent while that is 0.0), on a 100 ms grid shared by
every object, and once more when it arrives; Table 13-1 reports Present_Value
and Status_Flags, so a SubscribeCOV hears a fade once, when its level goes in,
and a SubscribeCOVProperty of Tracking_Value hears it move. When a live
Tracking_Value subscription (SubscribeCOVProperty, or a Multiple reference)
gives its own increment finer than that step, the finest such increment sets
the step instead (#1510), so that subscriber hears each of its own steps, at
most one per grid point; a coarser one changes nothing. The task looks the
increments up in the COV table on each pass, inside its database guard and in
the server's lock order.

Color and Color Temperature (Addendum 135-2020ca) hold their `Color_Command`
as a `BACnetColorCommand` (`bacnet_types::constructed`): a `ColorOperation`
plus an optional target colour (a `BACnetXyColor`), target colour temperature,
fade time, ramp rate and step increment (#1386). It reads operation NONE until
written. Over the network it travels as the command's context-tagged fields,
which `bacnet_encoding::constructed::encode_color_command` writes and
`decode_color_command_value` reads (`decode_color_command` reads one at an
offset); locally it reads as `PropertyValue::ApplicationData` holding those
octets, and `set_color_command` and `color_command` take and return the typed
value. `encode_xy_color` and `decode_xy_color` handle a Color object's xy
values. Each object checks a command against its own table of colour commands:

- A Color object takes FADE_TO_COLOR, which needs a target colour with both
  coordinates 0.0 to 1.0, and STOP.
- A Color Temperature object takes FADE_TO_CCT and RAMP_TO_CCT, which need a
  target colour temperature of 1000 to 30000 K, STEP_UP_CCT, STEP_DOWN_CCT and
  STOP.
- A field the operation uses must be in range: fade time 100 to 86,400,000 ms,
  ramp rate 1 to 30000 K/s, step increment 1 to 30000 K. A field it doesn't
  use is kept as written without a check.

Any other operation, NONE and those past STOP included, is refused with
VALUE_OUT_OF_RANGE, as is a missing target. Any other datatype, an OCTET STRING
included, is INVALID_DATA_TYPE, and octets that aren't exactly one command are
INVALID_DATA_ENCODING.

Both objects carry out each command they take (#1474), and `Color_Command`
keeps reporting it as written. There is no priority array, so at most one fade
or ramp runs, and Present_Value holds its target from the moment it starts:

- FADE_TO_COLOR, FADE_TO_CCT and RAMP_TO_CCT set Present_Value to the target
  and move Tracking_Value to it in a straight line from where it stood, over
  the fade time (or `Default_Fade_Time`) or at the ramp rate (or
  `Default_Ramp_Rate`), with In_Progress FADE_ACTIVE or RAMP_ACTIVE until it
  arrives. A Color object's fade moves x and y together, so the colour crosses
  the xy diagram in a straight line.
- STEP_UP_CCT and STEP_DOWN_CCT set Present_Value and Tracking_Value at once
  to Tracking_Value plus or minus the step increment (or
  `Default_Step_Increment`).
- A Color Temperature object clamps a target and a step result to
  `Min_Pres_Value` and `Max_Pres_Value`.
- STOP ends a fade or ramp, and Present_Value takes the value it reached;
  with none running it changes nothing. Any other command, or a Present_Value
  write, ends the one in progress too (Clauses 12.X.6.1 and 12.Y.6.1), and a
  new fade or ramp starts from where Tracking_Value stood.

In_Progress reads as an ENUMERATED `ColorOperationInProgress` and Transition
as an ENUMERATED `ColorTransition`, both in `bacnet_types::enums`.

Present_Value is writable on both (Tables 12-X and 12-Y code it W), and
`set_present_value` takes a value the same way. A Color object takes an xy
colour, a `PropertyValue::List` of two REALs as a WriteProperty decodes it,
with both coordinates 0.0 to 1.0. A Color Temperature object refuses a value
outside 1000 to 30000 K and clamps one inside to `Min_Pres_Value` and
`Max_Pres_Value` (Clause 12.Y.4), which `set_min_max` sets within that range;
a Present_Value outside new limits moves to the nearer one. The write halts a
fade or ramp in progress, then moves as `Transition` says: NONE (the default)
at once, FADE over `Default_Fade_Time`, or, on a Color Temperature object
only, RAMP at `Default_Ramp_Rate`.

Fades and ramps run on the server's monotonic task, as a Lighting Output's
do, and Tracking_Value is worked out from the clock when read. While it
moves, the task samples it for COV each time it has moved 0.001 along the xy
line (a Color object) or 10 K (a Color Temperature object), on the shared
100 ms grid, and once more when it arrives. Neither object has a
COV_Increment to change that step, but a Color Temperature Tracking_Value
subscriber's finer increment does, as on a Lighting Output (#1510). The server
compares an xy colour by any change rather than by an increment, so a Color
object's subscribers already hear every sample.

The rows follow the addendum's property tables. Present_Value and
`Color_Command` are W; `Default_Color`, `Default_Color_Temperature`,
`Default_Fade_Time`, `Default_Ramp_Rate` and `Default_Step_Increment` are R;
`Min_Pres_Value`, `Max_Pres_Value` and `Transition` are O. The defaults and
`Transition` are writable: `Default_Fade_Time` takes 100 to 86,400,000 ms,
`Default_Ramp_Rate` and `Default_Step_Increment` 1 to 30000, and
`Default_Color` any colour in range. `Default_Color_Temperature` is clamped as
Present_Value is, except that 0 is kept: Clause 12.Y.4 gives a zero default a
meaning of its own at restart. Neither table has `Status_Flags`,
`Event_State`, `Reliability` or `Out_Of_Service`, so neither object serves
them, and a whole-object COV report carries Present_Value alone.

Of the tables' optional rows, both objects serve `Audit_Level` and
`Auditable_Operations` once `set_audit_policy` provisions them before
registration, as on an Analog or Binary Value (#1525; see
[Object-owned Audit policy](#object-owned-audit-policy)). Neither table has
`Audit_Priority_Filter`, so a priority filter in the policy is left out.
`Tags`, `Profile_Location` and `Profile_Name` are served once `set_profile`
provisions them (#1553; see [Object profile rows](#object-profile-rows)).

`Value_Source` is served once `set_value_source_tracking(true)` turns tracking
on (#1552): it names the writer of the last Present_Value write, or of the last
Color_Command that set Present_Value, and only that writer may correct it (see
[Source of a noncommandable Present_Value](#source-of-a-noncommandable-present_value)).

Two choices here go past the addendum's text. A new object's
`Default_Fade_Time` is 100 ms, the shortest the range allows, as Lighting
Output's is: the addendum gives no initial value, and 0 lies outside its
range. Neither object models a restart, so `Default_Color` and
`Default_Color_Temperature` are only stored, and In_Progress never reads
NOT_CONTROLLED or OTHER.

The colour properties use their standard identifiers: `DEFAULT_COLOR` is
4194330, `DEFAULT_COLOR_TEMPERATURE` 4194331 and `COLOR_COMMAND` 4194334
(#887). Identifiers 508 to 511 are the Network Port properties
`ADDITIONAL_REFERENCE_PORTS`, `CERTIFICATE_SIGNING_REQUEST_FILE`,
`COMMAND_VALIDATION_RESULT` and `ISSUER_CERTIFICATE_FILES` (Addendum
135-2020cc); no object serves them yet.

A Channel passes each value written to its Present_Value on to its members
(Clause 12.53, #1151). Give it the members with `ChannelObject::set_members`,
each a `BACnetDeviceObjectPropertyReference` to an object in this device or
another, then optionally one delay in milliseconds per member with
`set_execution_delay`
and the control groups with `set_control_groups`. All three are writable
arrays on the network too. The member list and Execution_Delay always keep the
same size: a write of index 0 to either resizes both, as does a whole write of
the member list, while a whole write of Execution_Delay must give exactly one
delay per member (VALUE_OUT_OF_RANGE otherwise). A member naming the
server's own Device is stored as the local reference it stands for; one
naming another Device keeps it (#1264).

Present_Value takes any primitive value or one of the constructed
alternatives: a lighting command framed in context tag 0, or, as Addendum
135-2020ca adds, an xy colour in tag 1 or a colour command in tag 2 (#1474),
at priority 1 to 16 (Last_Priority reads 16 when the write
carried none). Write_Status then reads IN_PROGRESS, and any Present_Value
write is OBJECT / BUSY until the members are done. A running server writes
each member through the `write_local` path with the Channel as the initiating
object, at the priority the write carried, once that member's delay has passed;
every delay counts from the same start. The value is first converted to the
datatype of the member property's current value by the Table 12-63 rules (a
REAL 1.0 reaches a Binary Output as ACTIVE, a Multi-state Output as state 1).
A constructed value goes only to a member of its own datatype, and no
primitive goes to such a member: a lighting command to a `Lighting_Command`
and a colour command to a `Color_Command`, each as the command without its
framing, and an xy colour to a Color object's Present_Value (or
`Default_Color`) as its two REALs. `MemberDatatype::of` tells those members by
object type and property.
Readings of the rules: an Unsigned or ENUMERATED value above 2147483647
fails for INTEGER, REAL and Double members. A REAL or Double going to an
integer type keeps its integer part if it lies in 0 to 2147483000 (Unsigned,
ENUMERATED) or -2147483000 to 2147483000 (INTEGER; the upper bound Rules 5
and 6 print with a digit missing is read as 2147483000). A Double fits a REAL
up to `f32::MAX`. NaN and the infinities fail every conversion that has a
range, while rounding to a REAL's precision never fails.
A value that can't be converted, or a member that refuses the write, makes
Write_Status FAILED once every member has been tried; otherwise it reads
SUCCESSFUL. A NULL a member refuses as the wrong datatype (an Error of
INVALID_DATA_TYPE or a Reject of INVALID_PARAMETER_DATA_TYPE) isn't a
failure, so one Channel can relinquish commandable members alongside others.
With no members Write_Status stays IDLE, empty references (instance 4194303)
are skipped, and while Out_Of_Service is TRUE the value is kept but not passed
on.

A member in another device goes there as a confirmed WriteProperty, the way a
Command's remote action does (#1264): addressed from the device bindings,
with one targeted Who-Is and a wait of `cov_retry_timeout_ms` for the I-Am
when the device has no fresh binding (#1322, sent and limited as for a
Command), each attempt waiting `cov_retry_timeout_ms`, up to three
retries for silence, nothing sent while DeviceCommunicationControl restricts
initiation. A binding routed through the local network's own number is
written as a local device, with no DNET (#1358), as a Command's is.
Before the first write the server learns the property's datatype with a
ReadProperty there, sent as the distribution starts so it overlaps the
member's delay, and converts the value to it as for a local member (#1342):
a REAL 1.0 reaches a remote Binary Output as ACTIVE. A primitive datatype is
kept on the Channel until that member, or the whole member list, is written
again, or a write made with it is refused as a configuration fault (an invalid
datatype, an unknown property), so later distributions send no read until
then. No read is sent for a NULL, a constructed value, or a member whose
property fixes its datatype (`Lighting_Command`, `Color_Command`, a Color
object's xy colour). A read that gets no answer after its retries, or whose Who-Is finds
nothing, counts the device as silent, as a write would (every device executes
ReadProperty): the member fails as COMMUNICATION_FAILURE with no write sent. A
read that is refused, or returns NULL or a constructed value, keeps nothing,
and the value goes as written; the device then refuses a datatype it doesn't
take. Each member is written when its own delay is up (#1343): a remote
request that waits for its answer holds back no other member, though
Write_Status stays IN_PROGRESS (a Present_Value write, WriteGroup's included,
is refused BUSY) until every member has finished. Members in this device go
in delay order, list order among equal delays. The requests the server's runs
make in other devices, a Command's included, wait in two queues: each device
takes one at a time, as small and MS/TP devices often can only serve one, and
the server keeps at most 32 outstanding, an eighth of its 256 invoke IDs,
leaving the rest to confirmed notifications and Audit. A member due while its
device answers another request is written once that one ends. A device that
answers none of a request's attempts, or none of the Who-Is sent to find it,
counts as silent for the rest of that distribution: its members whose turn
comes after that fail at once with nothing sent, so a distribution waits out
one request's retries per silent device, while members in other devices and
local ones are still written. A run that `stop()` cuts short, or whose future
is dropped, during a remote request ends FAILED and frees its invoke ID.
Without a server, `tick_schedules` has no network, so a remote member fails
there.

Reliability reports how the last distribution ended (Clause 12.53.9):
NO_FAULT_DETECTED after a SUCCESSFUL one, otherwise the kind of the first
member failure to finish.
CONFIGURATION_ERROR means the value couldn't be converted to the member's
datatype, by datatype or by a coercion rule's range, or the member answered
UNKNOWN_OBJECT, UNKNOWN_PROPERTY, INVALID_ARRAY_INDEX,
PROPERTY_IS_NOT_AN_ARRAY, INVALID_DATA_TYPE, DATATYPE_NOT_SUPPORTED or a
Reject of INVALID_PARAMETER_DATA_TYPE. A value the member itself refuses as
out of range (VALUE_OUT_OF_RANGE) is PROCESS_ERROR: the clause leaves the
choice open, and here only the Channel's own conversion counts against its
configuration. COMMUNICATION_FAILURE means a remote member's device had no
fresh binding and no I-Am answered the Who-Is for it (or none was sent, under
the one-a-minute limit), DCC restricted initiation, or no attempt was
answered; an
attempt the transport failed to send waits like a silent one, so a send
failure on every attempt lands here too. PROCESS_ERROR covers any other
refusal (another Error code, another Reject reason, an Abort), a write the
server couldn't start (a value it can't encode, a request longer than one
APDU, no free invoke ID, a stopping server, no network), and a run cut short
before every member was tried. Reliability keeps its value
while a distribution runs, and Status_Flags shows FAULT whenever it isn't
NO_FAULT_DETECTED. While Out_Of_Service is TRUE it holds what it read and
takes a client's write of any Reliability value; back in service it shows the
last distribution's verdict again.

A running server also executes inbound WriteGroup on its Channels (Clause
15.11). For each change-list entry, every Channel whose Channel_Number is the
entry's channel and whose Control_Groups holds the request's group takes the
value as a Present_Value write, at the entry's own priority or else the
request's, and passes it on as above. Group membership is each Channel's own:
a Channel outside the group is left alone even when another Channel in the
device is in it. When the request sets Inhibit Delay, a Channel whose
Allow_Group_Delay_Inhibit is TRUE (`set_allow_group_delay_inhibit`, or a
network write) writes all its members at once; the others keep their delays.
A Channel that refuses its value, as a busy one does, doesn't stop the rest.
A channel number listed twice in one change list reaches the same Channels
twice: the later value is refused as busy while the distribution the earlier
one queued is still running, and taken once it has finished, so which value
stays depends on timing. Each Channel is checked again when its own write
runs: one that has left the group or changed its number by then is skipped,
and one whose Allow_Group_Delay_Inhibit is FALSE by then keeps its delays.
Nothing is answered and a malformed request is dropped. DCC's
DISABLE_INITIATION leaves WriteGroup running, as it only stops what the device
starts (the server refuses the deprecated DISABLE outright). Local mutation
policy decides each Channel write on its own (#1319): `MutationPolicy::DenyAll`
denies it, and an installed `mutation_authorizer` is called once per Channel
with a `MutationTarget::WriteGroup` (the Channel, group, channel number,
priority, value and inhibit flag), no invoke ID and
`MutationService::Unconfirmed(WRITE_GROUP)`, so it can allow some Channels and
not others. A denied write is skipped with nothing answered and counted in
`mutation_decision_counters().write_group`. Each Channel written makes one
WRITE Audit record (Table 19-5) of its Present_Value at the priority used,
naming the requester (its bound Device, when the audit profile knows one) and
no invoke ID; a denied one makes none (#1318). A Channel's Present_Value is
commandable (Clause 12.53.5), so its records, a WriteProperty's as well as a
WriteGroup's, carry the priority, and Audit_Priority_Filter applies to them
(Clause 19.6.3): one at a priority the filter disables is dropped. The
endpoint responder ignores WriteGroup.

Channel runs are owned as Command runs are (#1178). A `write_local` dropped
after the Channel took its value leaves the distribution running, as it does
a Command's run (#1367). Without a server,
`tick_schedules` runs a distribution its Schedule writes start before it
returns, delays included, and ends it FAILED if its future is dropped first.
The bare `handle_write_property` and `handle_write_property_multiple` handlers
end it at once as FAILED, without writing the members. The endpoint responder
refuses a Channel's Present_Value write with WRITE_ACCESS_DENIED.

A run that one Command's or Channel's write starts in another carries the
objects above it. If it would start an object already in that chain, as two
Channels naming each other would after their delays, or would have more than
eight runs above it, it isn't started: it ends as failed at once and the write
that started it fails with OBJECT / BUSY, so such a loop stops after one round.

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
| `AccessRightsObject` | `::new(instance, name)`, `::with_persistence(instance, name, persistence)` |
| `AccessZoneObject` | `::new(instance, name)` |
| `CredentialDataInputObject` | `::new(instance, name)` |

An `AccessRightsObject` built with `with_persistence` keeps the
`Positive_Access_Rules`, `Negative_Access_Rules`, Enable and Accompaniment
that peers write across a restart (#1392, #1393). Clause 12.34 doesn't ask
for this; the stack does it because head ends provision access rights over
the network. The storage is an application-owned `AccessRightsPersistence`
that loads and saves an `AccessRightsSnapshot`, whose four members stay
`None` until a write sets them. `FileAccessRightsPersistence` keeps it in one
file, replaced whole the same way as the Notification Class's. The file is
tagged `RBNACR01` and holds the object identifier, then the BACnet encodings
of each member a write has set, in order: the positive rules between opening
and closing context tag 0, the negative rules between tag 1, Enable as a
BOOLEAN with context tag 2, and Accompaniment's reference between opening and
closing context tag 3. Loading refuses a file past 128 KiB, an array of more
than 1024 rules, members out of order, trailing octets, and another object's
file. `with_persistence` then puts each saved array and Accompaniment
through the setters' checks, so a file holding a value they refuse fails it.
A saved Accompaniment serves the row whether or not the application sets one,
and `set_accompaniment(None)` doesn't remove it. To lift a saved requirement,
write the no-accompaniment reference (instance 4194303), which keeps the row;
to drop the row itself, remove the storage file, which also drops the saved
rules and Enable.
An object built with `new` keeps written values in memory only.

Saves follow the Notification Class's rules (see
[Schedule & Notification](#schedule--notification-6)): the save runs on the
object's own writer thread, and the bundled server stages each WriteProperty,
WritePropertyMultiple or `write_local` write of the arrays (whole, one
element, or the size at index 0), of Enable or of Accompaniment, and waits
for its save with the database guard dropped. A WritePropertyMultiple's
several such writes to one object stage one save of the state they leave
together (#1423): each write takes its own step as the request makes it, and
a request that stops part way puts storage back to what the object serves. A
state that cannot be saved is refused with DEVICE /
OPERATIONAL_PROBLEM, and nothing changes. A staged write that is never made
puts storage back to the served state on release, after its lifetime, at
`stop()`, or when the object drops. A saved value wins over the
configuration: once a write has set a property and it was saved,
`property_saved(property)` is true, and that property's setter
(`set_positive_access_rules`, `set_negative_access_rules`, `set_enable` or
`set_accompaniment`) checks its argument without storing it. Configuration alone is never saved,
but a write saves the whole array it leaves: an element or index-0 write to
an array no write has set yet saves the configured rules it didn't touch too.
`wait_for_saves()` blocks until queued saves have run.

Access Door, Access Point and Credential Data Input support COV (Table 13-1).
A door's SubscribeCOV report carries Present_Value, Status_Flags and
Door_Alarm_State; a Door_Alarm_State change sends one. An Access Point has no
Present_Value, so its report starts with Access_Event, then Status_Flags,
Access_Event_Tag, Access_Event_Time, Access_Event_Credential and
Access_Event_Authentication_Factor, and only an Access_Event_Time or
Status_Flags change sends one. A Credential Data Input report carries
Update_Time, whose change sends one. The application sets these values
before adding the object with `AccessDoorObject::set_door_alarm_state`,
`AccessPointObject::set_access_event` (an `AccessEventReport`: the event,
its tag, a `BACnetTimeStamp` or `None` for the Device clock's time, an
Access Credential reference or `None` for the no-credential reference,
instance 4194303, and a `BACnetAuthenticationFactor` or `None` for the
UNDEFINED one; another object type, 4194303 in only one of the object and
device instances, or a factor format outside the closed production is
VALUE_OUT_OF_RANGE) and `CredentialDataInputObject::set_present_value` (the
factor read and its Update_Time), and a door's Door_Status and Lock_Status
with `set_door_status` and `set_lock_status`.

Once the server holds them, the application reports these inputs through
`BACnetServer::report_access_event_local`,
`report_credential_read_local` (a `CredentialReadReport`: the factor and an
optional time, stamped from the Device clock when absent) and
`report_door_state_local` (a `DoorStateReport`: any of Door_Status,
Lock_Status and Door_Alarm_State) (#1132). Each record changes its values
together as one local write, so the COV report and the event pass follow it
as they follow `write_local`, and the call must run inside a Tokio runtime.
Each checks its values as the setters do, all or nothing, a door's values
against their productions and a point's event against the BACnetAccessEvent
production (named, or proprietary from 512 to 65535) too. While its
Out_Of_Service is TRUE each object keeps its own rule: the point refuses an
event with WRITE_ACCESS_DENIED and changes nothing, since it performs no
authentication then, while the door and the reader keep the reported values
aside in place of the device's earlier ones, as their setters do, so a
client's simulated values stay served with no COV report or event, and the
return to service serves the latest values reported. Any other object fails
with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.

An Access Point's Authentication_Status is READY until the application
reports another status with `set_authentication_status` (a value past
IN_PROGRESS is VALUE_OUT_OF_RANGE); it reads DISABLED while Out_Of_Service is
TRUE and the reported status again afterwards.

The point also serves Active_Authentication_Policy,
Number_Of_Authentication_Policies, Authorization_Mode and
Priority_For_Writing. `set_number_of_authentication_policies` sets how many
policies there are (1 by default; zero is VALUE_OUT_OF_RANGE), and a client
picks the policy in effect by writing Active_Authentication_Policy, an
Unsigned from 1 to that count. Until the application describes the policies,
what each holds is up to it. `set_authentication_policies` takes
`(name, BACnetAuthenticationPolicy)` pairs (#1325): the point then serves
Authentication_Policy_List and Authentication_Policy_Names, both read-only
over the network, the count becomes the number of pairs (no pairs is
VALUE_OUT_OF_RANGE), and a later count resizes both arrays, adding empty
policies with empty names. While the arrays are served the count is capped
at `MAX_AUTHENTICATION_POLICIES` (256): a longer list, or a larger count, is
VALUE_OUT_OF_RANGE. A policy is usable when it has at least one entry, each
entry names a Credential Data Input, and the indexes, in list order, start
at 1 and repeat or climb by one; a client's write naming any other policy
is VALUE_OUT_OF_RANGE. When the count drops below the policy in effect, or
the list makes it unusable, Active_Authentication_Policy becomes 0 until a
client writes a usable policy. Reliability is CONFIGURATION_ERROR while the
active policy is 0 or the list holds any invalid policy, a grown count's
empty ones included, and NO_FAULT_DETECTED otherwise. While it isn't
NO_FAULT_DETECTED the point generates no access events, so
`report_access_event_local` refuses them with WRITE_ACCESS_DENIED. The
point's Reliability takes simulated writes while Out_Of_Service is TRUE and
refuses them in service; out of service it ignores the policies, and the
return to service serves the derived value again.
Authorization_Mode starts at AUTHORIZE and takes a write of any mode in the
set `set_supported_authorization_modes` gives. The point enforces no mode
itself, so a new point supports AUTHORIZE alone: an application that acts on
the mode declares the other standard modes it carries out, and proprietary
ones from 64 to 65535, in a set that keeps AUTHORIZE and the mode in effect.
Any other value is VALUE_OUT_OF_RANGE, and another datatype
INVALID_DATA_TYPE.
`set_priority_for_writing` sets the priority the application commands the
Access_Doors at (16 by default, 1 to 16 accepted). The policy count and the
priority are read-only over the network; the point stores and checks these
values, and carrying them out is the application's work.

An Access Zone counts
occupancy: Occupancy_State reads DISABLED while counting is off
(`set_occupancy_count_enable(false)`, which also zeroes the count and
Adjust_Value), and otherwise compares Occupancy_Count with the limits
`set_occupancy_limits(lower, upper)` sets (zero is no limit; a nonzero upper
limit at or below the lower one is VALUE_OUT_OF_RANGE). Adjust_Value is the
one writable counting row: an Integer written in service is added to the
count (stopping at zero, and zero clears it), while out of service it is
kept without moving the count.

An Access Zone reports intrinsically on Occupancy_State with the
CHANGE_OF_STATE algorithm (Clause 12.32). It serves the event rows the
Multi-state Input does: Time_Delay, Notification_Class, Alarm_Values,
Event_Enable, Acked_Transitions, Notify_Type, Event_Time_Stamps,
Event_Message_Texts, Event_Detection_Enable and Time_Delay_Normal, the
configuration writable over the network, and the message texts and inhibit
rows every intrinsic reporter serves (#1329). Alarm_Values is a list of
BACnetAccessZoneOccupancyState values other than NORMAL (named, or
proprietary from 64 to 65535; anything else, NORMAL included, is
VALUE_OUT_OF_RANGE naming the element), which
`AccessZoneObject::set_alarm_values` sets too. NORMAL is refused because it is
the state with no limit crossed: as an alarm value it would put the zone in
alarm whenever its count sat inside its limits. Event_State goes OFFNORMAL
once Occupancy_State has stayed in Alarm_Values for Time_Delay seconds,
whether the count moves through `set_occupancy_count`, an Adjust_Value write
or a count simulated out of service, and back to NORMAL once it has stayed
out of them for Time_Delay_Normal; a Reliability other than
NO_FAULT_DETECTED is FAULT. The server sends each transition to the
recipients of the zone's Notification Class: a CHANGE_OF_STATE notification
carries Occupancy_State as its `zone-occupancy-state` New_State, and a
CHANGE_OF_RELIABILITY one lists Occupancy_State (Table 13-5). A count the
application sets directly on the object is picked up by the one-second
tick.

An Access Door reports intrinsically on Door_Alarm_State with the
CHANGE_OF_STATE algorithm (Clause 12.26), serving the same event rows as the
zone plus Fault_Values and Masked_Alarm_Values, all three lists of
BACnetDoorAlarmState values other than NORMAL (named, or proprietary from 256
to 65535) that clients can write and `set_alarm_values`, `set_fault_values`
and `set_masked_alarm_values` set. Door_Alarm_State is NORMAL or a member of
Alarm_Values or Fault_Values, never a masked state:
`AccessDoorObject::set_door_alarm_state` (now returning `Result`) and a
simulated write refuse any other state with VALUE_OUT_OF_RANGE, and a list
change that leaves the current state outside them, masking it included,
returns the door to NORMAL at once. An alarm value makes Event_State OFFNORMAL
after Time_Delay; a fault value makes Reliability MULTI_STATE_FAULT (the
FAULT_STATE algorithm) and Event_State FAULT. The server sends the transitions
to the door's Notification Class: a CHANGE_OF_STATE notification carries
Door_Alarm_State as its `door-alarm-state` New_State, and a
CHANGE_OF_RELIABILITY one lists Door_Alarm_State, then Present_Value (Table
13-5). The application decides when the door is in alarm, DOOR_OPEN_TOO_LONG
included: the door serves Door_Open_Too_Long_Time but runs no timer, and a
state the application sets directly on the object reaches the algorithm at the
one-second tick.

Over the network the Access Point event values stay read-only, but writing
its Out_Of_Service records an event on each edge (Clause 12.31.8):
OUT_OF_SERVICE on entry and OUT_OF_SERVICE_RELINQUISHED on the return, each
a new transaction (Access_Event_Tag moves on by one, wrapping) whose
Access_Event_Time comes from the Device clock and whose
Access_Event_Credential is the no-credential reference, so each edge sends
the COV report. A write that leaves Out_Of_Service as it was records
nothing. An
Access Zone's Occupancy_Count and Reliability, the rows footnote 1 of Table
12-37 marks, take WriteProperty and WritePropertyMultiple while
Out_Of_Service is TRUE and refuse them in service with WRITE_ACCESS_DENIED: a
count must be an Unsigned and a Reliability a BACnetReliability value.
`AccessZoneObject::set_occupancy_count` reports the zone's own count;
entering out of service puts the count and Reliability aside, the count the
application reports meanwhile replaces the one put aside,
`set_reliability_internal` is refused, and the return to service serves the
zone's values again. A Credential Data
Input's Present_Value and Reliability, the rows footnote 1 of Table 12-43
marks, take WriteProperty and WritePropertyMultiple while Out_Of_Service is
TRUE and refuse them in service with WRITE_ACCESS_DENIED. A Present_Value
write must be one `BACnetAuthenticationFactor` (else INVALID_DATA_TYPE) whose
format type and class match a declared Supported_Formats element and its
class, or the UNDEFINED or ERROR factor with class 0 (else
VALUE_OUT_OF_RANGE); it stamps Update_Time from the Device clock. A
Reliability write must be a BACnetReliability value. Entering out of service
puts the reader's Present_Value, Update_Time and Reliability aside,
`set_present_value` updates the values put aside, `set_reliability_internal`
is refused until the return to service, and the return to service serves the
reader's values again.

Without a usable Device clock a date-and-time stamp would be the unspecified
one every time, so Access_Event_Time and Update_Time, the Table 13-1
triggers of these two objects, would never move. Both are BACnetTimeStamp
values (Clause 21.6), and Clauses 12.31.29 and 12.36.11 allow an update time
in the sequence-number form, so with no usable clock the objects stamp that
form instead. An Access Point stamps each event it records without a
given time, an Out_Of_Service edge or a reported event, strictly after the
time it served before, since several events of one transaction share a tag
and each has to move the time: the next sequence number without a clock
(from 1 when the time served is in another form, wrapping from 65535 to 1),
and with one the clock's time, or one hundredth past the time served when
the clock isn't later. A Credential Data Input's simulated Present_Value and
format reset take the object's own next number, from 1 to 65535 and then 1
again. Neither stamps 0, the value of an update time with no update yet. A
time the application passes to `set_access_event` or `set_present_value` is
served as given.

A door's Door_Status, Lock_Status and Door_Alarm_State, the rows footnote 1 of
Table 12-30 marks, and its Reliability take WriteProperty and
WritePropertyMultiple while Out_Of_Service is TRUE, so a client can simulate
the door; in service they refuse writes with WRITE_ACCESS_DENIED. A write must
be an Enumerated in the property's production: a named BACnetDoorStatus or one
from 1024 to 65535, a named BACnetLockStatus (no proprietary range), a named
BACnetDoorAlarmState or one from 256 to 65535 that the door's alarm lists
admit, or a BACnetReliability value, its proprietary range included. A
simulated Reliability overrides the fault check until the return to service.
Entering out of service puts the door's own three values aside, a value the
application sets meanwhile replaces the one put aside, and the return to
service serves them again, dropping the simulation. A simulated
Door_Alarm_State sends the COV report as a real change does. The pulse relock
runs on its timer whatever the simulated values say.

A door's Secured_Status isn't stored: each read works it out from what the
door serves (Clause 12.26.14). It reads SECURED while the door is commanded
LOCK, isn't IN_ALARM, masks no alarm state, and its Door_Status and
Lock_Status show it shut and locked (or UNUSED). Any other input makes it
UNSECURED, so an UNLOCK or a pulse reads UNSECURED until it ends. A
Door_Status or Lock_Status of UNKNOWN or a fault makes it UNKNOWN, unless
another input has already made it UNSECURED. Simulated values count the same
as the device's.

##### Evaluating access rights

`bacnet_objects::access_control::evaluate_access_rights(db, credential,
point)` (#1331) checks an Access Credential against the Access Rights it is
assigned (Clause 12.34.9.2), for the Access Point where the credential was
presented. It is pure: it borrows the `ObjectDatabase`, reads each property as
a peer would, takes no lock and writes nothing, Access_Event included. A
server application calls it under `server.database().read().await` and acts
on the result after dropping the guard. It fails with OBJECT / UNKNOWN_OBJECT
when `credential` doesn't name an Access Credential in the database, or
`point` an Access Point.

The result, an `AccessRightsEvaluation`, holds a `decision` and an
`unresolved` list. The `AccessRightsDecision` is one of:

- `Exempt`: the credential's Authorization_Exemptions lists ACCESS_RIGHTS, so
  no rule is read.
- `Granted { rule }`: the first positive rule that held. An
  `AccessRulePosition` names the rule's Access Rights object, its array
  (`AccessRuleKind`) and its one-based index.
- `Denied { access_event, rule }`: the Access_Event value the failure
  carries, and the rule behind it when there is one.

Every enabled negative rule of every assigned object is tried before any
positive rule. A negative rule that holds denies with
DENIED_POINT_NO_ACCESS_RIGHTS when its location is the point, and with
DENIED_ZONE_NO_ACCESS_RIGHTS when it is a zone. One whose location is ALL
denies with DENIED_POINT_NO_ACCESS_RIGHTS too, since it bars this point; the
clause leaves that case open. When no positive rule holds, the denial is
DENIED_OUT_OF_TIME_RANGE if some enabled positive rule covered the point and
a value read here for its time range was FALSE (the first such rule is
named), and DENIED_NO_ACCESS_RIGHTS otherwise. `allows()`, `access_event()`
and `rule()` read a decision.

Skipped without a trace: Assigned_Access_Rights elements whose enable flag is
FALSE, the unused marker (instance 4194303), Access Rights objects whose
Enable is FALSE, and rules whose enable flag is FALSE. An enabled element
naming a missing object or another object type, or an object whose rules
don't read as rules (only an application's own object can do that), gives no
rules, and Clause 12.35.18 has the device ignore the first two. One naming
another device or the wildcard Device gives none either: the evaluator reads
nothing remotely, a policy of its own. The decision ignores all of them, and
`unresolved` lists each with its index and an `UnresolvedReason` (`Remote`,
`WildcardDevice`, `NotAccessRights`, `Missing` or `Unreadable`), so the
application can choose to deny.

A rule holds when it is enabled, its location covers the point and its time
range is TRUE:

- **Time range.** ALWAYS is TRUE. SPECIFIED reads the referenced property
  here, with the reference's array index. A BOOLEAN reads as itself, an
  Unsigned as TRUE when nonzero, and an INTEGER as TRUE above zero. An
  Enumerated reads by the property's enumeration
  (`bacnet_types::enums::ResolvedEnum::from_property`): for a BACnetBinaryPV,
  or any property whose type isn't known to be another enumeration (the
  Present_Value of any object among them), ACTIVE is TRUE and any other
  value FALSE. Those FALSE values make a rule out of its time range. A time
  range that can't read TRUE at any moment is FALSE at every moment and never
  makes a denial DENIED_OUT_OF_TIME_RANGE: a type that never reads TRUE
  under these rules (REAL, Double, strings, an enumeration known to be
  another one such as Event_State or Reliability, a whole array read without
  an index; a local choice the clause allows), an unknown specifier,
  SPECIFIED without a reference, an unspecified reference or one naming
  another device or the wildcard Device, a missing object or property, a
  failed read, NULL, and index 0, since an array's size is no time-range
  value.
- **Location.** ALL covers every point. An Access Point covers itself, and an
  Access Zone the points its Entry_Points names. Anything else covers
  nothing.

A reference names this device when it has no Device member or names the
database's own Device (`LocalDevice::is_local`), as elsewhere in the stack.
The wildcard Device instance 4194303 names no device in particular, so a
reference carrying it is never read, and neither is one naming another
device. Such a reference never grants access. A negative rule whose location
or time range names one doesn't hold, so it bars no one: Clause 12.34.9.1
has a reference that is unspecified or can't be retrieved evaluate to FALSE.
An application that would rather deny can act on the `unresolved` list. The
evaluator doesn't judge accompaniment, the credential's status or
validity window, or the other authorization checks; the documentation of
`evaluate_access_rights` lists them.

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

An Accumulator serves the optional Prescale only once `set_prescale` gives
it one: BACnetPrescale has no NULL, so until then the property is absent from
Property_List and the PICS, and a read is UNKNOWN_PROPERTY (#1417). Scale
(`set_scale`) and Prescale read as a `PropertyValue::ApplicationData` holding
their context-tagged Clause 21 forms, encoded with
`bacnet_encoding::constructed::{encode_scale, encode_prescale}`: Scale's
float `[0]` or integer `[1]` alternative, and Prescale's multiplier `[0]` and
modulo divide `[1]` (#1487). Both are read-only.

A Pulse Converter's Input_Reference, set with `set_input_reference`, reads and
takes writes like the Loop's variable references: the context-tagged
`BACnetObjectPropertyReference` in a `PropertyValue::ApplicationData`, with
the flat list refused as INVALID_DATA_TYPE (#1312). While unset it reads as
Accumulator 4194303's Present_Value, and writing a reference to instance
4194303 unsets it (#1417).

The database judges the reference (`ObjectDatabase::check_input_reference`,
#1341): Reliability reads CONFIGURATION_ERROR, with Status_Flags FAULT, while
it names a missing object or a property that doesn't read as an Unsigned or
INTEGER, and NO_FAULT_DETECTED once it names one that does or is unset
(Clause 12.23.9). An index on a property that isn't an array, index 0 (an
array's size) and the converter's own properties are faults too. A
Priority_Array slot is judged by the datatype the object is commanded in,
its Relinquish_Default's, so a slot that is NULL for now is no fault but has
nothing to count. The verdict is taken as a write of the reference commits
(WriteProperty, WritePropertyMultiple or `write_local`; the object isn't one
CreateObject builds), when the converter is added, whenever
`ObjectDatabase::add` or `remove` adds, replaces or removes the object it
names, and on every counting pass; the server fans COV out for a converter
that changes. While Out_Of_Service is TRUE, Reliability takes a client's
value (Clause 12.23.10), and the return to service applies the latest
verdict. A running server also counts from the property at least once a
second (`ObjectDatabase::count_pulse_inputs`, on every wake of its monotonic
operation task): each increase over the last reading goes into Count as
`add_pulses` would, and the first reading after the reference is set or
changed only sets the baseline, so re-pointing it at a larger value counts
nothing. A reading below the last one counts the wrap when the source is an
Accumulator's Present_Value, modulo its Max_Pres_Value + 1 (Clause 12.61.4);
from any other source it only sets the baseline again. The application can
still feed Count with `add_pulses`.

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

All 12 are commandable. Each serves `Current_Command_Priority`, the
Priority_Array slot Present_Value comes from, or NULL while
Relinquish_Default is in effect. Integer, Positive Integer and Large Analog
Value also serve `Units` (set with `set_units`) and a writable
`COV_Increment`, Unsigned on the two integer types and Double on Large Analog
Value, set over the network or with `set_cov_increment`. It starts at 0, so
every Present_Value change sends a SubscribeCOV notification; a larger
increment holds notifications back until Present_Value has moved that far from
the value last reported (Table 13-1). Large Analog Value refuses a negative or
non-finite increment with VALUE_OUT_OF_RANGE.

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
    .device_uuid(client_uuid) // Already provisioned and durably stored by the caller.
    .build()
    .await?;
```

Use `bip_builder()` for B/IP, `sc_builder()` for BACnet/SC, or
`generic_builder()` with a prebuilt transport.

### Transport access and BBMD helpers

`client.transport()` borrows the transport a built client owns, whichever
builder made it. The transport lives in the client's network layer for the
client's whole life, so this is a shared borrow: `&mut self` methods such as
`start`, `stop` and the setters stay out of reach, and `client.stop()` stops
the transport in place. What you take from it can outlive the borrow, which
suits a long-lived UI:

```rust
// BACnet/SC (sc_builder): an owned watch receiver, and a drop-count snapshot.
let mut state = sc_client.transport().connection_state_changes();
tokio::spawn(async move {
    while state.changed().await.is_ok() {
        let connected = *state.borrow_and_update() == ScConnectionState::Connected;
        // update the status bar
    }
});
let drops = sc_client.transport().npdu_drop_counts();

// B/IP (bip_builder): counter snapshots to poll, and the BBMD state if any.
let outgoing = bip_client.transport().bvlc_client_snapshot();
let management = bip_client.transport().management_counters();
let fanout = bip_client.transport().fanout_counters();
let fdt = bip_client.transport().fdt_counters(); // None unless a BBMD
let bbmd = bip_client.transport().bbmd_state().cloned(); // Option<Arc<std::sync::Mutex<BbmdState>>>

// MS/TP: the same counts-only handle you can take before handing the transport over.
let diagnostics = mstp_client.transport().diagnostics();
```

Reading state and diagnostics is what the borrow supports. Two things go around
the client and are not supported while it runs: sending through the transport
(`send_unicast`, `send_broadcast`), which the client's transaction state
machine never sees, so a hand-built confirmed request can reuse an in-flight
invoke ID; and holding the `bbmd_state()` lock across an await. That lock is
a synchronous `std::sync::Mutex` the receive loop and the client's broadcasts
take for short critical sections, so holding it stalls the transport or
deadlocks. Lock, copy and release. The live MS/TP master node and SC
connection are not public.

The management, FDT and fanout counters count what this transport does as a
BBMD (ACKs sent, registrations admitted, broadcasts forwarded). A client from
`bip_builder()` is never a BBMD, so its counters stay at zero; a client built
with `generic_builder()` over a BBMD-mode `BipTransport` reports live values.

`bvlc_client_snapshot()` instead describes this transport's own outgoing
Read-BDT, Write-BDT, Read-FDT, Delete-FDT and Register-FD exchanges, including
automatic registration. Each function reports successful local UDP sends,
valid matched ACKs, matched result counts and the latest result code, response
timeouts, local errors, malformed replies, and calls refused because the
single management slot was busy. Sending a datagram is not a BBMD receipt.
The snapshot uses fixed-size counters and a short synchronous lock; polling it
does not send traffic. Totals belong to this transport instance and survive
stop/start; last-result fields clear. BBMD-side counters keep their existing meanings.

Response matching checks the peer address and the operation. Read requests
accept their typed ACK or their own NAK; requests answered by a Result accept
success or that operation's NAK. Unknown codes and unrelated results remain
visible as unclassified observations and do not complete a request. Malformed
Results leave it pending. A malformed typed ACK returns the payload decode
error and does not count as a successful ACK. BVLC has no transaction ID, so
a delayed same-peer, same-kind response can still be confused with a later
attempt. These counters describe local correlations, not certain attribution
after a timeout. Encoding/send errors, cancellation and stop release the
pending slot without inventing a timeout.

The Rust B/IP client builder can configure automatic foreign-device mode:

```rust
use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::ForeignDeviceConfig;
use std::{net::Ipv4Addr, time::Duration};

let client = BACnetClient::bip_builder()
    .port(0)
    .foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::new(192, 0, 2, 10),
        bbmd_port: 47808,
        ttl: 60,
        renewal_interval: Some(Duration::from_secs(20)),
    })
    .build().await?;
let registration = client.transport().bvlc_client_snapshot().foreign_registration;
```

Add `renewal_interval: None` to existing `ForeignDeviceConfig` literals to use
the default of half the advertised TTL, including 500 ms for TTL 1. Automatic
mode requires a positive TTL and a positive interval shorter than that TTL;
invalid settings fail before transport I/O. Positive intervals below 100 ms
use 100 ms instead, limiting automatic attempts to ten per second even when
a manual request owns the slot or local sends fail immediately. This is a
local pacing policy; the requested TTL stays unchanged on the wire. The
effective interval also drives `next_attempt_in` and response waiting.
The automatic worker waits at most the smaller of the effective interval
and three seconds for each result, then schedules another attempt; busy manual
requests cause a locally counted busy refusal and a retry on the next interval.
There is no catch-up burst after a delayed attempt. One-shot
`register_foreign_device_bvlc` calls retain their three-second response wait
and permit zero-TTL requests; they do not switch broadcast mode.

`build()` means the client is running locally. Poll `last_outcome` for a
matched acceptance or refusal; a send failure or timeout leaves the remote
outcome unknown. A registration NAK rejects that attempt and does not prove
an earlier accepted entry disappeared. The `next_attempt_in` countdown is the
worker's scheduled attempt, not a remote lease expiry. Annex J.5.2's BBMD
expiry includes its grace period; the earlier local renewal is a scheduling
choice. Last-attempt status and countdown clear on stop/restart while totals
remain. Foreign-mode broadcasts use DBTN toward the configured BBMD even
before acceptance or after a rejected attempt.

The BBMD helpers (`read_bdt`, `write_bdt`, `read_fdt`, `delete_fdt_entry`,
`register_foreign_device_bvlc`) work on any client whose transport implements
`AsBip`: a client over `BipTransport`, or one over `AnyTransport` whose variant
is `Bip`. On any other variant they return `Error::UnsupportedTransport`
before sending anything:

```rust
use bacnet_transport::bip::AsBip;

let client = BACnetClient::generic_builder()
    .transport(AnyTransport::<NoSerial>::from(BipTransport::new(ip, 0, broadcast)))
    .build()
    .await?;
let bdt = client.read_bdt(&bbmd_mac).await?;
let counters = client.transport().as_bip()?.bvlc_client_snapshot();
```

### Routed Confirmed-Request Limits

Routed confirmed requests size each outgoing APDU to the smallest applicable
peer, local-transport, and routed-path allowance before registering a
transaction or emitting a frame. The local allowance retains the transport's
live maximum and the current routed destination-header cost. The routed-path
allowance is an NPDU limit: an unknown path starts from a conservative
228-octet NPDU envelope, then subtracts the forwarded header containing both
the destination address and the client's actual local source MAC. For example,
six-octet destination and source addresses leave 207 APDU octets.

Applications with path-specific evidence can configure the NPDU envelope
without changing `ClientConfig`:

```rust
client
    .configure_routed_path_max_npdu(&router_mac, dnet, 1497)
    .await?;

// Restore the conservative unknown-path policy.
client.clear_routed_path_limit(&router_mac, dnet).await?;
```

Every MAC on a routed path holds to `NpduAddress::MAX_MAC_LEN` (18 octets), the
longest address the NPDU codec carries and the network layer delivers (#1267).
A routed request refuses a longer DADR or local source MAC, and a router MAC
that is empty or longer, with `Error::Encoding` before it reserves or waits on
the path, and both configuration methods refuse such a router MAC too.

A routed confirmed request goes to one device on one remote network (#1278).
DNET must be in 1..=65534, since 0 names no network and 65535 is the global
broadcast, and the DADR must hold at least one octet, since an empty DADR has
the remote router broadcast the request. `confirmed_request_routed` and every
routed confirmed method built on it (`read_property_routed`, the
`_from_device` and `_to_device` methods, the routed COV subscriptions) refuse
any of them with `Error::Encoding` before the path is reserved or a
transaction registered. Both configuration methods use the same DNET check,
`add_routed_device` refuses such a peer, and the endpoint requester refuses
such a routed destination. Unconfirmed sends keep remote and global broadcasts,
for example `broadcast_network_unconfirmed`.

A routed destination whose DNET is the client's own network number, once the
client has learned it from Network-Number-Is (see
[Local Network Number controls](#local-network-number-controls)), is on the
client's own network (#1358). A routed confirmed request to it, from any of
the methods above or for a device added with `add_routed_device`, passes the
checks above and then goes as a local request: a unicast to the DADR with no
DNET, not through the router, so a non-routing peer there takes it. It holds
no routed-path state, and `router_mac` is not used. An answer from the DADR
with no SNET completes it, and so does one a router relays back with that
number as its SNET and the DADR as its SADR (#1465): network numbers are
unique, so both name the same station. That holds for any request to a station
on this network. A relayed answer still has to carry the request's invoke ID,
one naming another network or another station completes nothing, and while
the number is unknown only the direct answer counts. The link source of a
relayed answer is not checked, so any node on this link could complete the
request by claiming that SNET and SADR with the right invoke ID; a routed
request already trusts a claimed SNET/SADR the same way. A request routed to
this network before the number was learned keeps its routed key, and its
relayed answer still completes it. The peer's
limits still come from its routed device-table row, so a request past them is
refused or segmented as for the routed peer.
`broadcast_network_unconfirmed`, and `who_is_network` and a `write_group` to
`WriteGroupDestination::RemoteBroadcast` built on it, send a broadcast for that
number as a local broadcast. While the number is unknown, every destination goes as written.
Replies, SegmentACKs and Aborts to a routed peer keep the route its PDU
arrived by.

The shared endpoint's requester does the same once its session knows the
number (#1403). A `Routed` or `RoutedViaLocalBroadcast` destination naming it
passes the same checks, then goes to the DADR with no DNET. The DADR's own
answer completes the read, and so does one relayed back with that number as
its SNET and the DADR as its SADR (#1465). The client, the endpoint and the
server's own confirmed requests match answers through one rule,
`CanonicalPeer::from_source`.

State is keyed by the immediate router MAC together with DNET. One confirmed
request at a time owns that path; requests through a different router or to a
different DNET remain independent, and direct requests bypass this state. A
matching Reject-Message-To-Network reason 4 completes only the active owner as
`Error::RoutedPathTooLong { dnet }` and records the attempted NPDU length as an
exclusive upper bound. Learned negative evidence lasts for the client lifetime
and has no widening TTL. Both configuration methods wait for an active owner;
configuring replaces the prior value and deliberately resets learned evidence,
while clearing removes configured and learned evidence. Active Clause 19.4
path probing and cache persistence across process restarts are not provided.

The client retains at most 256 routed-path entries. At capacity it
deterministically reclaims the least-recently-used entry only when it has no
configured or learned evidence and its gate has neither an owner nor waiters.
Configured and learned safety evidence is never silently evicted. If no entry
is safely reclaimable, the operation returns
`Error::RoutedPathCapacityExceeded { capacity: 256 }` before TSM registration
or frame emission.

An ambiguously terminated send (including cancellation, timeout, or send
failure) quarantines its path for the configured APDU timeout multiplied by
the configured attempt count. The same-path gate remains exclusive during
that interval, and network controls already observed at ingress before the
next generation activates are discarded using a monotonic ingress sequence.
A source-correlated terminal response after one attempted frame can end the
generation without quarantine; multi-frame or retried generations remain
conservative.

### Property Access

For Tags values returned by RP or successful RPM rows, pass the echoed array
index and raw value bytes to `bacnet_client::tags::decode_tags_read`. Its
`TagsRead` result distinguishes `Whole(Vec<BACnetNameValue>)`, `Element` and
`Size(u32)`. Whole reads preserve order and accept an empty array; an indexed
read requires exactly one element, and index zero requires one Unsigned count.
Invalid or trailing data returns an error without a partial typed result.
Semantic tags have `value: None`, while a valued NULL is
`Some(PropertyValue::Null)`. Date and Time are valid separately; their combined
pair is invalid under the corrected NameValue grammar. Raw RP/RPM return types
and per-property errors are unchanged. This decoder does not apply the bundled
server's local provisioning limits or naming policy to remote data.


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

Single-property `subscribe_cov_property` and `subscribe_cov_property_to_device`
require a `std::num::NonZeroU32` lifetime in seconds; 28,800 seconds and the full
positive `u32` range are accepted. Use their explicit `unsubscribe_...` methods
for cancellation. The typed `SubscribeCOVPropertyRequest::encode` returns `Result`
and validates the entire subscribe/cancel field pairing before appending bytes.
The server rejects a missing member of the confirmed/lifetime pair as
INCONSISTENT_PARAMETERS (the selected syntax interpretation), and paired zero
lifetime as SERVICES/VALUE_OUT_OF_RANGE, before lookup or subscription mutation.
Structural decoding preserves these values so the formal responses remain distinct.
Ordinary `subscribe_cov` retains `None`/zero indefinite lifetime behavior. Its
`SubscribeCOVRequest::encode` also returns `Result`: a present lifetime requires
an explicit confirmed-notification mode, while mode alone is valid. Both absent
means cancellation. Invalid lifetime-only requests leave the output buffer
unchanged; the server returns INCONSISTENT_PARAMETERS before object lookup,
expiry cleanup or subscription changes. Public Rust/Python ordinary subscribe
methods already supply the mode and retain their optional lifetime signatures.
Python exposes ordinary COV and PropertyMultiple, not the single-property API.

The full server owns the served Device execution profile. Every Device's
`Protocol_Services_Supported` reports the fixed `EXECUTED_SERVICES`, with the
existing clock-dependent time-service filter. Both `Active_COV_Subscriptions`
and `Active_COV_Multiple_Subscriptions` are present: the selected lowest Device
gets live lists and other Devices get empty lists. Its `Device_Address_Binding`
lists the server's device bindings at the time of the read (#1369): each
configured `DeviceBinding` and each device whose I-Am arrived in the last ten
minutes (a targeted Who-Is's answer included), in instance order, with network
number 0 for a device on this network and the device's own MAC, not its
router's, for one elsewhere. Other Devices read an empty list. Network RP,
budgeted RPM, `read_local` and `generate_pics` share effective Device
definitions, including
Property_List, ALL/OPTIONAL/REQUIRED classification and array-index behavior.
Normal public Device profile mutation, same-OID replacement and custom object
readers cannot change this served contract. Other properties retain their object
behavior. WP, WPM and network-equivalent `write_local` reject writes to Device
`Protocol_Services_Supported`, both COV lists, `Device_Address_Binding` and
`Property_List` before calling
a custom writer. Existing object/index/value validation and authorization remain
in force; WPM retains its successful prefix and first failed write coordinate.
Direct object/database mutation remains the raw declaration boundary. Device
membership changes still do not rebind discovery identity.

`DeviceObject::set_services_supported` is a standalone declaration, not a
runtime service toggle. Raw built-in Device metadata and reads include property
152 only for declared SubscribeCOV or SubscribeCOVProperty, and property 481
only for declared SubscribeCOVPropertyMultiple; present standalone lists are empty.
Direct database reads, context-free `handle_read_property`/`handle_read_property_multiple`
and standalone `PicsGenerator` use raw object declarations. Use the full server's
read and PICS methods for its execution view.

PICS property rows aggregate all configured instances of each object type in
ascending property-ID order, including single-instance output. A row or read/write
flag means at least one instance supports it; actual access still depends on the
concrete object. A property is optional only when all of its present metadata rows
are optional; a required declaration wins, and absent rows do not vote. For example,
a writable stream File contributes writable File_Size while a writable record File
contributes writable Record_Count. Each served Device passes through the execution
view before this union. Type-level createable/deleteable flags and runtime File
behavior are unchanged. Generated PICS remains draft internal support evidence.

The bundled server keeps ordinary object, Single-property and Multiple-reference
subscriptions independent. All families identify the original BACnet client address
(local MAC or routed SNET/SADR), independently of the immediate router. Ordinary
and Single keys additionally identify process and monitored object; Single also
includes property and array index. Absent, zero and element indexes differ.
Confirmed mode is mutable for ordinary/Single renewal; Multiple includes form in
its context identity, so its two forms coexist.

Each successfully admitted ordinary/Single renewal selects its proposed delivery
route and terms. This includes already-permitted ordinary indefinite renewals;
Single still requires a positive finite lifetime on the wire. Cancellation through
either router removes the canonical context. Refused renewal preserves the live
target's route, terms and paired observation. Each accepted renewal replaces its
generation and resets its observation through the normal initial-notification path;
stale work cannot complete into the replacement. Already admitted old-route work
may finish, and confirmed observations still commit at admission rather than ACK.
Exact duplicates in a Multiple request use the last options once, with quota and
generation capacity reserved before any accepted context refresh.

The latest successfully admitted finite Multiple request sets the current delivery
route, lifetime and delay for every retained reference. An empty finite renewal
updates an existing context but creates none. Cancellation through either router
removes canonical targets without retargeting survivors. Rejected admission preserves
the live target context; unrelated expired-entry purging and counters may still run.
Changing route preserves unreplaced selected-value/flags observations and reference
generations, while a private route ownership token fences every old-route snapshot.
The exception is a confirmed context whose report is outstanding, or failed and
still owed, at the move: the kept untimestamped references that report carried
forget their observations, as described for fenced reports below (#923).
The routed address is a claimed protocol identity, not authentication; existing
mutation authorization still precedes subscription handling.

The pre-1.0 Rust API shares `CovRecipient` between subscription identity and
quota/notification accounting. It replaces the former `MultipleRecipient` and
`CovPeerKey` types without aliases. `CovSubscriptionKey::{Object, Property}` and
`MultipleContextKey` use a `recipient` field; `CovSubscription::recipient()` returns
the canonical address. `CovPolicy::reserved_recipients` holds explicitly reserved
canonical recipients (the existing `reserved_peers` remains a direct-MAC policy).
Table admission rejects routed recipients with an empty source MAC, just as NPDU
source decoding does, before purging or modifying subscriptions. Invalid routed
input is never reinterpreted as a direct peer.

`subscribe_multiple` takes an explicit `&SubscriberEndpoint` route after the context
argument and validates it against the recipient and proposals. It admits the
proposals in request order, checking the recipient's quota and the table's
capacity one proposal at a time; a renewal or a repeat of an earlier proposal
takes no slot. The first proposal that does not fit fails the call with a
`MultipleRefusal` that carries the RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT
error, its position and the snapshots kept for the proposals before it, which
renewed the context as an accepted request would (#1058, #1059). When that is
the first proposal, or the request fails as a whole (identity or route
mismatch, generation exhaustion), `refused` is `Some(0)` or `None` and nothing
changes. The SubscribeCOVPropertyMultiple handler sends the kept references'
initial notifications along with the error that names the refused one
(Clause 13.16.2).
`CovSubscription::endpoint()` on subscription data (also available through accepted
snapshots) reports its captured delivery route.

The public `CovSubscriptionTable` accepts proposed `CovSubscription` values through
fallible `subscribe`/`subscribe_multiple` methods and returns immutable
`CovSubscriptionSnapshot` values. Lookup/cancellation use `CovSubscriptionKey`;
completion takes the accepted snapshot, so an old initial or fanout completion
cannot overwrite a renewed/recreated subscription. Checked generation exhaustion
returns RESOURCES/NO_SPACE_TO_ADD_LIST_ELEMENT before live state changes;
cancellation remains available. `CovTimeRemaining::at(expiry, now)` distinguishes
indefinite, positive finite and expired lifetimes. Positive finite fractions round
up, saturating at `u32::MAX`; only indefinite state projects to wire zero. This is
our local representation policy, not a Standard-prescribed rounding formula.
`CovSubscriptionTable::remaining_lifetime(snapshot, now)` checks the captured
owner/key/generation/route authority and resolves the live expiry, including context-only renewal.
Initial and later notifications recheck eligibility after property reads and before
fresh admission. This is a point-in-time check, not byte retraction if cancellation
races afterward. Multiple retains only values with their own current authority;
a failed live read cannot authorize a stale sibling's payload or companion.
Already admitted confirmed notifications retain their APDU and retry/ACK lifecycle.
`BACnetServer::remove_peer_subscriptions` removes
only entries using the exact current immediate endpoint plus routed source; cleanup
of an obsolete router does not remove migrated subscriptions. Canonical recipient
accounting does not grant cleanup authority to an obsolete route.

Property subscriptions now prepare one selected-coordinate `CovSample` for comparison,
wire payload and fenced baseline completion; a failed selected read or encoding
never substitutes Present_Value. The pre-1.0 table API uses `last_notified_observation: Option<CovObservation>`
with completion owned internally by the notification executor. The former public
`set_last_notified_observation` bypass is removed. Each observation pairs
a required `CovSample` with compact absent/present flags.
`CovObservation::new(sample, flags)` validates present flags; private immutable
fields expose `sample()` and `status_flags()` (the four used bits).
`CovSample::new(&value)` is fallible;
its private immutable storage is normalized and shared by snapshot clones. It
bounds retention before copying/recursive encoding to 32 nested List levels
(root List is level 1), 1,024 nodes including empty Lists, and 65,536 scalar/raw
payload bytes. These local caps remain active under `CovPolicy::unlimited()`;
they do not constrain allocations inside a custom object's read callback.
Admission-time overflow returns RESOURCES/NO_SPACE_TO_ADD_LIST_ELEMENT before
replacement/context refresh. A later unavailable or oversized value is skipped
without advancing its baseline. Independent notification traffic budgets remain.

Unconfirmed notification preparation reserves a checked, nonwrapping ticket only
for a complete eligible observation, before later waits. Each live reference
retains one last-successful marker: successful sends atomically advance that
marker and the entire observation only when their ticket is newer. A failed,
cancelled or refused newer send does not block an older successful send. This
local policy covers ordinary, Single and Multiple reports, including specialized
Value_Source tuples; overlapping companions never complete unqualified references.
Existing owner, generation, route and lifetime fences still apply. Same-route
Multiple expiry refresh retains progress; reference replacement resets it.
Ticket exhaustion suppresses further candidates of either form for that table.

Confirmed reports draw tickets from the same counter but complete only on the
subscriber's Ack (#896). Each coordinate has one outstanding confirmed report: an
ordinary or SubscribeCOVProperty subscription, or a whole COV-multiple context.
A context is one coordinate because every notification to it carries all the
timestamped changes queued for it (Clauses 13.1, 13.16.3.1.2.3, 13.17.1.1.5), so
one reference's report cannot be outstanding while a sibling's goes out. While a
report is outstanding the coordinate is marked with its ticket and fanouts skip
it, so later changes wait instead of going out as a second report. The Ack
advances each carried reference's baseline to the acknowledged observation,
clears the mark and fans the coordinate out again through the usual path; for a
context that covers every live reference, held or carried. A change made in the
meantime, Status_Flags included, then follows in one notification, and an
unchanged value sends nothing. Under DCC that follow-up is dropped like any
fanout rather than deferred, so a held change waits for the coordinate's next
fanout after communication is re-enabled.

Exhausted retries and an Error, Reject or Abort answer leave the baseline where it
was and hold the coordinate off for one full retry cycle: the retry timeout times
the attempts, the first one plus every retry. A fanout inside the hold-off skips
the coordinate and schedules nothing. The first fanout after it reports the change
again; for a context, whichever object that fanout was for, it hands the whole
context to one follow-up, so changes held on every object go out together. Nothing
re-sends by itself, so a subscriber that stopped answering, or keeps refusing,
costs at most one delivery attempt per hold-off however often its objects change,
and cannot keep the per-peer and global in-flight slots to itself. Shutdown and
cancellation clear the mark without a hold-off, and so does DCC ending a report at
a retry (see [Confirmed notifications under
DeviceCommunicationControl](#confirmed-notifications-under-devicecommunicationcontrol)).
The retry timeout starts once each send has completed, and the transport bounds
the send itself, so a report stays
outstanding for the transport's send bounds plus the retry cycle. The standard
ends delivery with the confirmed-request retries (Clause 5.4.4); reporting again
after a hold-off is local policy.

A replaced ordinary or SubscribeCOVProperty subscription starts unmarked, as does
a context whose route changes or which is re-subscribed with a non-empty list
while it is busy. The old incarnation's report stops retrying and can no longer
complete or unmark the new one, so the initial report of a renewal or
re-subscription (Clauses 13.14.2, 13.16.2) is not held behind it. When a context
report is fenced this way while outstanding, or while a failed one is still owed a
follow-up, every reference of the context, kept or relisted, is evaluated again.
Relisted references have no baseline yet, so whichever of that follow-up and the
initial report goes first carries them as first reports, and the other finds the
context busy. The fenced report may already have reached the subscriber, so the
follow-up can repeat changes of the kept references.

Because the fenced report's outcome no longer counts, its delivery cannot be ruled
out, and the subscriber may hold values newer than the baselines. So the same
fence first clears the baseline of each kept untimestamped reference that report
carried (#923), and the follow-up reports the current value of each one. This
covers a fence during the hold-off after a failed report too. Without the reset, a
carried reference that went back to its old baseline value before the follow-up
would look unchanged, and the subscriber would keep the value the fenced report
carried until the reference changed again. References the report did not carry
keep their acknowledged baselines, so the follow-up does not grow to the whole
context. Timestamped references keep their baselines too: the fenced report's
history returns to their queue and a change back is captured as one more change,
so the subscriber ends at the current value. A fence while no report is
outstanding or owed clears nothing.

The same gap remains without a fence, and is accepted: a failed report keeps its
baselines, so if it did arrive (only its Acks were lost) and the value goes back
during the hold-off, the owed follow-up sees no change and nothing is sent again.
A fence that comes after the owed follow-up has been taken clears nothing either.

The follow-up task handles each batch of references on its own. If evaluating a
batch panics, the panic is caught (in unwind builds) and logged with the number of
references, and those references wait for their next fanout; later batches still
run. The peer and global in-flight limits, event budgets and throttling counters
apply to every report; a follow-up spends one event budget per object or context,
as a natural fanout does.

This orders prepared observations, not original object mutations, transport byte
order or remote receipt. In particular, a retained Binary Lighting terminal
snapshot prepared after a newer live report may become the baseline even though
its object state is older. Untimestamped reports have no event-time history or
replay guarantee.

Timestamped SubscribeCOVPropertyMultiple references (§13.16.3.1.2.3) record each
qualifying change together with the Device clock frame of its commit. The capture
runs under the database write guard of network WriteProperty and
WritePropertyMultiple, `write_local`, Staging target writes and source completion,
Binary Lighting terminal transitions, committed intrinsic transitions (both
write-triggered and those confirmed by the periodic Time_Delay task),
fault-detection reliability changes and schedule writes. WritePropertyMultiple
captures each successful attempt as it commits, so a request that writes a value
out and back, or fails after a committed prefix, conveys every change it made.
Life Safety objects capture exactly the properties each mutation changed, with the
same selection as their exact fanout; LifeSafetyOperation changes, on any object,
do the same. The admission check and the initial capture share one clock sample.
Changes queue per reference until a notification carrying them is delivered: sent,
for an unconfirmed context, or acknowledged, for a confirmed one (#896). Any
notification to a context also carries the pending changes of that context's other
references (§§13.17.1.1, 13.18.1.1), and each value carries its own
`Time_Of_Change`. A confirmed context sends nothing while its report is
outstanding, so the next notification carries everything held meanwhile (#896).
Earlier changes of a reference come first, in capture order, as repeated
coordinates. Its latest change then merges with untimestamped current values under
the existing one-value-per-coordinate rules. A coordinate explicitly subscribed
without timestamps is never repeated as history, and its current row carries no
time even when that selector did not qualify: it governs its coordinate outright.
An explicit timestamped selector that conveys no change in a round only fills in a
missing time when a sibling carries its coordinate. Its captures record the own
value at the commit time even when it moves less than the selector's COV
increment, and an admission or renewal capture counts too, so a carried value the
selector last saw takes the time of that commit. A value no producer captured
takes the preparation time, which is kept for that value; the selector's increment
baseline is untouched. With no time to give, because the Device clock is invalid
or a producer snapshot may be older than the record, the value is left out of the
notification, as a timestamped change without a clock is. A selector cancelled
since its fanout looked owns nothing, so the field goes out as an ordinary
untimestamped value. A companion that already carries a time keeps it. A history
row is dropped only when the next row for its coordinate repeats it exactly
(overlapping selectors of one change, or an unchanged companion); a value that
returns after a different one within the same clock tick stays. The header
timestamp names the newest change whose time the notification carries, captured
now or kept. It describes one notification, so across notifications to a context
on several objects it can move back. The initial report after admission or
re-subscription is stamped with the Device time of admission; this is a local
convention, since no change has been observed yet. A renewal keeps changes not yet
conveyed, including those of a notification that fails during the renewal.

Changes that one notification cannot carry go out in several (§13.1,
§13.18.1.1), strictly in capture order (#1008). Each notification fits the smaller
of the server's `max_apdu_length` and the max-APDU-length-accepted from the header
of the subscriber's latest SubscribeCOVPropertyMultiple request
(`subscribe_multiple` takes it as `subscriber_max_apdu`; `None`, unknown, keeps the
value advertised before). The oldest changes go first, as many per notification as
fit, with no exception for a reference's latest change, and the last notification
carries the untimestamped values with the newest changes that still fit. So every
change in one notification is older than every change in the next, and each
notification's header timestamp names the last change it carries. A reference
whose latest change went out in an earlier notification conveys no change in the
last one; a sibling there that carries its coordinate times it as for any
timestamped selector that conveys no change, described above. Its observation
completes when the notification carrying its latest change is sent, for an
unconfirmed context, or acknowledged, for a confirmed one. An unconfirmed report
sends every part, each retired once transmitted. One such report goes out per
context at a time: another fanout of the context meanwhile leaves its changes
queued, and the report hands the context to one follow-up once done, so no newer
change reaches the subscriber ahead of older parts. A send failure, an exhausted
event budget, or communication being disabled before a part (Clause 16.1) stops
the rest, which the `Max_Notification_Delay` backstop below retries once nothing
blocks it. Clause 13.18 expects several unconfirmed notifications when the changes
do not fit one; the confirmed service (Clause 13.17) says nothing about splitting,
so splitting a confirmed report is local policy: it sends only its oldest part and
returns the rest to the queue once it holds the context, the Ack's follow-up sends
the next part, and a part that fails goes out again first after the hold-off. A
change that does not fit a notification even alone, a reference's latest
included, goes out one value per notification instead, in the order its values
were captured, each with the change's `Time_Of_Change` and an envelope naming
the change (#1090). Only the notification with its last value completes the
reference; values whose notification fails or is deferred return as one
change, and once what is left of it fits, it goes out like any other change.
A value that does not fit even alone is dropped, and its change counted, since
every attempt to send it would fail; the change's other values still go out.

Untimestamped values that alone exceed one notification, as in the initial report
of a SubscribeCOVPropertyMultiple request over many objects, go out after every
timestamped change, in as few notifications as fit (#1038): runs of whole object
items, with one object's references apart only where its item alone does not fit.
Each of those notifications completes only the references it carries, and a
reference whose values fit no notification on their own is left out with a
warning, not sent over the limit; it is evaluated again at its next fanout.
Each report that leaves such a reference out increments
`CovCounters::untimed_references_oversized` once (#1066).
Untimestamped values have no queue: a report that began going out owes the
untimestamped references of the parts it did not deliver, those left after an
unconfirmed report stopped, or those a confirmed report deferred once it holds the
context. The backstop below treats an owed reference like a pending change of its
context, from when it was first owed, and the report that next evaluates it reads
its value afresh, so a newer change goes in place of the value first prepared. A
confirmed report puts owed references ahead of newer changes, so a split report
cannot keep deferring them. A report none of whose parts went out owes nothing
new, as before. An unconfirmed context without timestamped references holds its
one-report turn only while a report of several parts goes out.

As a local bound, one context's pending changes are limited to an estimate of what
four notifications of that size can carry. A notification's room for items is that
size less the octets the encoder puts around them for the context: the request
header, confirmed or not, the process identifier and the lifetime left at the
context's last admission in their fewest octets, the device identifier, the
timestamp and the list's tags, 25 to 33 octets in all (#1197). Each change counts
its encoding and one item's framing, as if it started an item of its own. The
context also keeps room, at most one notification's worth, for the most its
untimestamped values have taken in one report since it was last admitted or lost
a reference. Memory has a ceiling of its own (#1287), counted in the bytes each
pending change really takes (#1357): the change itself and, for each value, its
slots in the change's vectors and its encoded octets. One context's changes never
take more than four bytes for each octet four notifications of the server's own
maximum APDU have for items, whatever its subscriber's size, so many tiny changes
cannot outgrow it: 23,216 bytes at a 1476-octet maximum, some 116 of the smallest
changes. That memory takes no room in a notification, so with the shortest
envelope a 50-octet subscriber keeps four REAL Present_Value changes, one per
notification, on a server whose own maximum APDU is 77 octets or more. Near the
local maximum the ceiling binds first for small changes, and the subscription
caps (`CovPolicy`) limit how many contexts there are: under the default 1,024
subscriptions the histories take about 24 MB at most, besides the changes that
are never dropped (below). Only on overflow
of either limit, the last resort, is a change dropped: the oldest of the same
reference first, then the oldest in the context, never a reference's latest.
Nor is a reference's change in delivery dropped: once a change sent one value
per notification has a part delivered, or sent as a confirmed report's first
part, the rest of it stays queued until its last value is delivered, however
small the subscriber's maximum APDU (#1163). Beyond the bound, a context
therefore holds at most two changes per reference. Parts a confirmed report
defers return to the queue without that check, so the bound never drops what the
report just planned to send. Changes returned by a failed notification wait
while a newer change of the same reference is in flight; once a newer change is
delivered, older ones are dropped rather than delivered as stale state. Every
one of these drops increments `CovCounters::timed_changes_dropped`, as does each
change that loses a value too large for any notification;
splitting is not counted. The log gets one warning per context for each cause
(bound overflow, a value too large for any notification, superseded), and later
drops for that cause are logged at debug level only, until the context is
admitted afresh: a timestamped reference of it is subscribed again, or an
admission changes the maximum APDU its notifications must fit (#1039). The
counter is therefore the
running signal. Untimestamped references left out as too large
are counted apart, in `CovCounters::untimed_references_oversized`: nothing of
theirs is lost, since the next fanout reads their values again, and the count is
per report rather than per change. `CovSubscriptionTable::with_max_apdu_length`
sets the local maximum (the full server uses its configured capacity).

A subscriber whose maximum APDU cannot hold one timestamped change of its
references gets each such change one value per notification, as above (#1090).
Measured with this encoder, one timestamped change of a REAL Present_Value with
its Status_Flags takes 58 to 64 octets in an unconfirmed notification and 60 to
66 in a confirmed one (more for larger subscriber process identifiers and
lifetimes), and a Binary Present_Value with its Status_Flags 55 to 63: both over
the 50 octets of the smallest maximum APDU a request can advertise, and well
within the next size, 128. Each of those values alone takes 44 to 52 octets
unconfirmed and 46 to 54 confirmed, so a 50-octet subscriber typically gets
Present_Value and Status_Flags in two notifications, but not with the largest
process identifiers and lifetimes. A value that fits no notification even alone
is dropped and counted (#1039): the subscription is still accepted, since the
standard defines no error for refusing one because the subscriber's APDU is too
small. Without timestamps the same change takes 36 to 44 octets in one
notification, so a 50-octet subscriber that needs no change times is better off
with Timestamped=FALSE. At 128 or 206 octets, only large values such as long
character strings or lists can exceed one notification.

Changes are reported as soon as they happen. When their notification fails or is
held back (a failed send, a confirmed report that went unacknowledged,
DISABLE_INITIATION, an exhausted budget), `Max_Notification_Delay` bounds the
wait: once the delay has passed since the earliest queued change, the context is
fanned out again without waiting for another change (§13.1, §13.16.1.1.4). The
delay is an upper bound, so once nothing blocks them overdue changes go out
promptly: re-enabling communication (by DeviceCommunicationControl or when its
timer expires), or admitting a shorter delay, retries them at once; a confirmed
hold-off moves the retry to the end of the hold-off; and the Ack of a confirmed
report still outstanding sends whatever it held back. As local policy the
backstop acts no sooner than one second after the change, and otherwise retries
a blocked context at most once per delay (one second at least). Timestamped
WritePropertyMultiple changes of Life Safety references that the request's
exact fanout did not select are evaluated again right after it. A change no producer captured, such as
a raw database mutation, still reports through the builder's current-state
fallback, stamped when the notification is prepared.

Background commits fan COV out as a network write does, once their database guard
is dropped, to ordinary, SubscribeCOVProperty and Multiple subscribers alike: the
periodic intrinsic task's transitions (after their event notifications),
fault-detection reliability changes and schedule writes to controlled objects. Life
Safety objects report exactly the properties the pass changed. The bundled Event
Enrollment objects accept no COV subscriptions, so their periodic evaluation fans
nothing out. The usual COV criteria and DCC suppression apply. The criteria report
only an actual change: a missing or non-positive COV increment means any change,
and a value equal to the last one sent is not reported again, however often its
object is fanned out, unless a Status_Flags change carries it.

The built-in commandable objects expose `Priority_Array` as read-only (§19.2.1).
Set or relinquish a priority slot by writing a value or NULL to `Present_Value`
with the desired priority. Whole-array and indexed `Priority_Array` writes are
denied, including index 0 (the element count); indexed reads remain available.
A refused direct array write does not change the effective value or terminate
an active lighting operation. WPM preserves valid earlier writes when it reaches
such a denied element. This does not impose a write policy on custom objects.

The selected local property profile compares Real, Double, Signed and Unsigned
values in their own types. Only numeric Present_Value inherits the object's
COV_Increment when omitted; other numeric coordinates without an increment report
actual value changes. Integer deltas remain exact, including large Unsigned64
values. Actual array index zero reports count changes and ignores increments.
Positive numeric slots use their own delta; Null and numeric-type transitions
report without coercion. Structured non-array values and reviewed whole
Property_List, Priority_Array, State_Text, Event_Time_Stamps and
Event_Message_Texts coordinates use typed structural equality and ignore
increments. Support follows the reviewed object/property matrix, not the current
numeric appearance of List children. Unclassified whole arrays are refused with
PROPERTY/NOT_COV_PROPERTY whether an increment is present or absent. Indexed
non-arrays that pass existing read validation return PROPERTY_IS_NOT_AN_ARRAY.

For matching finite numeric values, nonpositive increments (including negative
infinity) report any change, and an unchanged value reports nothing; NaN and
positive infinity increments do not trigger numeric deltas. Initial reporting and type transitions still
report. Same-type nonfinite samples compare IEEE bits; identical NaN payloads and
infinities are stable. Structural equality also preserves float bits, while finite
numeric signed zeros compare equal. These are explicit local exceptional-value
policies, not Standard-prescribed arithmetic. Ordinary whole-object values follow
the same rule (#889): a numeric Present_Value must move by the increment, and a
non-numeric or increment-less one must change. Life Safety committed-delta
triggers are preserved, and a baseline still advances only when an unconfirmed
report is sent or a confirmed one acknowledged (#896).

Applicable Status_Flags changes independently trigger ordinary and property COV.
Property reports include the selected value and declared-present flags; explicit
flags appear once and Multiple emits one companion per retained object. Effective
`property_list()` declares presence. Present flags must be a one-byte BitString
with `unused_bits = 4` and zero unused low bits. Selected read/encoding/cap failure
or declared-present flags failure skips the whole observation, including ordinary
Present_Value: no partial flags-only report and no baseline advance. This transient
failure policy is local. An absent companion is distinct from no delivered baseline;
a later present value can trigger, disappearance alone cannot. A successful
selected-value report while absent records absence. An unreported absent-and-same-
value-return cycle is not tracked.

Multiple reads flags once per object **within each notification context**, pairing
selected values under the same DB/snapshot borrow, then releasing it before
transport. Separate contexts may sample at different times; custom interior-mutability
callbacks are not promised atomic hardware sampling. Only references surviving
late lifetime/ownership checks authorize companions, timestamps and paired baseline
completion. Ordinary nonnumeric/no-increment values report only on change (#889).

This profile does not add empty finite Multiple contexts, delayed Multiple
notifications, live Device subscription-property projection, general numeric
whole-array reduction or specialized object-specific report sets.


```rust
// Subscribe to one property with an explicit finite lifetime.
client.subscribe_cov_property(&mac, CovPropertySubscription {
    subscriber_process_identifier: process_id,
    monitored_object_identifier: oid,
    monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
    monitored_property_array_index: None,
    confirmed: false,
    lifetime: std::num::NonZeroU32::new(28_800).unwrap(),
    cov_increment: Some(0.5),
}).await?;
client.unsubscribe_cov_property(&mac, process_id, oid,
    PropertyIdentifier::PRESENT_VALUE, None).await?;

// Ordinary object subscription follows its separate lifetime rules.

// Subscribe
client.subscribe_cov(&mac, process_id, oid, true, Some(300)).await?;

// Subscribe to multiple properties at once
let cov_specs = vec![COVSubscriptionSpecification {
    monitored_object_identifier: oid,
    list_of_cov_references: vec![COVReference {
        monitored_property: PropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        },
        cov_increment: Some(0.5),
        timestamped: true,
    }],
}];
let request = SubscribeCOVPropertyMultipleRequest {
    subscriber_process_identifier: process_id,
    issue_confirmed_notifications: true,
    lifetime: Some(300),
    max_notification_delay: Some(10),
    list_of_cov_subscription_specifications: cov_specs,
};
let mut service_data = bytes::BytesMut::new();
request.encode(&mut service_data)?;
client
    .confirmed_request(
        &mac,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
        &service_data,
    )
    .await?;

// Receive notifications (broadcast channel — multiple consumers OK)
let mut rx = client.cov_notifications();
let notification: COVNotificationRequest = rx.recv().await?;

// Unsubscribe
client.unsubscribe_cov(&mac, process_id, oid).await?;
```

`SubscribeCOVPropertyMultipleRequest::encode` is fallible and validates the entire
request before appending bytes. Invalid timing pairs, empty nested reference lists,
prohibited property selectors and the cumulative reference limit return
`Error::Encoding` without changing the destination buffer. The former `try_encode`
and panicking `encode` split has been removed. An empty outer list remains encodable
with omitted or valid finite timing; finite empty encoding does not establish that
the bundled server materializes an empty subscription context.

### Discovery

```rust
client.who_is(None).await?;                             // every device, globally
client.who_is(Some(DeviceInstanceRange::new(1000, 2000)?)).await?; // a range
client.who_has(WhoHasObject::Name("Zone Temp".into()), None).await?;

let devices = client.discovered_devices().await;         // Vec<DiscoveredDevice>
let device = client.get_device(1234).await;              // Option<DiscoveredDevice>
client.clear_devices().await;                            // reset table
```

### Device Management

```rust
client.device_communication_control(&mac, EnableDisable::DISABLE_INITIATION, Some(60), Some("password".into())).await?;
client.reinitialize_device(&mac, ReinitializedState::WARMSTART, None).await?;
```

### Object Management

```rust
client.create_object(&mac, ObjectSpecifier::Type(ObjectType::ANALOG_INPUT), initial_values).await?;
client.delete_object(&mac, oid).await?;
```

### Alarms & Events

```rust
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;

// Echo the event notification's timestamp; time_of_acknowledgment is the local time.
let ack = AcknowledgeAlarmRequest {
    acknowledging_process_identifier: process_id,
    event_object_identifier: oid,
    event_state_acknowledged: EventState::HIGH_LIMIT,
    timestamp: notification_timestamp,
    acknowledgment_source: "operator".into(),
    time_of_acknowledgment: now,
};
client.acknowledge_alarm_request(&mac, &ack).await?;
let raw = client.get_event_information(&mac, None).await?;
```

GetAlarmSummary and GetEnrollmentSummary have no dedicated client methods.
Send them with `confirmed_request` and decode the ACK with `bacnet_services`:

```rust
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_types::enums::{AcknowledgmentFilter, ConfirmedServiceChoice};
use bytes::BytesMut;

// GetAlarmSummary takes no parameters.
let raw = client.confirmed_request(&mac, ConfirmedServiceChoice::GET_ALARM_SUMMARY, &[]).await?;
let alarms = GetAlarmSummaryAck::decode(&raw)?;

let request = GetEnrollmentSummaryRequest {
    acknowledgment_filter: AcknowledgmentFilter::ALL,
    enrollment_filter: None,
    event_state_filter: None,
    event_type_filter: None,
    priority_filter: None,
    notification_class_filter: None,
};
let mut service_data = BytesMut::new();
request.encode(&mut service_data);
let raw = client
    .confirmed_request(&mac, ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY, &service_data)
    .await?;
let enrollments = GetEnrollmentSummaryAck::decode(&raw)?;
```

### Life Safety

No dedicated client method: build the request and send it with `confirmed_request`.

```rust
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::enums::{ConfirmedServiceChoice, LifeSafetyOperation};
use bytes::BytesMut;

let request = LifeSafetyOperationRequest {
    requesting_process_identifier: process_id,
    requesting_source: "operator".into(),
    request: LifeSafetyOperation::SILENCE,
    object_identifier: Some(oid),
};
let mut service_data = BytesMut::new();
request.encode(&mut service_data)?;
client
    .confirmed_request(&mac, ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, &service_data)
    .await?;
```

### File Services

```rust
let access = FileAccessMethod::Stream { file_start_position: 0, requested_octet_count: 1024 };
let raw = client.atomic_read_file(&mac, file_oid, access.clone()).await?;
let ack = client.atomic_read_file_decoded(&mac, file_oid, access).await?;
client.atomic_write_file(&mac, file_oid, FileWriteAccessMethod::Stream { file_start_position: 0, file_data: data }).await?;
```

`atomic_read_file` remains the compatibility API for the raw encoded ACK payload.
`atomic_read_file_decoded` performs one request, decodes its `AtomicReadFileAck`,
and validates that the ACK access arm matches the request and does not exceed
the requested window. It does not iterate an entire file.

### ReadRange

```rust
let ack = client.read_range(&mac, oid, PropertyIdentifier::LOG_BUFFER, None, Some(RangeSpec::ByPosition { reference_index: 1, count: 10 })).await?;
let records = ack.trend_log_records()?; // or event_log_records, trend_log_multiple_records, audit_log_records

// Keep a page that breaks a rule, with the rules it broke:
let reply = client.read_range_with(&mac, &request, ReadRangeValidation::Lenient).await?;
for rule in &reply.violations { eprintln!("device broke a ReadRange rule: {rule}"); }
```

`read_range` checks the acknowledgement against its request
(`ReadRangeAck::violations`): the echoed object, property and array index, a
first sequence number present, nonzero and only where the range calls for one,
MORE_ITEMS never set with the flag for the end a ranged read moves toward (with
no range, never with both FIRST_ITEM and LAST_ITEM), and no more items than
the count. A broken rule fails with
`Error::ReadRangeViolation`, naming it; a malformed answer is still
`Error::Decoding`. `read_range_with` and `ReadRangeValidation::Lenient` keep
the decoded page instead, with every rule it broke in
`ReadRangeReply::violations`, so a device that numbers the record after its
sequence wrap 0 doesn't cost a second request. The endpoint client has the
same `read_range_with`.

`ReadRangeAck::log_records()` picks the record kind from the object type of a
Log_Buffer read. Each decoder requires every octet to belong to a record and
the count to equal `item_count`; a failure is a `LogRecordsError` naming the
failing record's index and offset, keeping the records before it.

### Reading a whole log

```rust
use bacnet_client::log_reader::LogCursor;

let mut cursor = LogCursor::Oldest; // or Sequence(n), Position(n), Time(date, time)
loop {
    let page = client.read_log_page(&mac, trend_log, cursor, 100).await?;
    if let Some(gap) = page.gap { eprintln!("lost records before {}", gap.first); }
    store(&page.records); // LogRecords::TrendLog(..), EventLog(..), ...
    cursor = page.next;
    if page.done { break; }
}
save_checkpoint(cursor); // resume from it later for the records logged since
```

`read_log_page` reads one page with one request outstanding:

- `Oldest` finds the oldest record from Total_Record_Count, Record_Count and
  Total_Record_Count again, reading the counts again while the total moves,
  so a record logged in between can't hide the oldest.
- Pages go on from the first sequence number plus the records returned,
  across the wrap from the top of the range to 1; `page.wrapped` marks a page
  that reaches the top.
- MORE_ITEMS only says the answer was cut to fit, so any page that isn't the
  last continues; LAST_ITEM or an empty page ends the read, and `next` is the
  checkpoint.
- An empty page reads the counts again. A record logged since is asked for
  again. A checkpoint the log no longer holds restarts from the oldest record
  with `page.gap` set, as does a first record past the one asked for; a full
  log may drop that one too before it is read, so a read starts over up to
  three times.
- A device that answers with records before the one asked for, as
  bacnet-stack 1.6.1 does past its wrap, fails with `Error::LogNotAdvancing`
  rather than repeating pages; so does one whose counts say it holds a record
  it won't return. Such a device's sequence numbers are inconsistent: read it
  from `LogCursor::Position(1)`. A log that is merely full doesn't need that,
  and a position read of a busy full log skips the records it drops without
  a gap.
- Pages are read leniently: a first sequence number of 0 after a device's
  wrap is accepted and listed in `page.violations`. A device that numbers the
  record after the top 0 rather than 1 is one number ahead of `next` after a
  `wrapped` page: reading on loses that record without a gap.
- A page whose records don't decode fails with `Error::Decoding`, dropping the
  records before the failing one; `read_range_with` and
  `ReadRangeAck::log_records` keep them.

The endpoint client has the same `read_log_page`, and
`bacnet_client::log_reader::read_log_page` runs over any `LogRequester`. The
endpoint session's `min_request_interval_ms` paces its pages as below.

### Pacing

`min_request_interval_ms` on every `BACnetClient` builder (and in
`ClientConfig`), default 0 and at most 3,600,000 (an hour), paces the
confirmed requests to each destination. A request goes once that long has
passed since the latest request sent to that destination finished, by a reply,
an error or its caller giving up, or since that request was sent while it is
still outstanding. A waiting request checks again when it wakes, so a late
reply still gets the whole pause; requests waiting together go one at a time,
the interval apart, in no promised order, and one given up before it went
leaves no trace. It covers paging and polling
alike, so a slow device can serve its other clients between them; requests to
different destinations don't wait on each other. Pacing runs before a routed
request takes its path lease, which every device on that network behind that
router shares, so there the pause after a reply holds for requests made one
after another, not for concurrent ones.

The endpoint client takes the same setting (#1542): `SessionConfig`'s
`min_request_interval_ms`, or `min_request_interval_ms` on `BipEndpointBuilder`,
`ScEndpointBuilder` and `MstpEndpointBuilder`, with the same default and cap
(more than an hour fails `EndpointSession::new`). It uses the same pacer, so
the interval is measured and a destination is keyed (network plus MAC) as
above. A request waits before it reserves an invoke ID, so a waiting request
holds none of the session's shared pool, and an audited request waits before
its record is stamped. A session that stops or drops ends every wait at once
with the shutdown error, so a waiting caller holds nothing of it. On either
client only new confirmed requests wait: a retry keeps its request's turn (as
does a segment of one `BACnetClient` sent; the endpoint client doesn't
segment), and replies, notifications and unconfirmed requests go at once.

```rust
let client = BACnetClient::bip_builder().min_request_interval_ms(50).build().await?;
let session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
    .min_request_interval_ms(50)
    .build_session()?;
```

### List Manipulation

```rust
// A BACnetLIST, here a Notification Class Recipient_List; arrays such as
// Object_List are refused with SERVICES/PROPERTY_IS_NOT_A_LIST.
client.add_list_element(&mac, nc_oid, PropertyIdentifier::RECIPIENT_LIST, None, element_bytes).await?;
client.remove_list_element(&mac, nc_oid, PropertyIdentifier::RECIPIENT_LIST, None, element_bytes).await?;
```

A device that answers with a ChangeList-Error surfaces as `Error::Structured`
with `ErrorDetail::FirstFailedElementNumber`; a device that sends only the
class and code surfaces as `Error::Protocol`.

### Private Transfer

No dedicated client methods: both forms share `PrivateTransferRequest`.

```rust
use bacnet_services::private_transfer::{PrivateTransferAck, PrivateTransferRequest};
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bytes::BytesMut;

let request = PrivateTransferRequest {
    vendor_id,
    service_number,
    service_parameters: Some(params), // already-encoded parameter bytes
};
let mut service_data = BytesMut::new();
request.encode(&mut service_data);
let raw = client
    .confirmed_request(&mac, ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER, &service_data)
    .await?;
let ack = PrivateTransferAck::decode(&raw)?;

client
    .unconfirmed_request(&mac, UnconfirmedServiceChoice::UNCONFIRMED_PRIVATE_TRANSFER, &service_data)
    .await?;
```

### Text Messages

No dedicated client methods: both forms share `TextMessageRequest`.

```rust
use bacnet_services::text_message::{MessageClass, TextMessageRequest};
use bacnet_types::enums::{ConfirmedServiceChoice, MessagePriority, UnconfirmedServiceChoice};
use bytes::BytesMut;

let request = TextMessageRequest {
    source_device: device_oid,
    message_class: Some(MessageClass::Text("fire".into())),
    message_priority: MessagePriority::URGENT,
    message: "Fire alarm".into(),
};
let mut service_data = BytesMut::new();
request.encode(&mut service_data)?;
client
    .confirmed_request(&mac, ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE, &service_data)
    .await?;

let status = TextMessageRequest {
    message_class: None,
    message_priority: MessagePriority::NORMAL,
    message: "Status update".into(),
    ..request
};
let mut service_data = BytesMut::new();
status.encode(&mut service_data)?;
client
    .unconfirmed_request(&mac, UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE, &service_data)
    .await?;
```

### Write Group and Who-Am-I

`write_group` sends a WriteGroup to a `WriteGroupDestination`: one `Device` by
its link address, a `LocalBroadcast`, a `RemoteBroadcast` to network 1 to
65534, or a `GlobalBroadcast`. It returns once the request is sent, since
nothing answers it, and fails before sending when the request doesn't encode
or the remote network is 0 or 65535. Who-Am-I has no dedicated method: build
the `bacnet_services` request and send it through the generic
unconfirmed-request API.

```rust
use std::num::NonZeroU32;

use bacnet_client::client::WriteGroupDestination;
use bacnet_services::who_am_i::WhoAmIRequest;
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_types::enums::UnconfirmedServiceChoice;
use bacnet_types::MacAddr;
use bytes::BytesMut;

// Channel 5 gets REAL 72.0; channel 6 gets NULL at priority 10.
let request = WriteGroupRequest {
    group_number: NonZeroU32::new(1).unwrap(),
    write_priority: 8,
    change_list: vec![
        GroupChannelValue {
            channel: 5,
            override_priority: None,
            value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        },
        GroupChannelValue {
            channel: 6,
            override_priority: Some(10),
            value: vec![0x00],
        },
    ],
    inhibit_delay: Some(false),
};
client
    .write_group(&WriteGroupDestination::Device(MacAddr::from_slice(&mac)), &request)
    .await?;
// The same change list for every device on network 5.
client
    .write_group(&WriteGroupDestination::RemoteBroadcast(5), &request)
    .await?;

// Who-Am-I is usually broadcast.
let who_am_i = WhoAmIRequest {
    vendor_id: 260,
    model_name: "Controller-X".into(),
    serial_number: "SN-0001".into(),
};
let mut service_data = BytesMut::new();
who_am_i.encode(&mut service_data)?;
client.broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_AM_I, &service_data).await?;
```

### Virtual Terminal

The client has no VT-specific methods. Build the `bacnet_services` request and
send it with `confirmed_request`. VT-Open carries both the terminal class and
the caller's own session number (Clause 17.2.1); VT-Close needs at least one
identifier and `encode` returns an error for an empty list; the VT-Data flag
goes out as an Unsigned 0 or 1; and a VT-Data ACK is either `AllAccepted` or
`Partial` with the accepted octet count (Clause 17.4.1.2).

```rust
use bacnet_services::virtual_terminal::{
    VTCloseRequest, VTDataAck, VTDataRequest, VTOpenAck, VTOpenRequest,
};
use bacnet_types::enums::{ConfirmedServiceChoice, VTClass};
use bytes::BytesMut;

let mut buf = BytesMut::new();
VTOpenRequest {
    vt_class: VTClass::DEFAULT_TERMINAL,
    local_vt_session_identifier: 5,
}
.encode(&mut buf);
let raw = client
    .confirmed_request(&mac, ConfirmedServiceChoice::VT_OPEN, &buf)
    .await?;
let remote_id = VTOpenAck::decode(&raw)?.remote_vt_session_identifier;

let mut buf = BytesMut::new();
VTDataRequest {
    vt_session_identifier: remote_id,
    vt_new_data: b"hello".to_vec(),
    vt_data_flag: false,
}
.encode(&mut buf);
let raw = client
    .confirmed_request(&mac, ConfirmedServiceChoice::VT_DATA, &buf)
    .await?;
match VTDataAck::decode(&raw)? {
    VTDataAck::AllAccepted => {}
    VTDataAck::Partial { accepted_octet_count } => {
        // Resend the octets after the first `accepted_octet_count`.
        let _ = accepted_octet_count;
    }
}

let mut buf = BytesMut::new();
VTCloseRequest {
    list_of_remote_vt_session_identifiers: vec![remote_id],
}
.encode(&mut buf)?;
client
    .confirmed_request(&mac, ConfirmedServiceChoice::VT_CLOSE, &buf)
    .await?;
```

### Audit Services

Audit `target_value` and `current_value` distinguish `None` (absent) from
`Some(Vec::new())` (present empty, such as an empty list). Both codecs retain
that distinction; encoded NULL remains a separate one-octet value. Structurally
valid values above 32 octets are permitted by the codec. Target Reporters
include complete known values of 0–32 encoded octets and omit larger values
whole under the existing local inclusion policy.

```rust
use bacnet_services::audit::{
    AuditLogQueryAck, AuditLogQueryRequest, AuditNotificationRequest,
};
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bytes::BytesMut;

let notification_request: AuditNotificationRequest = /* build typed request */;
let mut service_data = BytesMut::new();
notification_request.try_encode(&mut service_data)?;
client.confirmed_request(
    &mac,
    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
    &service_data,
).await?;

client.unconfirmed_request(
    &mac,
    UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
    &service_data,
).await?;

let query_request: AuditLogQueryRequest = /* build typed request */;
let mut query_data = BytesMut::new();
query_request.try_encode(&mut query_data)?;
let raw_ack = client.confirmed_request(
    &mac,
    ConfirmedServiceChoice::AUDIT_LOG_QUERY,
    &query_data,
).await?;
let query_ack = AuditLogQueryAck::decode(&raw_ack)?;
```

These remain generic-client examples. The bundled server executes
AuditLogQuery against the retained in-memory snapshot of an explicitly backed
`AuditLogObject`, returning newest-first typed records through the existing
ComplexACK segmentation path. ConfirmedAuditNotification and
UnconfirmedAuditNotification receipt are available only when the server is
configured with exactly one `audit_notification_sink` and the corresponding
fast `audit_notification_authorizer` or
`unconfirmed_audit_notification_authorizer`; missing, false, or panicking
policy fails closed. Each policy receives the immediate MAC, optional routed
NPDU source, configured sink, and decoded request separately from the
peer-reported payload; only the confirmed context has an invoke ID. Accepted
lists merge or create records atomically through the sink's durable backend.
For the built-in `AuditLogObject`, a successful confirmed receipt also stores
its complete exact-request identity and Unix UTC completion timestamp in that
same snapshot transaction. The 60-second / 256-entry receipt list survives a reopen;
retained duplicates are discarded before authorization without a SimpleACK
replay. Entries expire at 60 seconds, and a stored future timestamp fails open
rather than suppressing indefinitely. The general process-local confirmed-
request tracker remains the pending/session guard.

`Log_Buffer` is listed in the object's Property_List, but ReadProperty and
ReadPropertyMultiple answer it with `PROPERTY / READ_ACCESS_DENIED` (also
inside RPM `ALL` and `REQUIRED`): Clause 12.64.10 makes the buffer reachable
only through ReadRange and AuditLogQuery. ReadRange pages the same retained
ring that AuditLogQuery scans, through `AuditLogStorage::retained_records`,
oldest record first. Each item is one bare `BACnetAuditLogRecord` (timestamp
and datum, including log-status and time-change records, which AuditLogQuery
never returns), and By Sequence Number and By Time use the record's Unsigned64
sequence number and timestamp, so a record carries the same sequence number in
both services. `RangeSpec::ByPosition::reference_index`,
`RangeSpec::BySequenceNumber::reference_seq` and
`ReadRangeAck::first_sequence_number` are `u64` for these logs (Clause 15.8).

`AuditLogSnapshot::completed_receipts` is part of the public custom-persistence
snapshot contract. `FileAuditLogPersistence` writes schema v3, reads schema v2
and schema v1 (as an empty receipt list), rejects unknown future versions,
and retains the
existing two-slot generation/checksum recovery policy. When migrating a custom
`AuditLogPersistence` implementation to 0.11.0, add `completed_receipts: Vec::new()`
to newly constructed snapshots and when decoding an older format without
receipts. Thereafter, `commit` must durably store the supplied receipt list
and records in the same atomic snapshot, and `load` must restore both. Dropping
or separately committing the receipts loses confirmed-request duplicate
protection after a reopen. The built-in file backend needs no separate v1
conversion: it writes the current schema on the next successful commit.

Schema v1 and v2 files, which 0.11.0 and earlier wrote, store log-status
records with their three bits reversed. The file backend restores each one
when it loads such a file, so an old log-disabled record reads back as
`LogStatus::LOG_DISABLED`, and the next commit saves the log as v3. A custom
`AuditLogPersistence` that stored encoded records from those releases has to
make the same correction: reverse the three bits of every log-status record
it decodes.

Back up both `.slot0` and `.slot1` files before the first commit under a newer schema. A reader that
supports only an older schema cannot read newer snapshots; rolling back to such an implementation
requires restoring a compatible backup and loses changes made after that backup.

Unconfirmed receipt never emits a response and never stores a confirmed receipt.
Query authorization, sustained rate limiting, and multi-log routing
policy are not provided. The standalone server optionally forwards changed
accepted batches from its selected log after local commit: configure
`AuditLogObject::set_member_of` and a remote configured `DeviceBinding`.
This is one best-effort confirmed attempt, not durable forwarding; restart can
lose send progress. See [Audit Log forwarding](audit-log-forwarding.md) for
properties, failure behavior, resource bounds, and exclusions.
Query input changes never rewrite stored notifications or receipt identities,
and the requested-count, ACK-cap, and segmentation limits stay independent.
Executed-service bit 46
represents receipt only; no Audit Reporting BIBB, including AR-L-A, is claimed.

#### Commits off the database lock

`AuditLogObject` makes every `AuditLogPersistence::commit` call on a writer
thread of its own, one at a time, so a custom backend never sees two calls at
once; dropping the object waits for queued commits. The bundled server stages
each inbound notification batch, each network or `write_local` Log_Enable or
Buffer_Size change, and each `purge_audit_log`: the log builds the next
snapshot and queues its commit, the server
waits for the commit with the object database guard dropped, and only then
does the log take the snapshot. Other requests read and write the database
meanwhile, and the promises above hold: a batch reaches memory, and a
confirmed one its SimpleACK, only after its records and receipt are durable,
and a commit that fails leaves the log as it was. One commit is staged per log
at a time; a second batch waits for the first without holding the guard.
`AuditLogNotificationSink::stage_notification_batch` and
`finish_notification_batch` are the sink's side of this. Their defaults store
the batch at once, so a custom sink keeps committing where it did. The server
waits for a staged commit on Tokio's blocking pool, so under a paused test
clock (`tokio::time::pause`) virtual time does not jump to the next timer
while the writer thread commits.

Application code that changes the log while holding the guard
(`add_record`, or `write_property` through the database) still commits in
place and waits for the commit there, after letting a staged commit land
first. The file backend synchronizes a slot's directory the first time it
creates that slot (on Unix; on Windows `std` cannot open a directory, so that
step is skipped). The commit has landed by then, so a filesystem that cannot
synchronize a directory is passed over and any other failure there is logged,
not returned.

#### Buffer_Size and purging

Buffer_Size takes a write, from a peer or through `write_local`, only while
Log_Enable is FALSE (Clause 12.64.9). With logging on the write fails with
`PROPERTY / WRITE_ACCESS_DENIED`, whatever the value. The value is an
Unsigned up to `MAX_AUDIT_RECORDS`: another datatype is `INVALID_DATA_TYPE`,
and a larger size, 2^32-1 included, `VALUE_OUT_OF_RANGE`. A smaller size
keeps the newest records that fit and drops the rest without a status
record, as the ring does when it overflows; a larger size keeps them all, and
the current size changes nothing. The size is part of the stored snapshot:
a reopened log keeps the size last written, and the `buffer_size` passed to
`AuditLogObject::new` only sizes a log its storage does not hold yet. When the
two differ the log keeps the stored size and logs a warning. To give a stored
log a new size, turn Log_Enable off, write Buffer_Size and turn Log_Enable on
again, or start from empty storage.

Peers cannot purge an Audit Log. Its Record_Count is read-only, unlike the
other logs' (Clause 12.64.11), so a Record_Count write fails with
`WRITE_ACCESS_DENIED`. The application purges the log with
`BACnetServer::purge_audit_log(&oid)`, or with `AuditLogObject::purge` on a
log it holds itself. A purge clears the ring and appends a BUFFER_PURGED
status record, whether or not logging is enabled, flagged LOG_DISABLED too
while it is not (Clause 12.64.10). Total_Record_Count keeps counting, and the
completed receipts survive, so a confirmed notification already stored
is still a duplicate afterwards. AuditLogQuery then returns no records, since
it returns notifications only; ReadRange shows the purge record.

Both stage like a Log_Enable write, so the commit runs with the database
guard dropped and the log serves the new state only once storage holds it.
A WritePropertyMultiple that turns Log_Enable off and then writes
Buffer_Size stages the two together as one commit, made off the guard as
well; the request takes each change as it reaches it, and if it stops
between them, storage is set back to the state the log serves. That holds
when the server stops mid-request too (#1363): `stop()`, once it has joined
its requests, drops changes still staged and waits for the commit of the
served state, and a log dropped with changes still staged commits the served
state as it goes. A staged notification batch is left as it stands, since
the log takes a batch whose commit succeeded.
A commit that fails refuses the write or the purge with
`DEVICE / OPERATIONAL_PROBLEM` and leaves the log as it was; a Log_Enable
write whose commit fails is refused the same way. Changes to one log land one
at a time, in the order they were staged, so a notification batch lands whole
on one side of a purge: a batch already committing when the purge is asked for
lands first and is purged with the rest, and one that arrives while the purge
commits follows the purge record. `purge_audit_log` also refuses an unknown
object with `OBJECT / UNKNOWN_OBJECT`, any object other than a built-in Audit
Log with `OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`, a missing clock with
`DEVICE / OPERATIONAL_PROBLEM`, and any call once the server has stopped.

---

## bacnet-server

Async BACnet server that hosts objects and dispatches incoming requests.

### Server shutdown and local sends

`BACnetServer::stop()` seals new local broadcasts and mutations, joins admitted
server work, then stops the owned network and transport before returning success.
Once its requests are joined, it drops any write a Notification Forwarder,
Notification Class, Access Rights object or Audit Log still holds staged for
one of them and waits until every save those objects have queued has run, so
storage holds what they serve (#1363). That wait has no limit: storage that
stalls holds `stop()` up, and a warning naming the objects still saving is
logged after 5 s and every 30 s after that. `stop()` doesn't wait while the
application holds the database; the objects then settle once it lets go, and
put storage back when they are dropped. An abort lands when a request's task
next yields, so a request already running when `stop()` begins can still make
its write; its answer is sealed, but the object serves that write and storage
holds it (#1457).
Call `stop()` before dropping the server. A server dropped without it in async
code aborts its tasks and hands its object database to a task that drops it on
Tokio's blocking pool once those tasks have let go (#1409), so the drop doesn't
block a runtime worker while the objects' last saves run; nothing waits for
those saves, though. Storage may still change after the drop returns, as they
and the put-back of a staged write land, so `stop().await` before building
another server on the same storage. That task first cancels a DCC timer the
drop couldn't take (#1560), and the tasks `stop()` leaves to settle staged
writes and end runs once the application lets go also drop a last handle on
the blocking pool (#1513). An
application that keeps a clone of `server.database()` releases it with
`bacnet_server::server::drop_database_off_runtime`, which drops the database
on the blocking pool should that clone be the last handle; it usually isn't,
and the call just lets go. It is `stop().await`, not this, that waits for
storage. An endpoint session's own holders let go the same way; see
[Endpoint shutdown and the object database](#endpoint-shutdown-and-the-object-database).
The target-Audit drain retains the ingress needed for acknowledgments until its
existing completion/deadline boundary. Cancelling a stop waiter retains cleanup:
call `stop()` again to join it. Transport cleanup errors retain the owner for retry;
a cleanup-task panic remains an error on later calls.

`broadcast_i_am()` and cloned `IAmBroadcaster` handles share a fail-fast limit of
32 local sends in flight, independent of inbound peer quotas. An admitted send is
server-owned even if its caller stops waiting. Shutdown cancels and joins it;
retained handles reject new sends and do not prolong the transport lifetime.
While DeviceCommunicationControl restricts initiation, a send goes nowhere and
fails with `SERVICES` / `COMMUNICATION_DISABLED` (see [Discovery under
DeviceCommunicationControl](#discovery-under-devicecommunicationcontrol)); an
announce loop should treat that error as a skipped announcement.
Local mutation methods reject before changing objects once shutdown starts.
`read_local()`, PICS, counters and database inspection remain available after
Rust server stop. `local_mac()` retains the last bound address snapshot; it does
not assert that the transport remains active.

Drop seals admission and aborts owned application work. It does not synchronously
join task destruction. If transport cleanup has already begun, that owned task
continues while the runtime runs. Use awaited `stop()` for the joined resource
release guarantee. This is a local lifecycle contract, not a BACnet wire change.

### Confirmed transaction lifetimes

The stack detects exact ordinary confirmed duplicates only while their
server transaction is pending. Once an unsegmented SimpleACK, ComplexACK, Error,
Reject or Abort is encoded and its local network send is issued, the same peer,
Invoke ID and bytes may execute again. The boundary precedes the transport
future's eventual result; it establishes neither physical emission nor peer
receipt. MS/TP uses the synchronous encoded reply-channel handoff. Failed
encoding, failed handoff, cancellation and discarded work release ownership.

A segmented ComplexACK transfers ownership to its response child before the
request handler returns; the child may outlive that handler. It remains pending through the final SegmentACK or
until terminal Abort, timeout, send failure or cancellation. A generated terminal
Abort retires the transaction at local Abort issuance. Task and segmented-send
capacity permits keep their own lifetimes, so a newly reusable Invoke ID may
still encounter the normal resource limit while older send work is active.

Detection remains bounded to 256 pending entries and 64 KiB of service-request
bytes per tracked entry. Requests beyond those detection bounds proceed through
normal service admission. Local peers retain canonical MAC keys; valid routed
peers retain SNET/SADR keys across router changes. Accepted direct SC also retains
the immutable leaf/incarnation partition described above. There is no generic
completed cache or response replay. LifeSafetyOperation's separate completed
replay policy and Audit service receipts are unchanged.

This behavior is new in 0.12.0. `NetworkLayer::send_apdu_on_issuance`
provides the narrow post-NPDU-encoding callback used by these response owners;
constructing its lazy future does not invoke the callback. This does not resolve
response socket affinity or segmented-response ACK/Abort confinement (#524).

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
    .life_safety_operation_authorizer(|context| {
        // Use authenticated deployment identity where available; the
        // Requesting Source string is peer-controlled descriptive text.
        allowed_life_safety_peer(&context.source_mac, context.source_network.as_ref())
    })
    .build()
    .await?;

// SC-specific builder (requires `sc-tls` feature)
let server = BACnetServer::sc_builder()
    .database(db)
    .hub_url("wss://hub:1234")
    .tls_config(tls_config)
    .vmac([0, 1, 2, 3, 4, 5])
    .device_uuid(server_uuid) // Already provisioned; distinct from the client's UUID.
    .build()
    .await?;

// Access the database at runtime
let db = server.database().lock().await;
let value = db.get(&oid).unwrap().read_property(pid, None)?;

// Check communication state
let state = server.comm_state(); // DccState::Enable or DccState::DisableInitiation

// Stop
server.stop().await?;
```

Use `bip_builder()` for B/IP, `sc_builder()` for BACnet/SC, or
`generic_builder()` with a prebuilt transport.

All three Rust builders accept `.mutation_authorizer(|context| ...)`, also
available as `ServerConfig::mutation_authorizer`. It covers only confirmed
WriteProperty, WritePropertyMultiple, CreateObject, DeleteObject, AddListElement,
RemoveListElement, AtomicWriteFile, SubscribeCOV, SubscribeCOVProperty, and
SubscribeCOVPropertyMultiple. **Omitting it allows existing behavior**, unlike
Audit/LifeSafety's fail-closed absence. False or panic returns
`SERVICES / SERVICE_REQUEST_DENIED` without the denied mutation. WPM authorizes
each element in order and retains an authorized prefix on later denial or
malformed input; other covered services authorize once after service decoding.
A WPM element that an object saves before serving it (a forwarder or Notification
Class list, an Access Rights rule array, Enable or Accompaniment, an Audit Log's
Log_Enable or Buffer_Size) is decided before the server stages its save off the
database lock (#1321): still once, and in wire order among those elements, so
ahead of earlier elements no object saves first, and possibly for an element the
request never reaches because an earlier one fails. Counters and audit records
cover only the elements the handler reaches.
Callbacks must be fast, nonblocking, and side-effect-free. Context addresses and
process IDs are claimed, not authenticated identities. DCC/Reinit, Audit/LifeSafety,
reads, discovery, trusted local writes and unconfirmed services other than WriteGroup
are unchanged. An inbound WriteGroup is decided once per Channel write it would make
(#1319): the context's `target` is `MutationTarget::WriteGroup`, its `invoke_id` is
`None` and its `service_choice` is `MutationService::Unconfirmed(WRITE_GROUP)`
(a confirmed request's is `MutationService::Confirmed`, with `Some` invoke ID).
`MutationPolicy::DenyAll` denies each one. A denied Channel write is skipped silently,
since nothing answers an unconfirmed request, and counted in the `write_group` row of
`mutation_decision_counters()`.

Each decision context also carries the reassembled ingress snapshot
(`provenance: TransportProvenance`) and the derived channel/relay scope
(`trust: MutationTrust`, mirroring RB-09 `ControlTrust`): `Unverified`,
`VerifiedChannel` (SC-TLS channel), or `VerifiedRelay` (SC-hub relayed
origin). These enums are scope-only; `context.direct_sc_identity()` separately
exposes the [accepted-direct leaf/incarnation](#accepted-direct-tls-identity).
Baseline-only profile: an unknown
origin — including a hub-mediated unknown leaf, which arrives unverified —
never satisfies a baseline-only allow rule, and receive-permission is never
write-permission; the callback owns the rule. Context `Debug` is redacted
(address lengths and target kind only, no MAC bytes or decoded inputs), and
decision counters retain no per-source state. The callback runs after
validation and before mutation; denials perform no database mutation, no
COV/event fan-out, and no audit-log write (counters and bounded diagnostics
only). This policy governs standalone-server mutations; direct handler calls
and trusted local writes stay outside it. The shared endpoint's narrow
[Device-write authorizer](#authorized-endpoint-device-writes) is configured
separately. Python exposes the same native static policy through
`BACnetServer(..., mutation_policy="permissive" | "deny_all")`, without a Python
authorizer callback. See
[Local mutation authorization](mutation-policy.md).

### Life Safety execution and COV

Inbound LifeSafetyOperation is fail-closed unless an authorizer is configured.
The built-in Life Safety Point and Zone objects execute the six silence and
unsilence operations. `RESET`, `RESET_ALARM`, and `RESET_FAULT` execute only
through a configured application-owned Point/Zone reset executor after exact
`Operation_Expected` arming; omitted commit fields remain unchanged and no
physical state is inferred. Exact confirmed duplicates receive the byte-identical
recorded response from the bounded process-local request tracker (60-second /
256-entry retention; requests over 64 KiB execute untracked), so irreversible
actuation still requires application-owned idempotency across tracker expiry
or restart.

Trusted runtime logic can arm or rearm a Life Safety object through
`BACnetServer::set_life_safety_operation_expected_local`, awaited inside a
Tokio runtime like every local write; a caller dropped once the change is
made still notifies Operation_Expected's subscribers (#1520). The lower-level
`BACnetObject::set_life_safety_operation_expected_internal` channel also remains
available to custom database owners. Protocol WriteProperty and
WritePropertyMultiple cannot forge `Operation_Expected` or `Silenced`.

`BACnetObject::apply_life_safety_operation` returns
`Result<LifeSafetyOperationOutcome, Error>`: an effect plus exact committed
property changes in stable reporting order, with no duplicates. Custom objects
own this projection. `AlreadyApplied` means no state changed and carries no
deltas; errors leave object state unchanged. The default hook explicitly returns
`OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`.

This pre-1.0 API replaces the coarse return value and removes
`apply_life_safety_operation_detailed`; implementations and callers migrate to
the sole outcome-bearing hook. The built-in Point/Zone reset executors, exact
arming, and existing delta calculation retain their behavior. The former public
server `handle_life_safety_operation` helper is removed; use the object hook for
local execution or the client service API for wire requests. Confirmed dispatch
uses one internal handler retaining exact COV changes, including successful
operations whose only changes are private state and have no COV properties.
The bundled server uses those deltas, trusted rearm readback, and exact WP/WPM/
`write_local`/live-Schedule pre/post readback to route Life Safety COV after
unlocking and after the service ACK where applicable.
Whole-object reports are exactly `Present_Value` plus `Status_Flags` and trigger
only when either changes. Property reports are the subscribed property plus one
`Status_Flags` and trigger when either changes. Point and Zone property COV
both support `Present_Value`, `Status_Flags`, `Tracking_Value`, `Silenced`, and
`Operation_Expected`; a Zone reset commit carries `tracking_value` the way a
Point commit does.
Low-level object setters still bypass server notification ownership. They take
the typed enums: `set_present_value` and `set_tracking_value` a
`LifeSafetyState`, `set_mode` a `LifeSafetyMode`, `set_silenced` a
`SilencedState` and `set_operation_expected` a `LifeSafetyOperation`. Each stores
the value as given, so a proprietary value reads back unchanged.

While `Out_Of_Service` is TRUE, WriteProperty, WritePropertyMultiple and
`write_local` take `Tracking_Value` (a standard `LifeSafetyState` or one from
256 to 65535) and `Reliability` (a named value or one from 64 to 65535); in
service both are `PROPERTY / WRITE_ACCESS_DENIED` (#1108). Entering out of
service sets the object's own `Tracking_Value` and `Reliability` aside and the
return to service restores them. Meanwhile `set_tracking_value` and a reset
commit's `tracking_value` replace the value set aside, not the simulated one,
and `set_reliability_internal`, the in-service route for a fault the
application detects, is refused. A simulated value notifies through the same
COV path as any write, and Present_Value, `Silenced` and `Operation_Expected`
don't follow it; a reset executor sees the simulated `Tracking_Value` in its
context.

Once the server holds a Point or Zone, the application reaches its
Present_Value with `BACnetServer::set_present_value_local` and its
Tracking_Value with `BACnetServer::set_tracking_value_local` (#1123), which go
through the `BACnetObject::set_present_value_internal` and
`set_tracking_value_internal` hooks. Both take an Enumerated `LifeSafetyState`,
standard or from 256 to 65535: another number fails with
`PROPERTY / VALUE_OUT_OF_RANGE`, another datatype with
`PROPERTY / INVALID_DATA_TYPE`, an unknown object with `OBJECT / UNKNOWN_OBJECT`
and any other object type with `OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`.
Each sets only its own property: Silenced and Operation_Expected stay put, and
the object never derives one value from the other, so latching Present_Value
until reset is the application's rule (keep Present_Value on the alarm state,
report the live state through Tracking_Value, and commit the post-reset
Present_Value from the reset executor, whose context sees the values the route
left). Present_Value isn't decoupled out of service, so it is taken in either
state; an application Tracking_Value sent while `Out_Of_Service` is TRUE
replaces the value set aside, as `set_tracking_value` does. Changes notify
through the Life Safety COV path once the lock is released: Present_Value
reaches SubscribeCOV and Present_Value property subscribers, Tracking_Value
only its property subscribers. The built-in objects run no intrinsic
reporting, so the post-write event pass raises nothing.

`Accepted_Modes` lists the modes a WriteProperty or WritePropertyMultiple of
`Mode` may select. It starts as every standard `LifeSafetyMode`, and
`set_accepted_modes` replaces it (each mode kept once, in the order given). A
network `Mode` write naming an unlisted value fails with
`PROPERTY / VALUE_OUT_OF_RANGE` and leaves `Mode` unchanged; the local
`set_mode` is not checked against the list.

Member_Of (Point and Zone) and Zone_Members are BACnetLISTs of
`BACnetDeviceObjectReference` (#1182), read-only over the network: a read
serves one `PropertyValue::ApplicationData` per member, and a member in another
device keeps its Device member. The application fills them with
`LifeSafetyPointObject::add_member`, `LifeSafetyZoneObject::add_member` and
`LifeSafetyZoneObject::add_zone_member`, which take a
`BACnetDeviceObjectReference` (an `ObjectIdentifier` converts to a local one)
and return `Result`: a member already listed stays listed once, and another
object type (Member_Of names Life Safety Zones, Zone_Members Points and Zones)
or a Device member that isn't a Device identifier is
`PROPERTY / VALUE_OUT_OF_RANGE`.

This is a bounded operational-state slice with pinned partial metadata (Point
`POINT_BASE` 18 rows, Zone `ZONE_BASE` 17 rows; exact PICS projection tests) and
network read-only `Silenced`/`Operation_Expected`/`Accepted_Modes`; not complete
Life Safety Point/Zone tables, formal PICS/BIBB/profile/device-advertisement,
or intrinsic `CHANGE_OF_LIFE_SAFETY` event-algorithm conformance
(`Event_State` intrinsic-only, `IN_ALARM` latent with no setter).

### Handled Services

The server automatically dispatches:

**Confirmed:**
- ReadProperty, WriteProperty (mutation-gated)
- ReadPropertyMultiple, WritePropertyMultiple (mutation-gated)
- SubscribeCOV, SubscribeCOVProperty, SubscribeCOVPropertyMultiple (mutation-gated)
- CreateObject, DeleteObject (mutation-gated)
- DeviceCommunicationControl
- ReinitializeDevice (restart, apply changes, or a Clause 19 backup or restore
  step): decoded and password-validated, then carried out by the
  `on_reinitialize` handler (see [The ReinitializeDevice handler](#the-reinitializedevice-handler));
  refused with `SERVICES / SERVICE_REQUEST_DENIED` when no handler is set or
  the state is undefined, with password and decode errors keeping their
  precedence
- GetEventInformation, AcknowledgeAlarm
- GetAlarmSummary, GetEnrollmentSummary
- ConfirmedTextMessage
- LifeSafetyOperation (authorized silence/unsilence; reset via configured application executor)
- ConfirmedAuditNotification (explicit sink and fail-closed authorizer; process-local duplicate detection)
- ConfirmedEventNotification (acknowledged once it decodes, then offered to the Notification Forwarder objects)
- AuditLogQuery (retained records; three-state success filter; no query authorization)
- ReadRange
- AtomicReadFile, AtomicWriteFile (writes are mutation-gated)
- AddListElement, RemoveListElement (mutation-gated)

**Unconfirmed:**
- WhoIs / IAm
- WhoHas / IHave
- TimeSynchronization, UTCTimeSynchronization (opt-in source, step and rate limits: [time synchronization policy](time-sync-policy.md))
- UnconfirmedTextMessage
- UnconfirmedEventNotification (offered to the Notification Forwarder objects)
- UnconfirmedAuditNotification (explicit sink and distinct fail-closed authorizer; no response or duplicate tracking)

**Outgoing (server-initiated):**
- COV notifications (confirmed and unconfirmed, with `NotificationTransactions` retries for confirmed)
- Event notifications (confirmed and unconfirmed, routed via NotificationClass recipients, and the copies Notification Forwarder objects send on)
- Audit notifications (confirmed and unconfirmed, from Audit Reporters and Audit Log forwarding; one attempt each, and they keep going out under DISABLE_INITIATION, see [Confirmed notifications under DeviceCommunicationControl](#confirmed-notifications-under-devicecommunicationcontrol))

Confirmed notification invoke IDs, terminal admission and retries belong to
`NotificationTransactions`. A separate private learned-router cache stores up to
64 DNET next hops from admitted, nonempty routed terminal responses. Routed Address
recipients use a learned router on the first attempt and local broadcast on
later retries; a configured Device binding keeps its fixed next hop. The former
public `ServerTsm` type and its unused transaction methods have been removed
without a compatibility alias. `CovAckResult` remains available at its existing
`bacnet_server::server` path. A refused confirmed request ends as
`CovAckResult::Error(Refusal)`: `Refusal::Error { class, code }` for an Error
PDU, `Refusal::Reject(reason)` or `Refusal::Abort(reason)` otherwise (#1323).
`Error::from(refusal)` gives the `Error::Protocol`, `Error::Reject` or
`Error::Abort` a client's WriteProperty would report. The notification senders
only ask whether the request was taken; a Channel member in another device
uses the payload to tell a NULL refused as the wrong datatype, which counts as
written, from other refusals.

A recipient on the local network by number is sent to as a local recipient,
for Notification Class delivery and forwarded copies alike (#1299). Once the
server knows its network's number (see
[Local Network Number controls](#local-network-number-controls)), an Address
recipient naming that number gets its notification with no DNET: a unicast to
its MAC, or a local broadcast when the MAC is empty or the link's broadcast
MAC. A Device binding routed to that network is sent straight to its final
MAC rather than through its router, and a confirmed notification then waits
for the answer from that MAC directly. Such a binding whose final MAC is the
link's broadcast MAC, or any other group address of the link, names no
single device here, so it is skipped and counts in `recipient_unroutable`, as
a local binding at such a MAC does, so no forwarded copy goes to it either.
Clause 6.5.1 sends traffic for the local network without a DNET, and a
non-routing node drops an NPDU whose DNET names a network (Clause 6.5.2.1),
so the routed form might never arrive. The
number is read once per notification from `NetworkLayer::local_network_number`,
without the database lock, and a confirmed notification keeps the route it
was first sent on for its retries. While the number is unknown, an address
naming any network is sent routed, as it is written. The server has one port,
so the local network is that port's; a multi-port device would need the
network attached to each port (#863).

### The ReinitializeDevice handler

`ServerBuilder::on_reinitialize` (also on the B/IP and SC builders, or
`ServerConfig::on_reinitialize`) installs a `ReinitializeHandler`. The server
calls it with a `ReinitializeContext` and the object database once a request
has decoded, passed `reinit_password` and named a state Clause 16.4 defines.
The context carries the requested `state`, the immediate `source_mac`, the
routed `source_network` when there is one, the transport `provenance` (with
`direct_sc_identity()` for a verified direct BACnet/SC peer) and the
`invoke_id`. The struct is `#[non_exhaustive]`, and its `Debug` output shows
address lengths rather than addresses. `ReinitializeContext::new(state,
source_mac, source_network, provenance, invoke_id)` builds one for calling a
handler directly, as its unit tests would; outside `bacnet-transport` only
`TransportProvenance::unverified()` is available for the provenance.

`Ok(())` sends the SimpleACK and `Err(Error::Protocol { .. })` sends that
class and code instead. The rules:

- The SimpleACK leaves only after the handler returns, so it must not restart
  inline. Accept or refuse, prepare, and schedule the restart, backup or
  restore step for after the reply (Clause 16.4.2).
- Nothing yet signals when the SimpleACK has left (#1565), so any delay before
  restarting is best effort, and on MS/TP it can need longer, since a
  postponed reply waits for the token. `BACnetServer::stop()` seals responses
  and aborts request tasks before joining them, so a restart path that stops
  the server before the reply has left drops the SimpleACK.
- On a full server, an accepted WARMSTART or COLDSTART immediately ends
  DISABLE_INITIATION, cancels its timer and resumes held COV (Clause 16.1.2),
  even if the reply later fails.
- It runs synchronously on a runtime worker with the database write-locked:
  keep it quick and hand slow work, such as writing backup files, to a task.
- Its edits through `&mut ObjectDatabase` are raw: they skip what
  `BACnetServer::write_local` adds around a write (the Object_Name uniqueness
  check, the COV fanout, the event pass, Schedule and Command follow-ups, and
  the Audit record). Apply a change that needs those, as ACTIVATE_CHANGES work
  on Network Port or Device properties may, through `write_local` after the
  reply, from a task the handler schedules.
- Without `reinit_password` any peer reaches it. The mutation policy and the
  mutation authorizer don't cover ReinitializeDevice, so restrict sources
  through the context; its addresses are claims, not authenticated identities.
- Once the handler has run, the request can no longer be rejected (Clause
  20.1.8): a returned `Error::Reject`, or any error other than `Protocol` or
  `Structured`, is answered `SERVICES / OTHER`, and so is a panic (caught with
  unwind builds), since the handler may have partly acted.

```rust,ignore
use bacnet_server::server::ReinitializeContext;
use bacnet_types::enums::{ErrorClass, ErrorCode, ReinitializedState};

let (restart_tx, mut restart_rx) = tokio::sync::mpsc::channel(1);
tokio::spawn(async move {
    if let Some(state) = restart_rx.recv().await {
        // Best effort: nothing yet reports that the SimpleACK has left (#1565).
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        restart_device(state); // application code
    }
});
let server = BACnetServer::bip_builder()
    .database(db)
    .reinit_password("secret")
    .on_reinitialize(move |request: &ReinitializeContext, _db: &mut ObjectDatabase| {
        match request.state {
            ReinitializedState::WARMSTART | ReinitializedState::COLDSTART => {
                // Accept now; the task above restarts after its delay.
                restart_tx.try_send(request.state).map_err(|_| Error::Protocol {
                    class: ErrorClass::DEVICE.to_raw() as u32,
                    code: ErrorCode::CONFIGURATION_IN_PROGRESS.to_raw() as u32,
                })
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            }),
        }
    })
    .build()
    .await?;
```

Endpoint sessions take the same handler; see
[Endpoint ReinitializeDevice](#endpoint-reinitializedevice).

### The DeviceCommunicationControl state

`BACnetServer::comm_state()` returns a `DccState`: `Enable` or
`DisableInitiation`. Every `DccPolicy` refuses a DISABLE request with
`SERVICES` / `SERVICE_REQUEST_DENIED` (Clause 16.1), so the server never
disables communication outright and the type has no variant for it.
`EnableDisable::from(state)` gives the wire value (0 or 2), and
`DccState::initiation_restricted()` says whether the server is holding back
what it would start. Only an accepted DeviceCommunicationControl request and
the expiry of its duration change the state, and every start begins at
`Enable`.

### Confirmed notifications under DeviceCommunicationControl

While DeviceCommunicationControl restricts initiation the server sends no COV
or event notification, and that holds for the retries of a confirmed one
already outstanding (Clause 16.1, #1327). Every attempt, the first and each
retry, reads the communication state before it sends. An attempt that DCC
blocks is not sent: the notification ends there, its invoke ID freed at once
instead of after the remaining timeouts. An answer that has already taken the
lease still ends it as usual. The server refuses the deprecated DISABLE, so
DISABLE_INITIATION is the state that does this. What happens next depends on
the notification:

- **COV.** The report ends with no hold-off, because the subscriber did not
  fail, and its baselines stay where they were. Timestamped history goes back
  to its queue, and the `Max_Notification_Delay` backstop sends it once
  communication is enabled again, at once if its delay has run out by then.
  Untimestamped values are reported by the reference's next fanout, as a
  change DCC held back before its first send would be. A change partly sent
  value by value stays in delivery, so the history bound keeps the rest of it.
  A report withdrawn before its first attempt is taken back out of the COV
  counters, since nothing went out.
- **Events.** Nothing in `EventNotificationCounters` moves, and the
  notification is not sent again once communication is enabled, the same as a
  transition DCC stops before its first send. `Acked_Transitions` keeps what
  the transition set; delivery never changes it.
- **Audit.** Not withdrawn, and not held back. Clause 16.1 exempts Confirmed-
  and UnconfirmedAuditNotification from DISABLE_INITIATION, so no audit sender
  reads the communication state (#1370). An audit notification makes a single
  attempt with no retries: one already sent waits for its answer, and one due
  while initiation is disabled goes out as usual. That covers Audit Reporter
  records, immediate or batched by `Maximum_Send_Delay`, their
  AUDITING_FAILURE summaries and Audit Log forwarding. Only real delivery
  moves a Reporter's health, and a record dropped for want of a send slot is
  summarized as usual. Writes whose commit owes a notification, such as a
  Device's `Audit_Notification_Recipient`, a Reporter's own properties or
  `Send_Now`, and an object's mandatory audit policy, are accepted under
  DISABLE_INITIATION and report like any other.

With target Audit configured, each DCC change the server carries out is itself
audited (Table 19-5, #1387): DEVICE_DISABLE_COMM for an accepted
DISABLE_INITIATION, sent under the state it reports, and DEVICE_ENABLE_COMM for
an accepted ENABLE or a timed disable that runs out. A refused request writes no
record. See [Audit records](dcc-policy.md#audit-records).

A write a Command or Channel makes in another device follows the same rule
(see [Building Control](#building-control-7)).

### Discovery under DeviceCommunicationControl

During DISABLE_INITIATION the full server answers matching Who-Has requests
with I-Have, alongside its existing Who-Is/I-Am responses (#1590). This adopts
the optional interoperability allowance described in
[ASHRAE IC 135-2020-22](https://www.ashrae.org/file%20library/technical%20resources/standards%20and%20guidelines/standards%20intepretations/ic-135-2020-22.pdf).
Object names, identifiers and device-instance ranges still determine whether
a Who-Has matches. Replies use the existing routing and discovery limiter:
an admitted I-Have consumes response budget and establishes a coalescing
entry, which survives an ENABLE request or DCC timer expiry.

Independent I-Am announcements remain restricted (#1388). Calling
`broadcast_i_am()` or an `IAmBroadcaster` during DISABLE_INITIATION sends
nothing and returns `SERVICES` / `COMMUNICATION_DISABLED`. The solicited
I-Have allowance does not enable unsolicited announcements. Network-layer
messages, such as a Network-Number-Is answer, are not application services
and go out as usual.

### Notification forwarding

A `NotificationForwarderObject` (type 51, Clause 12.51) originates no events.
The server offers it every ConfirmedEventNotification and
UnconfirmedEventNotification it receives, and every notification one of its
own objects sends to a Notification Class recipient naming the server's own
Device object, with that recipient's process identifier. A forwarder takes a
notification when it is in service, its `Process_Identifier_Filter` is NULL
or equals the notification's process identifier, its `Local_Forwarding_Only`
is FALSE or the notification is the device's own, and, for a received
notification, its `Port_Filter` (absent unless configured with
`set_port_filter`) enables Port_ID 0, the server's one port. It sends a copy to
each `Recipient_List` destination whose days, times and transitions admit the
notification and to each live `Subscribed_Recipients` entry, confirmed or not
as the destination asks. A forwarder that cannot serve
`Process_Identifier_Filter` or `Local_Forwarding_Only` as Clause 12.51 types
them takes nothing, and a `Recipient_List` or `Subscribed_Recipients` that does
not decode, or runs past its cap, gives no destinations. A copy differs from
the received notification only in its process identifier: the rest goes on
octet for octet, whatever the character set of its message text.
`ForwardedEventNotification::decode` checks the request's structure without
decoding the text or event values, and refuses anything after the event values. Copies go through the same send path, route
skips and counters as the server's own notifications. Notifications are never
sent segmented, so a copy longer than the local APDU capacity, such as one of a
notification that arrived segmented, is not sent to that destination and counts
in `apdu_too_large`; the other destinations are still served.

No copy goes by global broadcast, a notification received by global broadcast
(`ReceivedApdu::global_broadcast`) is not forwarded, a received notification is
not broadcast back onto the local network, and one received by broadcast goes
to no node on the local network. The server ignores every confirmed request
that arrives by broadcast or multicast (`ReceivedApdu::is_group`), whatever
its service, with no answer (Clause 5.4.5.1), so a ConfirmedEventNotification
it answers was addressed to it alone. Once the server knows the local
network's number, configured on the registered Network Port
(`ServerBuilder::registered_network_port`) or learned from Network-Number-Is
with or without one (#1298), a recipient naming that number is on the local
network to these rules, and a copy they let through goes to it as a local
recipient, with no DNET. While the number is unknown, such an address is taken
as remote. These skips are configured behaviour and move no counter.
DeviceCommunicationControl's DISABLE_INITIATION stops every copy.
A destination naming the server's own Device object hands the copy to the
forwarders that have not yet taken it, and across such a chain each
destination (recipient, process identifier and confirmation) gets one copy.

One notification goes to at most `MAX_FORWARDED_DESTINATIONS` (64, a full
`Recipient_List` and `Subscribed_Recipients` of one forwarder) destinations
across every forwarder that takes it, chained ones included, in forwarder then
list order. Only copies that would be sent count: a destination the loop
rules refuse, whose route is skipped or whose copy is too large takes no room,
and neither does a destination naming the server's own Device object. A local
notification whose Notification Class names that Device object under several
process identifiers is one notification, with one cap and one copy per
destination. The destinations past the cap are dropped and counted in
`forwarding_cap_dropped`; a capped notification logs a warning at most once a
minute per process, and at debug level in between. There is no rate limit on
received notifications.

The server executes ConfirmedEventNotification, so it acknowledges every
well-formed one once it decodes, before any copy is sent and whatever
forwarding then finds, including one no forwarder takes. A received
notification that no forwarder takes counts in `received_not_forwarded`. One
that does not decode is rejected with the reason naming its fault, as the
client rejects one (#1446).

The server remembers each ConfirmedEventNotification that decodes for 60
seconds, at most 256 at once with the oldest dropped first, keyed by source
address and invoke ID and matched only by the same service-request octets and
only while its 60 seconds last. A
retransmission it matches, sent because the acknowledgment went missing, is
acknowledged again but not offered to the forwarders again, and does not count
in `received_not_forwarded`. One that comes later, or after newer
notifications pushed its entry out, is forwarded again. The record is in
memory only.

`with_persistence` keeps `Recipient_List` and `Subscribed_Recipients` in an
application-owned `NotificationForwarderPersistence`, saving both lists as one
`ForwarderSnapshot` when a write changes either, when a subscription lapses,
and at most once a minute while the subscriptions' minutes fall, and restoring
them when the forwarder is built again (Clauses 12.51.8 and 12.51.9). A
restored subscription so carries at most about a minute more than it had
left, and repeated restarts still run it out.
`FileNotificationForwarderPersistence` keeps both lists in one file, replaced
whole through a synchronized temporary file, a rename and a synchronized
directory (on Unix; Windows skips the directory step).

Saves run on the forwarder's own writer thread, one at a time. The bundled
server waits for them with the object database guard dropped, except where
noted below; application code writing a list through the database waits for
the save in place. The bundled server stages each network or
`write_local` list write: the forwarder queues the save, the server waits for
it with the guard dropped (on the blocking pool, as for the Audit Log), and the
write then takes the saved list. A write
that cannot be saved fails with DEVICE / OPERATIONAL_PROBLEM and leaves the old
list. WritePropertyMultiple stages too, its writes to both lists as one save
(#1423); under a `mutation_authorizer` the server decides each such attempt
before staging it and stages only those allowed (#1321). The operation task's
lapse and minute saves coalesce, so a burst
costs one save of the latest lists; one that fails is logged and retried a
minute later. `save_counters()` returns a `ForwarderSaveCounters` handle,
shared with the object, whose `failed_saves()` counts every refused save; take
it before adding the object to the database. `wait_for_saves()` blocks until
queued saves have run, and dropping the forwarder waits for them too, so the
server's DeleteObject drops a removed forwarder on a blocking thread after
releasing the guard. A staged write its request never makes (an earlier
WritePropertyMultiple attempt failed, say) is dropped, and the forwarder at
once queues a save of the lists it serves, so storage never keeps a list the
forwarder refused. The same holds when the server stops mid-request (#1363):
`stop()`, after joining its requests, drops a staged write still held and
waits for that save, and a forwarder dropped with one still held saves the
lists it serves before its writer stops. A drop without `stop()` saves the
lists as they stood at the forwarder's last operation-task call, so an entry
that lapsed within that last second can come back with a minute left;
`stop()` saves them as they stand. The writer is a plain `std` thread with no
Tokio runtime, one per forwarder that has saved and parked while idle, and a
`save` that panics counts as a failed save. Once its rename succeeds a
file save has landed: a filesystem that cannot synchronize a directory is
passed over, and any other failure there is logged, not returned.

A written `Recipient_List` wins over the configured one. `ForwarderSnapshot`
holds `recipient_list: None` until a write sets the list; the destinations the
application configures with `add_destination` are never saved and apply at
every start until then. Once a write has set the list and it was saved, a
rebuilt forwarder serves the saved list (`recipient_list_saved()` is true) and
`add_destination` checks a destination but does not add it. To return to the
configured list, clear the storage.

### Undelivered event notification counters

`BACnetServer::event_notification_counters()` returns an
`EventNotificationCounters` snapshot: lifetime totals of event notifications the
server did not deliver, each saturating at `u64::MAX`, zero for a new server and
still readable after `stop()`. Fields are sampled independently.

```rust
let counters = server.event_notification_counters();
counters.notification_class_missing;    // no Notification Class with that number
counters.recipient_list_unavailable;    // its Recipient_List could not be read
counters.recipient_list_invalid;        // the list did not decode as a whole
counters.recipient_list_too_long;       // a custom class served more than 32 destinations
counters.device_recipient_unbound;      // a Device recipient with no current binding
counters.recipient_unroutable;          // a recipient no binding or retry can route
counters.confirmed_broadcast_recipient; // confirmed requested at a broadcast or group address
counters.confirmed_no_invoke_id;        // no invoke ID free for a confirmed notification
counters.confirmed_rejected;            // the recipient answered Error, Reject or Abort
counters.confirmed_unanswered;          // no acknowledgment after the last retry
counters.unconfirmed_send_failed;       // an unconfirmed send the transport refused
counters.apdu_too_large;                // a notification longer than the local APDU size
counters.received_not_forwarded;        // a received notification no forwarder took
counters.forwarding_cap_dropped;        // a destination past the forwarding cap
counters.received_not_logged;           // a received notification past its source's allowance
```

The four recipient-list fields count transitions, event and acknowledgment
notifications alike, whose Notification Class lookup failed closed: one per
`RecipientLookupOutcome` that suppresses delivery, alongside the warning each
one logs. `NoConfiguredDestinations` and `NoMatchingDestinations` are
configured behaviour and are not counted, nor are notifications held back by
DCC or Event_Enable, a confirmed one DCC ends at a retry included. The three
confirmed fields count notifications to one
recipient; a reservation refused because the server is stopping is not
counted.

`unconfirmed_send_failed` (#1196) counts unconfirmed notifications whose send
returned a transport error, once per destination, and the transition's other
destinations are still served. A confirmed send that fails locally counts in
`confirmed_unanswered`. No field counts an encode failure: the committed
payload and message text are validated before the destinations are walked, so
a well-formed transition always encodes.

`apdu_too_large` (#1225) counts notifications not sent to one destination
because their APDU is longer than the local APDU capacity; notifications are
never sent segmented, and the other destinations are still served. It is
mostly a forwarded copy of a notification that arrived segmented.
`received_not_forwarded` counts received event notifications that decoded but
that no Notification Forwarder took, and `forwarding_cap_dropped` (#1259) the
destinations a notification was not forwarded to because it already had
`MAX_FORWARDED_DESTINATIONS` across the forwarders, one per destination (see
[Notification forwarding](#notification-forwarding)).

The three route fields (#1160) count destinations that matched the transition
but were skipped while their route was resolved, once per destination; the
transition's other destinations are still served. They are grouped by what
fixes them, and the warning logged with each skip gives the finer reason:

- `device_recipient_unbound`: no Device binding was configured or observed, or
  the observed one expired, and a look for the device found none. Before
  skipping such a recipient the server looks for its device as it does for a
  Command's remote write (#1368): one Who-Is limited to its instance, at most
  one a minute per device, none under DCC, and the notification waits up to
  `cov_retry_timeout_ms` (a minute at most) from that Who-Is for the I-Am.
  It waits in a queue for its device, which a task of its own drains, so the
  transition's other recipients aren't held up; while the queue holds
  anything, the device's later notifications join it, and each device drains
  on its own. Notifications the server makes for one device one after
  another reach it in that order, across the wait too. Notifications made at
  once from different tasks have no order between them, and a confirmed
  notification's attempts run in a transaction task of their own, so two
  confirmed ones may start in either order. A device that answers gets the
  notification, which then counts as any other send does, DCC checked again
  before each send; one that stays silent, or can't be looked for within the
  minute after a fruitless Who-Is, counts here once. At most 1,024
  notifications wait at once across every device; one more is skipped and
  counts here at once, with no Who-Is. Observing the device's I-Am again, or
  configuring a binding, clears it. A confirmed notification whose observed
  binding expires before a retry ends at that retry, its invoke ID freed, and
  counts here, not in `confirmed_unanswered` (#1371).
- `recipient_unroutable`: the entry can't be routed as written. Its Device
  identifier names an object that isn't a Device (or its binding is unusable on
  this link), or its address puts a MAC on network 65535.
- `confirmed_broadcast_recipient`: the entry asks for confirmed notifications at
  a local, remote or global broadcast address, or at any other group address
  of the link, such as a B/IP multicast address or the broadcast IP at
  another port (#1493). Clause 6.3 allows only unconfirmed requests there,
  and sending one unconfirmed would lose the acknowledgment, so the entry is
  skipped before any invoke ID is reserved. An unconfirmed notification still
  goes to such an address.

### Concurrency

- Lock ordering: always `db` before `cov_table`, and `db` before the device
  binding table, which a read of Device_Address_Binding samples after the
  COV table's guard is gone (#1369); no binding-table guard is held across
  a send or a wait
- `seg_receivers` capped at 128 (DoS prevention)
- `cov_in_flight` semaphore: max 255 concurrent confirmed COV notifications
- Targeted Who-Is probes for unbound devices: at most 256 devices tracked,
  one Who-Is a minute per device, and a wait for the I-Am of
  `cov_retry_timeout_ms` held to a minute (#1322, #1368)
- Event notifications waiting for their Device recipient's I-Am: at most
  1,024 at once, across every device (#1368)
- `comm_state`: `Arc<CommState>`, a lock-free DCC state that only the DCC
  timer (an accepted request and its expiry) changes

---

## Error Handling

All async operations return `Result<T, bacnet_types::error::Error>`. Key variants:

| Variant | Meaning |
|---------|---------|
| `Error::Protocol { class, code }` | Remote BACnet error response |
| `Error::Structured { class, code, detail }` | Remote error whose Clause 21 body adds fields (ChangeList-Error, CreateObject-Error, WritePropertyMultiple-Error and others): `detail` is the boxed `ErrorDetail` |
| `Error::Timeout(msg)` | APDU retry exhausted |
| `Error::Reject { reason }` | Remote device rejected request |
| `Error::Abort { reason }` | Remote device aborted request |
| `Error::RoutedPathTooLong { dnet }` | Router rejected the active message as too long for DNET |
| `Error::RoutedPathCapacityExceeded { capacity }` | No routed-path entry can be allocated without discarding protected safety state |
| `Error::UnsupportedTransport { required, actual }` | The client's data link cannot carry the operation (a BBMD helper on a non-B/IP transport) |
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

#### SC Device UUID migration

`ScServerBuilder::device_uuid([u8; 16])` is now required at runtime: `build()`
returns `Error::Encoding` for omitted/all-zero identity before dialing. Reconnect
configuration is still checked first; the existing TLS/binding/budget checks
retain their relative order. This is an intentional runtime compatibility break
for SC server callers. The Rust SC client already requires a nonzero UUID; its
identity and VMAC policy are unchanged.

The application must generate the UUID before first deployment and durably store
and reuse exactly the same bytes throughout the device's lifetime (base 2020
AB.1.5.3). Pass that stored value each time you build the node; built-in reconnect
reuses it. There is no runtime generation, guessed storage location, persistence
backend, UUID version/variant enforcement, or lifetime-immutability guarantee.
Without application storage/history the library cannot detect a changed UUID.

Distinct devices need distinct UUIDs, independently of their Device instance and
VMAC. Known UUIDs retain the existing intended connection replacement behavior
(AB.6.2.3); two connections sharing one UUID are not expected to coexist. Examples
of **test-only** distinct values are `8e62ac46-d708-4226-9137-76a32b619315` for a
server and `95dfe4ef-97f6-490d-9a2c-f2b4b0c0e682` for a client. Do not deploy these
shared demo identities; supply your own provisioned arrays. The hub also requires
its hosting device's lifetime UUID: see [hub identity migration](#bacnetsc-hub).
Raw `ScTransport` now has the [startup guard](#bacnetsc-client-transport) above;
remote-peer VMAC rules remain unchanged. This does not move
higher-level builder checks or promise that every local VMAC is rejected before
dialing. The owner-approved #517 acceptance closeout
resolves the scoped default/nil identity problem under these boundaries, not all
low-level public paths or RFC bit-profile/lifetime enforcement; no PICS/profile promotion.

**Receiving hub compatibility break:** a received Connect-Request with an all-zero
Device UUID now fails after TLS/WebSocket establishment and before admission,
activity refresh or replacement. The existing eligible NAK is
`COMMUNICATION/PARAMETER_OUT_OF_RANGE` (7/80), marker zero, not Duplicate-VMAC;
reply addressing uses the envelope source and existing broadcast/reserved-source
suppression. New malformed peers close; malformed repeats retain registration,
negotiated limits and heartbeat/activity state. Legacy raw peers must supply a
nonzero UUID. This local policy treats nonzero bits as opaque, including sparse
or non-RFC-shaped values; generic encoding/decoding and manual raw sending still
permit nil syntax. Initiating nodes silently discard Connect-Accept with a zero
UUID after TLS/WebSocket setup. AB.2 forbids replies to response messages: the
internal range classification is not a wire NAK. Rejection leaves pending state,
peer identity/limits and local identity unchanged and does not restart the
absolute connect wait. A later valid Accept can complete the same handshake;
nil-only traffic times out. Invalid-plus-wrong-ID Accepts are discarded, while
otherwise-valid wrong-ID Accepts retain the terminal mismatch error. Failed
restoration probes do not replace the active failover or reseed the local VMAC.
This receive-shape policy adds no UUID version/variant, generation or storage
requirements; optional [Hub certificate bindings](#hub-certificate-bindings) are
configured separately.

**Zero-limit receive policy (Refs #519):** the shared Connect validator
rejects zero Max-BVLC or Max-NPDU in either received Connect message, after the
existing envelope/length/identity checks and before MU diagnostics. This is
**zero-only local policy**, not a universal minimum-capacity conformance claim.
Eligible Requests receive `COMMUNICATION/PARAMETER_OUT_OF_RANGE` (7/80) with the
existing addressing/suppression rules, before activity, admission, capacity or
UUID replacement. Accepts are silently discarded under AB.2: no NAK, pending or
peer-limit commit, Connected publication, or original connect-deadline reset.
Later valid Accepts recover; failed reconnect/restoration probes do not poison
active limits or retire a good failover peer. Local UUID/VMAC are not reseeded.
All positive values remain compatible, including 1/1, 65535/65535, 1200/480 and
300/1476. These are policy boundaries, not proof that tiny capacities can carry
useful services. No positive floor or Max-NPDU/Max-BVLC relationship is imposed.
Local defaults, adapter caps and independent per-peer outgoing budgets remain
unchanged; generic codecs/constructors and manual raw sending still permit zero
syntax. Post-start public mutation is outside this receive guard. #519 remains
open/partial; closed #517 identity acceptance and lifetime exclusions remain valid.

```rust
use bacnet_client::client::BACnetClient;
use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};

// Start the hub with already loaded site CA, hub chain, and matching key DER.
let hub_tls = ScHubTlsConfig::from_der(ca_certs, hub_cert_chain, hub_key)?;
let mut hub = ScHub::start_with_tls_config(
    listen_addr, hub_tls, [0xFF, 0, 0, 0, 0, 1], hub_uuid,
    ScHubHandshakeTimeouts::default(),
).await?;
let hub_addr = hub.local_addr().expect("started hub has a bound address");

// Build the node policy from separately loaded site trust and operational DER.
let tls_config = bacnet_transport::sc_tls::ScNodeTlsConfig::from_der(
    node_ca_certs, node_cert_chain, node_key,
)?;
// Use a persistent nonzero client_uuid, distinct from hub_uuid and every other peer.
let mut client = BACnetClient::sc_builder()
    .hub_url(&format!("wss://127.0.0.1:{}", hub_addr.port()))
    .tls_config(tls_config)
    .vmac([0, 1, 2, 3, 4, 5])
    .device_uuid(client_uuid)
    .build()
    .await?;
// ... use the client ...
client.stop().await?;
hub.stop().await;
```

### MS/TP with USB Adapter

```rust
use bacnet_transport::mstp::MstpTransport;
use bacnet_transport::mstp_serial::{TokioSerialPort, SerialConfig};

let serial = TokioSerialPort::open(&SerialConfig {
    port_name: "/dev/ttyUSB0".into(),
    baud_rate: 76800,
})?;

let client = BACnetClient::generic_builder()
    .transport(MstpTransport::new(serial, 1, 127))
    .build()
    .await?;
```

### MS/TP with Raspberry Pi RS-485 Hat (GPIO)

```rust
use bacnet_transport::mstp::MstpTransport;
use bacnet_transport::mstp_serial::{GpioDirectionPort, TokioSerialPort, SerialConfig};

let serial = TokioSerialPort::open(&SerialConfig {
    port_name: "/dev/ttyS0".into(),
    baud_rate: 76800,
})?;

// Seeed Studio RS-485 Shield: GPIO18 for DE/RE, active-high
let port = GpioDirectionPort::new(serial, "/dev/gpiochip0", 18, true)?;

let client = BACnetClient::generic_builder()
    .transport(MstpTransport::new(port, 1, 127))
    .build()
    .await?;
```

---

### Configured Network Port snapshots

`NetworkPortObject::new_bip(instance, name, BipPortConfig)` constructs a complete,
unbound flat IPV4 application configuration, which reads as NORMAL until a
registered owner publishes another B/IP mode (see below). Instance is the declared local
Port ID (local policy: 1–255), separate from UDP port zero. `BipPortConfig` carries
fixed four-octet IP/mask/gateway values, a nonempty DNS array, network number
0–65534, and APDU_Length399 >=50. Defaults are unknown zero addresses/mask/gateway,
one zero DNS address, UDP47808 and declared port capacity1476. Device62 and its
discrete APDU sizes remain independent; neither constructor discovers a NIC or
binds a transport.

The snapshot denies activation-dependent network writes and derives its readonly
MAC from configured IP/UDP. Reconstruct it to change configuration. It has no
pending activation, inert Command, or obsolete port62 projection. Link_Speed is
optional and zero means unknown. The former raw `new` constructor and configuration
setters are removed. `new_non_bip` takes explicit Ethernet/VIRTUAL number, MAC and
capacity and exposes common application rows only; it does not claim a complete
SC or Ethernet profile. Both `DeviceIdentity` database builders use these same
constructors with declared1476 port capacity independent of Device/role limits.
Live transport association, post-bind synchronization and activation are separate.

### Registered B/IP Network Port

This receiving-port association is a bounded single B/IP profile, in whichever B/IP mode the owned transport runs (#939). Local Network Number behavior is described below; complete Network Port conformance and multiport routing remain outside this registration contract.

A transport reports its registration capability through `TransportPort::bip_port()`, which returns a `bacnet_transport::port::BipPort`: the endpoint (configured before start, announced after) and a `bacnet_types::bip_port::BipPortMode`. Startup publishes that mode with the bind, and the port's BACnet_IP_Mode reads it: NORMAL, FOREIGN or BBMD. Property_List and the property metadata follow the mode (Clause 12.56, Table 12-71 footnotes 11 to 13):

- **FOREIGN** adds FD_BBMD_Address (a `BACnetHostNPort`) and FD_Subscription_Lifetime, from the transport's `ForeignDeviceConfig`, which can't change while the server owns it.
- **BBMD** adds BBMD_Broadcast_Distribution_Table and BBMD_Foreign_Device_Table (BACnetLISTs of `BACnetBDTEntry` and `BACnetFDTEntry`) and BBMD_Accept_FD_Registrations. The object holds the transport's own BBMD state through the `bacnet_types::bip_port::BbmdTables` view and reads it on every property read, so a BDT change, a new or expired registration, or a policy change made through `BipTransport::bbmd_state()` shows on the next read. FDT entries carry each registrant's time to live and the seconds it has left, grace period included; an entry past its time no longer appears even before the purge task removes it. The BDT includes the BBMD's own row.
- **NORMAL** has none of them: reading one answers UNKNOWN_PROPERTY.

All five are read-only for now, and the PICS draft lists them as not writable. Clauses 12.56.34, 12.56.35, 12.56.37 and 12.56.38 make the BDT, Accept_FD_Registrations and both FD properties writable, with a write setting Changes_Pending until ReinitializeDevice activates it; until that activation path exists, a write answers WRITE_ACCESS_DENIED, as every configuration row of this object does. A transport configured both as a BBMD and as a foreign device has no single mode, and starting with a port registered on it fails with an error that says so. After the owner stops, the port keeps reporting the last published mode, as it keeps the last published bind; a BBMD's rows then show the stopped transport's tables as they were left, until a new registration. NAT traversal (BACnet_IP_NAT_Traversal, BACnet_IP_Global_Address) and B/IP multicast (BACnet_IP_Multicast_Address) are not modeled yet.

The BBMD state lock is a synchronous `std::sync::Mutex` (it was a Tokio mutex before #939): the transport and the Network Port view hold it only for short, synchronous critical sections, so the object can read it under the database read lock without awaiting. Lock order stays ObjectDatabase, then the BBMD state; the transport never takes the database.

### Local Network Number controls

Three owners consume the two local nonrouter controls automatically, one per transport: the full server, the shared endpoint and the standalone client. The full server and the standalone client cover B/IP in NORMAL, BBMD and configured foreign-device modes, B/IPv6 in normal and configured foreign-device modes, BACnet/SC, MS/TP and Linux Ethernet. The shared endpoint covers the three B/IP modes, BACnet/SC and MS/TP; there is no B/IPv6 or Ethernet endpoint builder. Every built-in data link transport opts in through `TransportPort::supports_local_nonrouter_number_controls`, and each owner keys only on that capability, with no per-transport branch. Only the in-process `LoopbackTransport`, which attaches an application to a router port, keeps the false default. A valid local unicast or broadcast What-Is-Network-Number receives a local-broadcast Network-Number-Is when the owner knows its number (Clauses 6.4.19–6.4.20). There is no proactive startup announcement: the Standard asks that of configured routers, not of every nonrouter node. An explicit registered Network Port with a nonzero configured number reports `CONFIGURED` and never replaces that number from an announcement. Zero starts `UNKNOWN`; an owner without registration also starts unknown, regardless of other declared objects.

A valid local-broadcast announcement with flag zero updates an unknown/learned owner to `LEARNED`. Flag one sets `LEARNED_CONFIGURED` and takes precedence over all subsequent flag-zero announcements. Further flag-one announcements may replace that learned value, including conflicts; an equal value still upgrades its quality. Both learned qualities transmit flag zero in their own responses. Selected-object `Network_Number` and `Network_Number_Quality` reads use the same database-owned state. Configuration remains immutable, so a new registration resets the pair from configured provenance; a new unregistered runtime starts unknown. There is no persistence of learned state across constructing a new runtime. After stop, the object retains the last observed pair until reconstruction or a new registration.

Routed controls, malformed payloads and unicast Network-Number-Is are ignored. A BBMD Forwarded-NPDU is a logical broadcast even when its UDP hop is unicast and remains eligible. Ignoring number zero, 65535 and flags outside zero/one is this implementation's validation policy, rather than an additional quoted Standard mandate. Conflicting announcements against a locally configured number produce a debug diagnostic without changing configuration.

The full server publishes the number its owner holds on `NetworkLayer::local_network_number` (a `bacnet_network::network_number::LocalNetworkNumber` handle), where event routing, writes in other devices and Audit delivery read it without the database lock (#1298, #1358). With a registered Network Port the port stays the one authority: startup copies the port's number there once the bind is published, and the worker copies the port's state after each control under the same database write lock that changed it, so the handle never holds a number the port does not. Without a registered port, the handle holds the number the worker learned. A later announcement that replaces the number takes effect for the next notification or write sent. Nothing withdraws a known number, since no announcement can, so it stays until a new runtime starts again from the configured number or unknown. The standalone client publishes its learned number on its own layer the same way, where its routed requests and network broadcasts read it (#1358); neither exposes its layer. Every sender that reads the number sends a destination naming it as local traffic, with no DNET, and keys a confirmed request to the MAC it went to, since the answer comes from there with no SNET. `NetworkLayer`'s own send methods frame the destination they are given and never rewrite it, so answers to a request, and COV notifications to a subscriber, keep the route the request arrived by. The shared endpoint session's owner publishes on its own layer in the same way, a registered port's number from startup, and the endpoint's requester and source Audit routes read it (#1403); its answers also keep their route.

Standalone clients start UNKNOWN on transports that opt into local nonrouter Number controls. They learn and reply using the same validation and precedence rules, without a Device object, registered Network Port, configured-number setter or persistence. One 256-entry serial worker owns this state; full or closed admission drops only Number controls. A held Number send leaves routed reason-4 Reject correlation and independent APDU dispatch available. Stop aborts and joins both the Number worker and dispatch before transport cleanup, retaining their joins across a canceled stop waiter. Drop aborts both. Already transmitted bytes cannot be retracted. Controlled-client tests qualify the shared intake/lifecycle behavior; Linux NORMAL-B/IP loopback and Ethernet virtual-link tests independently observe actual reply frames. Constrained-TLS SC tests observe Hub broadcast VMAC and exact Number bytes, including replies to direct-peer queries, while ordinary confirmed client requests complete. SC stop/drop retires client connections; the external DirectListener must separately be stopped and joined before its bind is released. Pending-send/queue cancellation remains covered by the generic controlled-client tests, rather than inferred from wire silence. Rust standalone-client B/IPv6 tests independently capture normal selected-link OriginalBroadcast and configured-foreign DBTN with exact source, destination, interface and Number bytes. Positive reply fences cover UNKNOWN, precedence and invalid/admission refusal; stop and eventual Drop release the socket, and reconstruction starts UNKNOWN. These external ignored Linux tests require the integration `ipv6` feature and isolated observer; ordinary hosted CI does not execute them. They add no Python foreign-device API, configured-client authority or physical-LAN claim. Separate isolated Linux standalone-client BBMD/foreign tests capture own Original-Broadcast versus forwarding traffic and exact DBTN to the configured BBMD. Positive Number fences cover UNKNOWN, BDT/FDT admission/refusal, alternate-sender compatibility, precedence and representative invalid/routed controls; registration NAKs retain DBTN attempts and the timer retries registration. An ordinary client ReadProperty completes during live Number controls, and awaited stop permits exclusive socket rebind before client Drop. No configured client number, new registration policy or complete Annex J claim is added. MS/TP LoopbackSerial tests also cover the standalone client in both execution modes, with the same frame decoding and fences as the full server and shared endpoint below; its own ReadProperty to the peer completes while a Number send is held.

The pre-1.0 Rust helper moved directly from `bacnet_objects::network_port::NetworkNumber` to `bacnet_types::network_number::NetworkNumber`, with no compatibility alias. Its default is UNKNOWN; `configured(number)` returns `None` for reserved 65535. Pure observation reports configured conflicts to the caller for logging. Shared nonrouter packet parsing and reply encoding live in `bacnet_network::network_number`; registered database authority remains in the server/object adapter.

Transport wrappers must delegate `TransportPort::supports_local_nonrouter_number_controls` when preserving these semantics; the default is false. This capability is independent of B/IP registration and conveys no configured-number or control-origin authority.

SC starts UNKNOWN with no configured SC Network Port API; unrelated configured objects provide no authority. SC logical broadcast is the BVLC broadcast destination VMAC. A direct unicast What-Is is valid, but its Number reply uses the Hub broadcast path, never the saved original-direct APDU response capability. Hub-relayed controls do not identify an originating TLS leaf; SC control-origin authorization remains separate (#518). A message from a direct-connection peer is never a logical broadcast, so it can ask but cannot teach.

B/IP BBMD and configured foreign modes start UNKNOWN unless a Network Port is registered for them, which then supplies configured authority as in NORMAL mode. BBMD mode learns admitted Original-Broadcast, BDT Forwarded-NPDU and registered foreign-device DBTN announcements; its own Number reply is a local Original-Broadcast, which it also forwards as a Forwarded-NPDU to its BDT peers and registered foreign devices like its other broadcasts (#937). It rejects an exact self UDP source tuple before forwarded delivery or fanout, preserving the self BDT row and admitted peers on the same IP at different ports. Configured foreign mode accepts structurally valid Forwarded-NPDU from alternate UDP senders under its existing compatibility policy and answers by DBTN to its configured BBMD. Logical broadcast conveys no authenticated origin. Registration rejection does not suppress DBTN attempts; the existing periodic registration loop continues. Shared-endpoint Number wire tests cover BBMD ServerOnly admission and Original-Broadcast replies, foreign ClientOnly alternate forwarding and DBTN through registration rejection/retry, and Both requester/responder progress during live controls plus stop/drop socket release. Linux loopback supplies the independent BBMD broadcast capture; foreign direct capture also runs on macOS. Existing controlled endpoint tests separately prove held-send cancellation and resumed stop. Broader shared-endpoint BBMD/foreign behavior remains experimental; only a registered Network Port adds configured authority in these modes.

B/IPv6 starts UNKNOWN with no configured IPv6 Network Port authority. Normal mode learns admitted OriginalBroadcast announcements and answers by multicast OriginalBroadcast on the selected link. In the Rust configured foreign-device mode, an admitted Forwarded-NPDU from the configured BBMD is a logical broadcast despite its unicast UDP hop; replies use DBTN to that BBMD. A different BBMD endpoint cannot teach. Unicast NNI, routed controls and malformed payloads remain ineligible. Existing selected-link, source-address, destination/interface and VMAC checks still apply. There is no new IPv6 endpoint builder, number setter or Python foreign-device API.

Linux Ethernet full servers and standalone Rust clients start UNKNOWN with no configured Ethernet Network Port authority. The AF_PACKET receive path admits only the bound MAC and all-FF broadcast, before UI, XID or TEST handling; of those, only all-FF is a logical group. As a destination, though, every MAC with the group bit (the low bit of its first octet) set is a group destination (`bacnet_transport::ethernet::is_group_mac`), so no confirmed request goes to a multicast MAC either (#1493). No station sends from a group address, so a frame whose source MAC has the group bit set is dropped before the XID and TEST handlers and before decoding, and counted in `EthernetTransport::group_source_drops()` (#1492): otherwise an answer to it, or to a confirmed request in it, would go to the whole group. This is the local single-link admission policy, not an additional quoted Clause 7 mandate. Existing self-source refusal remains. Actual isolated Docker Ethernet tests independently inspect destination/source, 802.3 length, LLC bytes, exact learned reply flag zero and padding, and verify stop/drop raw-FD release plus canceled transport-stop resumption. No privileged host interface or physical LAN is involved. The [opt-in fixture](../crates/bacnet-integration-tests/tests/ethernet_network_numbers/README.md) requires Linux and CAP_NET_RAW and is excluded from ordinary CI. There is no Ethernet endpoint builder or Python Ethernet API in this slice.

MS/TP starts UNKNOWN with no configured MS/TP Network Port API. The full server, shared endpoint and standalone client learn only broadcast DataNotExpectingReply announcements; either a local unicast or broadcast query receives a DataNotExpectingReply to station `0xFF` after token opportunity. Both Tokio and DedicatedThread modes use the existing serial/MAC owner. LoopbackSerial tests run all three owners in both modes. They independently decode complete standard frames, check precedence and invalid-control recovery with a later valid response as an ordering fence, and qualify application progress while the Number producer is held: the server and endpoint answer a peer's ReadProperty, and the client's own ReadProperty to the peer completes. Separate post-enqueue and serial-write gates prove canceled stop and drop release that producer without completing a held Number frame. Queue admission alone is not frame transmission; already completed bytes cannot be retracted. This simulator evidence does not qualify physical RS-485 timing, transceiver control or hardware interoperability.

Control work has its own 256-entry receiver and serial worker, so a blocked learning operation or SC Hub write does not stop incoming APDU handling or already-admitted Audit acknowledgment dispatch. A blocked socket writer still serializes physical egress; this does not promise a second concurrent send. Stop seals, aborts and joins that worker before transport cleanup; cancellation retains cleanup ownership. Queued endpoint control sends are caller-owned and canceled with the worker, while a send already started may have reached the wire. The registered-object lease remains with the final socket and admitted work.

Tests cover actual inbound BVLL controls through both B/IP owners and outgoing NPDU observation after a successful real broadcast send, and separate tests cover the unchanged BVLL framing layer. The NORMAL B/IP owner fixture does not capture outgoing BVLL frames. Separate full-server BBMD/foreign tests independently decode actual loopback UDP: Linux BBMD tests distinguish own Original-Broadcast replies from Forwarded-NPDU fanout, and cross-platform foreign tests capture DBTN directly. Positive response fences cover refusal and recovery; held producer tests qualify APDU progress, canceled stop, drop and socket release. This loopback evidence does not qualify a physical LAN. Separate constrained-TLS SC fixtures independently decode actual broadcast-VMAC NNI bytes through both owners and `AnyTransport`; deterministic single-writer socket gates qualify bounded control queues, ACK/handler progress, cancellation and joined teardown. Already-admitted detached ordinary APDU sends retain their existing ownership; canceling Number work is not their retraction. Separate opt-in Linux tests capture full-server normal multicast and foreign DBTN bytes with an independent raw observer/BBMD, and a fresh installed Python extension qualifies normal multicast intake/output on the same isolated topology. These external-network tests are distinct from ordinary CI coverage. These controls do not establish a complete Network Port, Annex U/AB or router profile.


A configured object becomes the receiving port only through explicit selection:
`ServerConfig.registered_network_port = Some(oid)`, the server builder's
`.registered_network_port(oid)`, or the B/IP endpoint builder's method of the same
name (`EndpointSession::with_registered_network_port` for direct composition).
The selected built-in IPV4 object must already exist with instance 1–255,
matching concrete unicast interface and configured UDP port. Port zero is valid
before bind. A NORMAL, foreign-device or BBMD transport may register; a
wildcard interface, a BBMD that also registers as a foreign device, and
non-B/IP links are rejected before publication; declarations alone remain
unregistered.
Custom objects and wrappers cannot impersonate a selected built-in: the database
uses crate-authorized concrete storage access before configuration callbacks.
Borrowed Device read views remain supported; no public mutable downcast is exposed.

Startup validates the actual B/IP capability again after bind and reconciles
only the selected object and optional identity entry with the announced IP, actual
UDP port, derived MAC and B/IP mode; a BBMD must have created its tables by then. Port `APDU_Length` (399) is independently supported at 1476;
Device `Max_APDU_Length_Accepted` (62) may remain 480. Mask, gateway and DNS remain explicit configuration, with
no NIC discovery or fabricated subnet. The obsolete identity `sync_bip_bind`
setter is removed. Configuration/activation writes remain denied, and registered
Out_Of_Service writes, object replacement/removal and adapters are refused before
effects. Protection lasts through admitted work and the final socket/cleanup
owner, including cancellation and Drop; an idle exported role is not a lease.

Full-server RP/RPM and bounded endpoint RP resolve Network-Port instance 4194303
using this owner's selected identity, with concrete ACK object identifiers.
Unregistered responders cannot inherit another owner's association from a shared
database. Mixed RPM succeeds with inline UNKNOWN_OBJECT for an unavailable port
when another property is accessible. Successful target Audit uses the same
concrete object and preserves per-target records. Endpoint responder RPM remains
unsupported. Same-device multiport/router generations, rebind, pending activation
(and with it writes to the BBMD and foreign-device rows), NAT traversal, B/IP
multicast, DHCP and full Network Port conformance remain outside this profile.

`EndpointSession::bip_local_address()` returns the active post-bind announced
address, including the actual ephemeral UDP port, for registered and unregistered
B/IP sessions. It is absent before publication, during/after shutdown and for
other links. This is a logical announced address, not physical NIC provenance.
Cancelled startup can be stopped and joined; transport cleanup failures are
reported by endpoint stop rather than discarded.

## bacnet-endpoint

`bacnet-endpoint` composes client and server roles for one BACnet device under
one transport owner. One `EndpointSession` owns the transport, ingress, and
shared outbound coordinator. Use its builders when both roles need that shared
lifecycle. Standalone `BACnetClient` and `BACnetServer` remain public APIs with
their own service and data-link capabilities; they are not deprecated. The
endpoint responder's narrower service scope is described below.

### Builders

```rust
use std::net::Ipv4Addr;
use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::session::SessionRole;

// One device, both roles, B/IP.
let identity = DeviceIdentity::new(1001, 42)?
    .with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)?;
let db = identity.build_database()?;
let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
    .role(SessionRole::Both)
    .database(db)
    .identity(identity)
    .build_session()?;
session.start().await?;
session.broadcast_i_am().await?;
session.stop().await?;
# Ok::<(), bacnet_types::error::Error>(())
```

`ScEndpointBuilder` composes SC (`build_loopback_session` for unit
validation; `build_hub_session` over a caller-dialed `TlsWebSocket`,
`sc-tls` only, proven against the local constrained-TLS hub).
`MstpEndpointBuilder` composes one serial owner (simulator evidence only —
no bench or on-wire conformance; timing is RB-26).

### Choosing standalone or shared client/server roles

| Standalone construction | Shared endpoint composition |
|---------------------|---------------------|
| `BACnetClient::bip_builder()...build().await` | `BipEndpointBuilder::new(iface, port, bcast).role(ClientOnly).build_session()?` then `start()` |
| `BACnetServer::bip_builder()...build().await` | `BipEndpointBuilder::new(iface, port, bcast).role(ServerOnly).database(db).identity(id).build_session()?` then `start()` |
| Client + server on one device | One `...role(Both).database(db).identity(id).build_session()?` (B/IP, SC, or MS/TP builder) |
| `BACnetClient::sc_builder()...build().await` | `ScEndpointBuilder::new(vmac, uuid)...build_hub_session(ws)?` (dial first, then compose) |
| `BACnetServer::sc_builder()...build().await` | Same `ScEndpointBuilder` with `ServerOnly` + `database` + `identity` |
| `generic_builder().transport(mstp)...` | `MstpEndpointBuilder::new(serial, station)...build_session()?` (one serial owner) |

Notes: the endpoint server role defaults to `ReadProperty` (+ `Reject`/`Abort`
+ segmentation-`Abort`). The explicit Device-write opt-in below adds one
bounded WriteProperty path, and the ReinitializeDevice opt-in adds that
service through an application handler; full `bacnet-server` parity is out of
scope. The responder supplies exactly the service bits it executes (RP, plus
WP and ReinitializeDevice when enabled) and exposes neither COV
list property, even for a custom database Device. Server-role identity service
lists must include RP and contain no unsupported bits; validation runs before
ingress even when both opt-ins are off. With an opt-in, any declaration within
the executed set is accepted, and that set is committed only after all
validation succeeds. ClientOnly creates no
responder and retains its services vector as a local declaration.
Standalone BBMD helpers (`read_bdt` / `write_bdt` / `read_fdt` / foreign
registration) stay on `BipTransport` and on `BACnetClient` over B/IP (see
[Transport access and BBMD helpers](#transport-access-and-bbmd-helpers));
the endpoint BBMD setters only stage
pre-start state. Local Number controls have the bounded wire coverage above;
broader BBMD/foreign administration remains experimental.
BIPv6/Ethernet have no endpoint builder — keep the standalone path there and
do not expect identical administration across data links.

### Endpoint shutdown and the object database

`EndpointSession::stop()` joins its request owners, removes source membership,
then settles forgotten staged writes and waits for queued durable save attempts,
including any correction that restores the state the object serves (#1581).
This applies to every session role with an attached database. It awaits the
database lock, so release application-held guards to let shutdown progress;
the guard is released before waiting for storage. A stalled backend keeps
shutdown pending and produces periodic warnings. Completion means those save
attempts finished, not that the backend succeeded or the data survived power loss.
Application work that continues independently of the session is not joined.

Canceling `stop()` retains its original dispatch or ingress outcome and one
settlement task. Call it again to finish waiting. The session stays stopping
until settlement completes, then returns the saved outcome. It keeps its object
database through `stop()` and lets go of it when it drops.

Every endpoint holder of the database lets go through
`bacnet_server::server::drop_database_off_runtime` (#1561): the session, the
server role's responder (which a cloned `ServerRoleHandle` or the dispatch task
may hold last), the source Audit runtime (which an audited request in flight
may hold last), the session's Number task and its settlement task. So whichever goes last, in async
code a durable object's final saves run on Tokio's blocking pool, not on a
runtime worker. Dropping a session aborts its settlement task without awaiting
it; storage may still change after drop returns. Await `stop()` for the joined
settlement boundary.


### Authorized endpoint Device writes

`EndpointSession::with_device_writes(authorizer)` or
`BipEndpointBuilder::device_writes(authorizer)` enables WriteProperty for the
one local Device's `Description` and, with a complete source profile,
`Audit_Notification_Recipient`. Supply a mandatory Rust
`bacnet_server::mutation::MutationAuthorizer`; its decoded context retains the
immediate peer, claimed routed source, transport provenance and invoke ID.
The callback must be fast, nonblocking and side-effect-free. Refusal or panic
denies before mutation. Provenance is channel scope, not authenticated leaf identity.

Startup requires a server role and a concrete built-in local Device in the
attached database (the lowest when it holds several; see
[Databases with several Devices](#databases-with-several-devices)). An optional
`DeviceIdentity` must match that Device
and contain only the service bits the responder executes. Configuration
validation precedes transport startup and any profile/source-ownership changes.
The enabled Device and identity advertise exactly RP+WP, plus ReinitializeDevice
when that is enabled too, including sessions without an identity. Default
sessions keep their existing RP-only responder.
Valid priorities 1–16 are ignored for noncommandable Description. Authorized
NULL relinquishment succeeds without changing its value. Array indices,
out-of-range priorities and other non-string values fail; numeric priority
range errors use SERVICES/PARAMETER_OUT_OF_RANGE. Missing objects/properties
return UNKNOWN_OBJECT/UNKNOWN_PROPERTY; known out-of-scope writes are denied.
Typed Device authority is revalidated under the commit lock, including when
the lower-level responder is used directly. Other targets, properties and
WritePropertyMultiple remain excluded, including source Reporter configuration.

Deterministic request/reply tests cover authorization, framing, routing,
reply channels, group silence, segmentation and shutdown; a B/IP loopback test
covers an authorized write and service-profile readback. This is not general
endpoint mutation parity or inbound replay suppression. The source recipient extension
is described below and in the [Device recipient contract](device-audit-recipient.md).

### Endpoint ReinitializeDevice

`EndpointSession::with_reinitialize(handler)` or
`BipEndpointBuilder::reinitialize(handler)` lets the endpoint execute
ReinitializeDevice through the same `ReinitializeHandler` signature as the
full server (see [The ReinitializeDevice handler](#the-reinitializedevice-handler)),
with the request's `ReinitializeContext` built from the received source,
routing and provenance. `with_reinit_password` / `reinit_password` sets the
password a request must carry; startup refuses a password without a handler,
and `build_transport` refuses either one. Startup requirements match the
Device-write opt-in (a server role and a concrete built-in local Device), and
the Device and identity then advertise ReinitializeDevice next to RP (and WP
when writes are on).

The handler rules are the server's, with one addition: the session answers
inbound requests on a single dispatch task, so a slow or blocking handler
stalls every request behind it. A panic or a returned Reject is answered
`SERVICES / OTHER` and the session keeps serving. A close that wins while the
request waits for the database refuses it before the handler runs.

### Direct endpoint WriteProperty and source WRITE reporting

`ClientRoleHandle::write_property` accepts a direct B/IP IPv4 unicast MAC, a
`bacnet_services::write_property::WritePropertyRequest` (object, property, optional
index, complete encoded property value, optional wire priority), and the required
`bacnet_endpoint::roles::Commandability::{Commandable, Noncommandable}`.
This assertion is required with or without a source Reporter; neither object type,
property identifier, local object state nor the supplied priority establishes it.
The method refuses other endpoint transports and invalid/group destinations before
traffic. There is no routed WP method in this subset.

The shared requester validates zero or more complete TLVs, priority 1–16 when
supplied, and the complete unsegmented APDU size before reserving an Invoke ID.
Empty lists and encoded NULL are distinct valid representations. The wire priority
and bytes remain unchanged. A matching SimpleACK returns `Ok(())`; Error, Reject,
Abort and timeout retain the established error mapping. A wrong-service Error or ACK, or a wrong
ACK shape, cannot complete the write. Error correlation also protects notification
leases in the shared coordinator.

The same session source Audit owner captures live Reporter policy, recipient,
source Device, timestamp, identity and Invoke ID once. Commandable omitted priority
is effective 16 for filtering and reporting, including NULL. Noncommandable writes
ignore priority for Audit, even when it was supplied on the wire. The source
Reporter controls level/operation/priority filtering; remote policy is not consulted.
Eligible attempted writes generate at most one WRITE record across retries, with
complete 0–32-byte `Target_Value` and no value field for larger payloads. The source
never invents remote `Current_Value`, target timestamp, or execution evidence.

Before source admission, cancellation releases caller-owned work. Eligible admitted
writes retain terminal observation after caller cancellation. Nonreported writes
remain caller-owned: cancellation retracts queued requester sends and drops any
in-progress transport future. A transport attempt may already have reached the
peer, so cancellation never proves the write was not executed. Ordinary detached
egress sends retain their existing semantics. The existing 64-operation budget, notification budget,
recipient generation fences, three-second delivery deadline and stop/drop behavior
are shared with reads. Notification failure cannot replace the caller's write result.
Source WP is a bounded extension under #345/#852; WPM, routed writes, standalone
source ownership and other transports remain outside this profile.

### Bounded endpoint source READ reporting

Standalone direct/routed and endpoint ReadProperty share ACK object/property/index
validation (Clause 15.5). Device/Network Port instance 4194303 requests accept only
same-type concrete peer-reported identifiers; other mismatches return decoding
errors. This client contract does not implement the bundled server's Network
Port ingress-port alias mapping (#785).

Endpoint ReadPropertyMultiple accepts 1–64 explicit property occurrences across
nonempty object specifications, with concrete object identifiers. ALL, REQUIRED,
OPTIONAL and wildcard object instances are excluded from this endpoint profile;
standalone RPM retains its broader profile. Array index zero is valid. Shared
RPM request encoding is fallible and validates both lists before appending bytes.

RPM ACK correlation checks all object/property counts, order and identifiers
before returning success or projecting any record. A successful value must echo
the requested index. An inline error may omit a requested index or repeat it,
but cannot substitute another index; an unindexed request requires no index.
The Audit attempt retains the requested index. Indistinguishable duplicate
omitted-index errors cannot reveal a peer's ordering violation. Request and raw
ACK bytes must fit the configured unsegmented max APDU. The server's separate
known-scalar error-index response issue remains tracked in #789.

One RPM operation retains one requester lease, source-operation slot, timestamp,
invoke ID, owned worker and recipient/configuration snapshot across retries and
caller cancellation. Each eligible occurrence produces a separate value-free
READ record, including duplicates. AUDIT_CONFIG excludes Present_Value per
occurrence; returned values remain caller-only. Inline errors retain their
class/code. Whole Error/Abort/Reject, timeout, malformed, mismatched or segmented
outcomes are locally represented by the final operation failure on every eligible
attempted reference; they do not imply remote per-property execution. Notification
admission is independent and bounded through the existing delivery and resource
failure-summary owners; no batch reserves 64 notification permits across request I/O.
Known synchronous egress QueueFull also counts as local resource loss; Closed or
shutdown and already-attempted transport/ACK failures do not. A full-queue summary
returns its count to the same bounded generation-fenced coalescer and waits for
capacity. It never counts itself or retries an ordinary record; stop owns that wait.

After a complete validated RPM ACK, one distinct concrete Device object with at
least one successful property establishes Target_Device for all records in that
operation. Error-only Device results establish none; conflicting successful
Device IDs retain Address attribution without rejecting the otherwise valid ACK.
This is operation-local knowledge, with no discovery cache. Direct B/IP source
limits and the absence of Python source Reporter configuration remain unchanged.

The initiating role supports `read_property`, `read_range`, `read_property_multiple`
and direct B/IP `write_property`, plus explicit
endpoint destinations. ReadRange returns a correlated `ReadRangeAck` with raw
item bytes; empty and multiple-item ACKs each produce one value-free source READ
record. `read_range_with` takes a `ReadRangeValidation`: a lenient read keeps
a page that breaks a rule and is audited as a success. Request encoding validates before output/transaction admission: ALL,
REQUIRED, OPTIONAL, array index zero, zero/non-INTEGER16 counts, and nonconcrete
ByTime components are rejected. Zero position/sequence references are valid and
may match no items. Rust and Python support all-items, position, sequence and ByTime.
Endpoint requests/responses are unsegmented; a received segmented response is a
failed attempted read and is reported using the caller's terminal result.

On direct B/IP IPv4, provision the typed recipient on the built-in Device and
select `EndpointSession::with_source_audit_reporter`. Device recipient choices
resolve through immutable `BipEndpointBuilder::source_audit_device_binding` entries;
a direct Address choice needs no binding. Once the session knows its network's
number, an Address choice naming that number is direct too, and its records go
to that MAC with no DNET (#1403). An audited read routed to that number is
audited as the direct read it then is. A provisioned Address naming a network
the session's number does not name, or not yet, starts unresolved with
CONFIGURATION_ERROR and resolves once the number names it (#1461); see
[Device Audit recipient](device-audit-recipient.md). The local database must have a
concrete built-in local Device (the lowest when it holds several; see
[Databases with several Devices](#databases-with-several-devices)) and the
selected Audit Reporter. Configure the Reporter's READ bit
and audit level before startup; both `ClientOnly` and `Both` sessions support
confirmed and unconfirmed notifications. `Both` additionally requires an explicit
Device write authorizer. Startup itself emits nothing.

```rust
use std::net::{Ipv4Addr, SocketAddrV4};
use bacnet_endpoint::{bip::BipEndpointBuilder, DeviceIdentity, SessionRole};
use bacnet_objects::{audit::AuditReporterObject, traits::BACnetObject};
use bacnet_types::{bitstring::AuditOperationFlags, enums::{AuditLevel, AuditOperation, ObjectType}, primitives::ObjectIdentifier};
# async fn example() -> Result<(), bacnet_types::error::Error> {
let mut db = DeviceIdentity::new(123, 42)?.build_database()?;
let local_device = ObjectIdentifier::new(ObjectType::DEVICE, 123)?;
let logger = ObjectIdentifier::new(ObjectType::DEVICE, 999)?;
db.get_mut(&local_device).unwrap().device_authority_internal().unwrap()
    .provision_audit_recipient(bacnet_types::constructed::BACnetRecipient::Device(logger))?;
let mut reporter = AuditReporterObject::new(1, "Source READ")?;
reporter.set_audit_level(AuditLevel::AUDIT_ALL)?;
let mut operations = AuditOperationFlags::empty();
operations.insert(AuditOperation::READ);
reporter.set_auditable_operations(operations);
reporter.set_issue_confirmed_notifications(true);
let source = reporter.object_identifier();
db.add(Box::new(reporter))?;
let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
    .role(SessionRole::ClientOnly).database(db)
    .source_audit_device_binding(
        ObjectIdentifier::new(ObjectType::DEVICE, 999)?,
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, 47808),
    ).build_session()?.with_source_audit_reporter(source);
session.start().await?;
// Use session.client().unwrap().read_property(...) or read_range(...) for a direct IPv4 target.
// Trusted runtime writes use the same Device owner; None relinquishes unchanged.
session.write_audit_recipient(None).await?;
session.stop().await?;
# Ok(())
# }
```

`Monitored_Objects` must be absent for this profile: empty and NULL-only lists
are also rejected, atomically before startup consumes the transport. Selecting
a source requires typed Device provision even at NONE; the removed ownership-only
mode and static recipient selector have no compatibility aliases. Unresolved
Device bindings expose CONFIGURATION_ERROR and suppress ordinary records without
changing READ results or consuming an audit sequence. Source policy and the
Device recipient route are sampled before each admitted READ; a later change
does not rewrite an in-flight request's record or destination. The local
`AUDIT_CONFIG` classification excludes `Present_Value` and includes other
properties. Priority filters do not filter READ. Requests selected for source
reporting reject routed, broadcast, or non-IPv4 destinations before traffic.

A record contains the local source Device, one request-time timestamp, the actual
ReadProperty, ReadRange or ReadPropertyMultiple invoke ID shared across retries, and the requested
property/array index. Successful ReadProperty records use the validated ACK's
object identifier, including concrete Device/Network Port replies to wildcard
requests. On failure or without a valid ACK, the record retains the requested
object (including its wildcard alias) as the attempted identity; it does not
infer a concrete remote object from a malformed or mismatched ACK. A validated
successful concrete Device ACK also establishes Target Device for that record,
for either ReadProperty or ReadRange, as required by Table 19-4.
Network Port and other object ACKs, failures and missing valid ACKs retain the
exact direct BACnet address, including the UDP port. No cross-operation remote
Device cache is created. The selected Device recipient or Address identifies the
logger sink; it is never substituted for the operation target. Unknown user, source
object, remote timestamp, priority and property values are omitted. Independent
source and target reports may both arrive; when the source record retains an
address target, it need not correlate with a target record that knows its own Device.

A valid matching ACK has no Result. Peer Error class/code is preserved when
representable. Local records use Clause 18.7 COMMUNICATION codes for timeout and
known Abort/Reject reasons, including proprietary reasons. Reserved or unmapped
reasons (including Reject reason 10), malformed/mismatching ACKs, and ambiguous
local transport failures use COMMUNICATION/OTHER. These are record fields, never
Error PDUs sent to the peer. An already-observed peer terminal takes precedence
over a contradictory local send error. Rejected pre-send admission is silent;
a later retry failure retains evidence of earlier transmission attempts.

Once an audited request transfers to session ownership, dropping its caller
waiter does not cancel it: the session observes the response or deadline and
records once. Ordinary requests retain caller-owned RAII cancellation. There
are 64 whole-operation slots, acquired before asynchronous policy reads, and a
separate shared pool of 64 active audit notifications. Read results do
not wait for audit delivery. Notifications have one absolute three-second
send/ACK deadline, no retries and no ordinary-record backlog. Expired or canceled
queued notification commands are discarded before transport execution; a send
already in progress may have reached the peer when cancellation wins.

Overload, encoding, send and acknowledgment failures update the selected
Reporter's instance-owned Reliability without replacing the read result.
Completion authority includes the configuration generation: an old delivery
cannot clear a newer failure or update health after configuration changes.

Eligible, encodable READ records that fit the APDU but lose admission to the
shared audit permit pool or confirmed invoke-ID pool contribute to one bounded,
memory-only AUDITING_FAILURE batch. The local filtering policy requires a
non-NONE Audit_Level and the AUDITING_FAILURE operation bit. The summary places
the earliest lost record's source timestamp in Target_Timestamp, the local
Device in both Source_Device and Target_Device, and a saturating application
Unsigned count in Current_Value. Other optional fields are absent. Earliest
means record admission order, including reversed completions, sequence wrap and
changes of clock representation. Encoding/size errors, filtering, pre-send
rejection, closed admission, transport/ACK failures and summary failures do not
contribute. Failed summaries never recursively produce another summary.

One owned worker waits passively for actual semaphore or shared coordinator
capacity; requester-only releases also wake it. Further losses coalesce into
sequential batches, without queuing or replaying ordinary records. Each batch
belongs to one immutable Reporter instance, configuration generation, delivery
mode and destination. Changes discard incompatible pending counts, including
A-to-B-to-A changes without another READ. Active Device/Reporter removal and
replacement are denied. A new context
can supersede the single pending slot; stale completions cannot transfer their
counts into it. Target reporting instead retains bounded captured historical
contexts, as described in [delayed target Audit reporting](delayed-target-audit.md). Admitted notifications retain the three-second total deadline and
no retries.

`stop()` seals admission, cancels operations and notifications, and joins owned
workers before uninstalling under the DB guard. Canceled stop retains sealed
protection and join handles for a later stop; Drop cancels and retains structural
protection only until owned task frames quiesce. Source projection and configuration
restrictions deactivate on sealing. Shutdown and context changes can lose
undelivered records and pending counts. This is not a durable delivery promise
or full Audit Reporting/BIBB/BTL conformance. Other source operations, multiple Reporters,
selector semantics, batching/send delay, standalone source ownership and other
transports remain outside this subset.


### Object-owned Audit policy

Analog Value and Binary Value support independently optional, writable
`Audit_Level`, `Auditable_Operations`, and `Audit_Priority_Filter` properties.
Color and Color Temperature support the first two the same way (#1525); their
tables have no `Audit_Priority_Filter`, and neither object has a commandable
property for one to filter.
Provision `bacnet_objects::audit::ObjectAuditPolicy` through `set_audit_policy`
before registration. `None` omits a property; DEFAULT level and
`AuditPriorityPolicy::Inherit` (a present NULL priority filter) inherit the selected
Reporter's settings. Metadata and Property_List expose only provisioned rows.
Only commandable AV/BV instances expose the optional Audit_Priority_Filter;
it applies to commandable-property writes, not Description or lifecycle
operations. Noncommandable instances retain the other supported provisioned
Audit fields. Provisioning does not install or enable a Reporter.

Target READ/WRITE/CREATE/DELETE use the effective instance policy. The selected
Reporter's NONE level remains the master suppression boundary. With an enabled
Reporter, an actual object Audit_Level change is recorded across NONE and despite
a cleared WRITE bit. Actual Auditable_Operations changes bypass WRITE only while
the effective object level is enabled. Equal-value and failed writes use ordinary
filters. Each WPM element captures its pre-state; later elements see committed
policy. Successful BV CREATE uses the created policy, DELETE captures it before
removal, and failed CREATE uses Reporter fallback. Network AV creation remains
unsupported. Source reporting ignores remote object policy.

For those eligible actual setting changes, network WP, each WPM element and
`BACnetServer::write_local` prepare the immediate notification before assigning
the built-in policy field. Unavailable route, runtime, send capacity, confirmed
lease or APDU fit returns SERVICES/SERVICE_REQUEST_DENIED without changing that
field, consuming a clockless sequence or retaining a worker/lease. WPM keeps its
successful prefix and stops at the denied element. This stronger admission rule
is a local policy, not a Standard-mandated write rejection. A successful admission
owns one bounded delivery attempt; later send/ACK failure cannot undo the write.
Ordinary, equal and NULL writes keep their existing best-effort behavior. These
mandatory records bypass the separately configured [delayed target queue](delayed-target-audit.md).

`BACnetServer::write_local` uses the same target observer, with local Device
provenance and no invoke ID. Device recipient changes still emit only their
old/new pair. Application Input/noncommandable Value updates through `set_present_value_local`
are silent to the target Audit observer;
raw object/database authoring bypasses notification ownership.

The AV/BV object clauses (§12.4 printed185/PDF187; §12.8 printed211/PDF213)
inherit the Reporter's priority filter when the object row is absent or NULL.
Generic §19.6.3 (printed820/PDF822) conflicts for the absent case. This bounded
implementation follows the object-specific clauses; the 2024-04-29 errata does
not resolve that wording and adds the commandability condition. Other object
families and broader Audit completion remain open.

### Object profile rows

Most object tables list `Tags`, `Profile_Location` and `Profile_Name` as
optional rows. `bacnet_objects::object_profile::ObjectProfile` holds the three;
an object that carries one serves the rows its fields provision (#1553). The
Color, Color Temperature, Lighting Output, Binary Lighting Output, Analog Value,
Binary Value, Multi-state Value, Analog Input, Binary Input and Multi-state Input
objects take one through `set_profile` before registration, which checks it first and
refuses a bad one without changing anything. An unprovisioned object serves and
lists none of them, so its wire behaviour, Property_List and PICS are as before.
The three rows retain their relative order: Tags, Profile_Location, Profile_Name.
They are independent of a Value object's Present_Value access and source tracking,
and of an Input object's Out_Of_Service state. Input Present_Value simulation
writes and internal application updates retain their existing ownership rules.
Existing descriptor order is preserved when the optional rows are added.

- `Tags` is a BACnetARRAY of `bacnet_types::constructed::BACnetNameValue`, a
  name with an optional primitive `PropertyValue`, whose codec
  is `bacnet_encoding::constructed::{encode_name_value, decode_name_value}`.
  The 2024-04-29 errata restricts each value to one primitive: Date and Time
  are allowed separately, but their pair is refused. `None` is a semantic tag;
  `Some(PropertyValue::Null)` is a valued tag.
  Reads return each element as `PropertyValue::ApplicationData`. Peers write it
  whole, one element by index, or its size at index 0, which truncates or
  appends empty semantic tags (Clause 12.1.5.1); an index past the end is
  INVALID_ARRAY_INDEX and doesn't grow it. A name with a semicolon is
  VALUE_OUT_OF_RANGE (Annex Y.1.4), more than `MAX_TAGS` (1024) elements is
  NO_SPACE_TO_WRITE_PROPERTY, an element of another datatype INVALID_DATA_TYPE
  and one that doesn't decode INVALID_DATA_ENCODING.
- `Profile_Location` and `Profile_Name` belong to the application and are
  read-only over the network, as their O code allows; the PICS lists them as
  readable only. `set_profile` takes an empty location or one whose URI scheme
  is http, https or bacnet, and a profile name that starts with a decimal vendor
  identifier and a dash.

Objects built with `new` or `with_access` keep Tags in memory. Color, Color
Temperature, Lighting Output and Binary Lighting Output also provide
`with_tags_persistence(instance, name, Arc<dyn TagsPersistence>)`. The Value
constructors accept an explicit `PresentValueAccess` before the storage argument:

- Analog Value: `with_tags_persistence(instance, name, units, access, storage)`.
- Binary Value: `with_tags_persistence(instance, name, access, storage)`.
- Multi-state Value: `with_tags_persistence(instance, name, number_of_states, access, storage)`.

The Input constructors retain their normal arguments and append storage:

- Analog Input: `with_tags_persistence(instance, name, units, storage)`.
- Binary Input: `with_tags_persistence(instance, name, storage)`.
- Multi-state Input: `with_tags_persistence(instance, name, number_of_states, storage)`.

For example, construct a writable Analog Value with
`AnalogValueObject::with_tags_persistence(1, "Temperature", 62, PresentValueAccess::Writable, storage)?`,
then call `set_profile` to provision the desired rows. `storage` is an
`Arc<dyn TagsPersistence>` supplied by the application. The shared
`bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot,
FileTagsPersistence}` contract attaches application-owned storage to those
objects. This is an opt-in product durability feature; optional Tags rows do
not establish a universal Standard persistence requirement.

Call `set_profile` to provision the rows after constructing the object. A
successful saved Tags write overrides the configured array, including a saved
empty array. No saved write is distinct from a saved empty array. Storage never
provisions a row: `tags: None` keeps Tags absent from reads, writes, Property_List
and PICS even when old Tags are stored. Removing and later reprovisioning Tags
reapplies the saved override. Profile_Location and Profile_Name remain separately
provisioned and are never part of the saved payload. `set_profile` validates
before changing anything and retains the attached store; a valid provisioning
change supersedes any staged Tags write through the existing correction path.
Configured defaults are not saved merely by provisioning them.

Tags writes validate a prospective array before submitting a save. The object
serves it only after the backend reports success; a failed save returns
DEVICE/OPERATIONAL_PROBLEM and leaves the old served array. Backend implementations
must refuse a save without replacing their previous snapshot. Whole-array,
index-0 resize and indexed-element writes save the complete resulting array.
WritePropertyMultiple composes its Tags changes in order, saves the final
applicable state once, and retains normal successful-prefix behavior on a later
failure. The bundled server stages network and `write_local` saves while releasing
the database lock. Direct synchronous object/database writes wait in their caller,
as with other persistent objects.

Attached storage enforces `MAX_TAGS_SNAPSHOT_BYTES` (1 MiB for the complete encoded
snapshot including its identity/version header) in addition to `MAX_TAGS`.
Oversized writes fail with NO_SPACE_TO_WRITE_PROPERTY before the backend is
called. This is an opt-in storage resource limit, not a BACnet limit; memory-only
objects retain their existing limits. Both file and custom loaded snapshots are
checked for the same Tags semantics and encoded-size limit. The file backend also
rejects another object's identity, unsupported format versions and malformed data.

The existing durable writer handles abandoned stages, correction, maintenance,
graceful stop and object Drop. `wait_for_tag_saves` waits for queued attempts; it
does not convert a failed attempt into successful persistence. Stop waits for
correction attempts after canceling requests. Restoring the served state still
requires a successful backend correction. `FileTagsPersistence::new(path)` takes
an explicit application path and uses the shared synchronized temporary-file and
atomic replacement backend. Parent-directory synchronization after rename is
best effort and logged on failure. A successful save or local file round trip
does not qualify power-loss survival; the backend's actual guarantees apply.
The file backend does not coordinate competing owners or processes at one path.

Cloning a Color, Color Temperature, Lighting Output or Binary Lighting Output
object copies its served data into a memory-only object. Application clones and COV snapshots do not inherit persistence,
pending writes or saved-override authority; clone/drop neither saves nor corrects,
settles or waits on the source writer. Changes to a clone remain local to it.
Typed remote Tags decoding is available through the Rust client helper, Python
reads and CLI reads. Other object families remain separate work
(#1584); this Rust provisioning surface adds no Python server provisioning API.

### Target Device Audit recipient

The standalone target profile uses `DeviceObject::provision_audit_recipient` for
initial state and `AuditReportersConfig { reporters }` for selection. Active local
and authorized network recipient writes share atomic admission of the change's two
notifications: to the old and the new recipient, or to the new one and by global
broadcast when the old one is an Address the network number does not name.
See the [Device recipient contract](device-audit-recipient.md) for supported routes,
metadata, failure semantics and shutdown ownership. The endpoint source profile
uses the same typed Device value and a source-owned paired delivery path; it has
the narrower role, route and service boundaries described above.

### Multi-device batch concurrency

`BACnetClient::{read_property_from_devices, read_property_multiple_from_devices,
write_property_to_devices}` take `Option<std::num::NonZeroUsize>` for their
concurrency limit. `None` uses 32; `Some(NonZeroUsize::new(1).unwrap())` serializes
the requests. Zero is unrepresentable at this boundary. All three retain their
`Vec` results in completion order and complete empty batches. Each
`DeviceReadResult`, `DeviceRpmResult` and `DeviceWriteResult` now includes
`request_index: usize`, the zero-based occurrence in the original input vector,
including duplicate identical requests to one Device. The existing `device_instance`
and typed `Result` outcomes remain; requests are consumed without cloning a full
request or encoded write value into the result. Use the index to correlate with
caller-owned input metadata. This is a pre-1.0 result-shape change without aliases.

Dropping the batch future cancels pending requests and stops queued work without
returning a partial vector; already-sent remote writes cannot be retracted.
`DeviceWriteRequest` Debug shows the encoded value length instead of its bytes;
RP/RPM result Debug omits successful ACK payloads. Values remain accessible through
the public outcomes, and other error/debug formatting is not a secrecy boundary.
