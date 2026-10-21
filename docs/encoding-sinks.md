# Encoding into caller-owned storage

Tag headers, context opening/closing markers and the bounded scalar primitive
family, the two primitive timestamp encoders, five bounded reference encoders,
the CharacterString, OCTET STRING and BIT STRING encoders, the APDU header
writer and the Who-Is, I-Am and ReadProperty encoders support caller-selected
storage. Their existing `BytesMut` entrypoints and output remain unchanged.
They are the encoding half of #1614, which also gives `bacnet-types`,
`bacnet-encoding` and `bacnet-services` a profile with no allocator; the other
composite encoder families still use their `BytesMut` buffers and need
`alloc`.

## Output contract

`bacnet_encoding::sink::EncodeSink` has two operations:

- `remaining_capacity(&self) -> Option<usize>` reports the bytes that can still
  be appended. `Some(n)` is a bounded sink's remaining room; `None` means it
  does not advertise a fixed bound. This is independent of a growable
  container's currently allocated capacity.
- `write(&mut self, bytes: &[u8]) -> Result<(), SinkFull>` appends the complete
  slice, or returns an error without changing output bytes or the output
  position. Implementers must preserve this per-write atomicity.
- `write_parts(&mut self, parts: &[&[u8]]) -> Result<(), SinkFull>` appends
  several slices in order as one value, or none of them. The provided method
  adds up their lengths, refuses a total that overflows `usize` (reporting
  `needed: usize::MAX`) or exceeds the advertised capacity, and only then
  writes each part. That is atomic when `write` never refuses an append the
  advertised capacity covers, and a sink advertising `None` never refuses at
  all, which holds for every adapter below. A sink that can refuse despite
  advertising enough room, such as one with a quota it doesn't advertise or
  fallible storage, must override `write_parts`, staging the parts or undoing
  the ones it wrote.

`SinkFull { needed: usize, available: usize }` describes the refused append
without a string allocation. Each sink encoder returns `Result<usize,
SinkFull>`, reporting the appended byte count on success. A known shortage
returns the total required and available lengths without calling `write`.
A custom sink can still refuse a write despite unknown or sufficient advertised
capacity; its error is returned unchanged. This contract is for byte storage,
not partial I/O.

The functions `tags::encode_tag_to_sink`, `encode_opening_tag_to_sink` and
`encode_closing_tag_to_sink` stage a complete header and make one write. Tag
headers occupy at most seven bytes, and opening/closing markers at most two.

Each bounded scalar `_to_sink` function also makes one caller write for its
**whole value**, including any tag and length octets. Tagged values use an
eleven-byte staging buffer: up to eight content bytes and three header bytes.
A refusal leaves earlier output, the slice cursor, container length and unused
tail unchanged. Header and payload are never issued as separate caller writes.
The eleven-byte output bound does not describe the full call-stack requirement.

A variable-length encoder stages its header and leading octet, at most eight
octets, and appends them with the caller's payload in one `write_parts` call:
the payload is never copied into a staging buffer.

Atomicity stops at one encoder call. Several successful calls followed by a
refusal retain the completed earlier values. A total-capacity preflight does
not provide rollback if an arbitrary custom sink refuses a later write. No
rollback, seek or allocation interface is added.

## Scalar entrypoints and wire behavior

The following existing `primitives` functions each have a matching `_to_sink`
entrypoint. Their sink argument accepts `EncodeSink + ?Sized`.

| Form | Existing names with the new suffix available |
| --- | --- |
| Raw contents | `encode_unsigned`, `encode_signed`, `encode_real`, `encode_double` |
| Application tagged | `encode_app_null`, `encode_app_boolean`, `encode_app_unsigned`, `encode_app_signed`, `encode_app_real`, `encode_app_double`, `encode_app_enumerated`, `encode_app_date`, `encode_app_time`, `encode_app_object_id` |
| Context tagged | `encode_ctx_unsigned`, `encode_ctx_signed`, `encode_ctx_real`, `encode_ctx_double`, `encode_ctx_enumerated`, `encode_ctx_boolean`, `encode_ctx_object_id`, `encode_ctx_date` |

Unsigned and Signed retain their minimum big-endian widths and the existing
`unsigned_len` / `signed_len` results. Enumerated uses the unsigned layout.
Real and Double preserve their IEEE-754 payload bits in big-endian order.
Application Null has no contents; application Boolean carries its value in the
header, whereas context Boolean has one content byte. Date and Time retain
their four fields, including wildcard bytes; ObjectIdentifier retains its
10-bit type and 22-bit instance. Tag lengths and these scalar layouts follow
ASHRAE 135-2020 Clauses 20.2.1.3, 20.2.2–20.2.7 and 20.2.11–20.2.15.

For example, context Unsigned 256 under tag 15 occupies four bytes:

```rust
use bacnet_encoding::primitives::encode_ctx_unsigned_to_sink;

let mut storage = [0xA5; 4];
let mut tail = &mut storage[..];
assert_eq!(encode_ctx_unsigned_to_sink(&mut tail, 15, 256).unwrap(), 4);
assert!(tail.is_empty());
assert_eq!(storage, [0xFA, 0x0F, 0x01, 0x00]);
```

The corresponding public rustdoc example is executable in the default and
alloc-only profiles. Independent literal wire cases cover widths, boundaries,
inline and extended context tags; a separate unchanged baseline probe checks
legacy `BytesMut` output. Compatibility cases separately preserve tag 255,
unvalidated Date/Time fields and nonfinite floating-point payloads. Those cases
do not advertise the inputs as valid in every BACnet service context.

## Timestamp entrypoints and wire behavior

`primitives::encode_timestamp_choice_to_sink` writes a bare `BACnetTimeStamp`
CHOICE, while `encode_timestamp_to_sink` includes its enclosing context tag
pair. Both accept `EncodeSink + ?Sized`, return `Result<usize, SinkFull>` and
make one write for the whole public call. The bare choice occupies at most
twelve bytes; the outer frame occupies at most sixteen. These are encoded-output
and staging bounds, not full call-stack measurements.

Time retains its context-0 primitive form. SequenceNumber retains context 1
and the shortest representation of its `u16` value. DateTime retains the
context-2 frame around application Date followed by application Time. An outer
tag of 2 encloses that complete inner frame; the two layers do not collapse.
These layouts follow the BACnetTimeStamp and BACnetDateTime productions in
ASHRAE 135-2020 Clause 21 and the tagging, sequence and CHOICE rules in Clause
20.2. The Standard marks the Time choice deprecated from protocol revision 21;
this migration preserves the existing input without adding a Rust deprecation.

The old `BytesMut` functions keep their `Result<(), Error>` signatures and
bytes, delegating to the infallible growable adapter. No timestamp semantic
error is added. Unchecked Date/Time fields and outer tag 255 remain separately
labeled compatibility inputs; preserving bytes does not make them valid in
every service context. Independent literals cover each choice, the sequence
width boundary, outer tags and nested context-2 frames. An unchanged baseline
consumer separately checks legacy output. Failure tests cover complete-frame
capacity and arbitrary refusal, and mutations challenge layout and one-write
atomicity. Earlier successful encoder calls remain committed if a later one
fails.

## Reference entrypoints and wire behavior

These `constructed` entrypoints accept `EncodeSink + ?Sized` and return
`Result<usize, SinkFull>`. Each stages its complete output and makes one caller
write, including every optional member and any enclosing frame.

| Entry point | Maximum encoded/staged bytes |
| --- | ---: |
| `encode_property_reference_to_sink` | 10 |
| `encode_object_property_reference_to_sink` | 15 |
| `encode_setpoint_reference_to_sink` | 17 |
| `encode_device_object_property_reference_to_sink` | 20 |
| `encode_device_object_reference_to_sink` | 10 |

These are encoded-output and staging bounds, not whole-stack measurements.
PropertyReference retains property then optional array index. ObjectPropertyReference
retains object, property and optional index; SetpointReference encloses that
complete sequence in context 0. Its optional absence is still represented by
callers emitting nothing. DeviceObjectPropertyReference puts an optional device
in context 3 after the other members. DeviceObjectReference instead puts its
optional device in context 0 before the required object in context 1. The
layouts follow the corresponding Clause 21 definitions and ASHRAE 135-2020
Clauses 20.2.1 and 20.2.14–20.2.16.

Missing and zero-valued array indices remain distinct, and unsigned members
keep their shortest widths. The existing `BytesMut` functions retain their
signatures and bytes, including the private device-object-property body used
by other composites. No semantic checks or type changes are added. Raw
property/index bounds, wildcard identifiers and non-Device values in a device
field are labeled compatibility inputs in tests; retaining encoder bytes does
not make those values valid for every service or property.

Independent literals cover member tags/order, optional omissions, all unsigned
width boundaries, complete Setpoint frames and maximum output lengths. An
unchanged external baseline probe also checks enclosing callers of the private
reference body. A refusal preserves prefix, cursor, length and tail for the
whole reference call. This does not make a variable-length enclosing encoder
atomic or provide rollback across successful calls.

## String, octet string and bit string entrypoints

These `primitives` functions accept `EncodeSink + ?Sized`, return
`Result<usize, SinkFull>` and append the whole value through `write_parts`:

| Form | Entry points |
| --- | --- |
| Contents only | `encode_character_string_to_sink`, `encode_bit_string_to_sink` |
| Application tagged | `encode_app_character_string_to_sink`, `encode_app_octet_string_to_sink`, `encode_app_bit_string_to_sink` |
| Context tagged | `encode_ctx_character_string_to_sink`, `encode_ctx_octet_string_to_sink`, `encode_ctx_bit_string_to_sink` |

A CharacterString is the UTF-8 character set the stack encodes: the
character-set octet X'00', then the string's octets (Clause 20.2.9). A BIT
STRING leads with its unused-bit count, unchecked as before (Clause 20.2.10),
and an OCTET STRING is its octets alone (Clause 20.2.8). The header takes the
contents' length, in its short or extended form (Clause 20.2.1.3). Independent
literals cover each form; a comparison at every length where the header's form
changes, up to 70,000 octets, checks the `BytesMut` encoders' bytes, which are
unchanged; refusal tests cover every shortfall of room.

A value whose contents a header's 32-bit length can't express, reachable
only with more than 4 GiB in one value, is refused with `SinkFull`, whose
`needed` is its encoded length and `available` the sink's room, or
`u32::MAX` for a sink that advertises none. The CharacterString `BytesMut`
encoders, and the contents-only BIT STRING one, delegate to these sink
encoders: a too-long CharacterString is a static `Error::Encode` with nothing
written, which a test shows with a 4 GiB string in untouched zeroed memory.
The tagged OCTET and BIT STRING `BytesMut` encoders keep their own code,
since they write such a value with its length saturated instead.

## APDU header and services

`apdu::header::ApduHeader` writes the header of an Unconfirmed-Request (two
octets), of an unsegmented Confirmed-Request (four) or ComplexACK (three), or
of an Error (three), and a whole Reject or Abort (three), with the octets the
owned `encode_apdu` writes (Clauses 20.1.2, 20.1.3, 20.1.5, 20.1.7, 20.1.8
and 20.1.9). The Error, Reject and Abort let a device without a heap refuse
what it doesn't serve (#1771); an Error's class and code follow its header
as two `encode_app_enumerated_to_sink` values. `encode_to_sink` makes one write; `encode_with_parameters_to_sink`
appends the header and the service parameters as one value. The max-segments
and max-APDU fields are typed (`MaxSegments`, `MaxApdu`), so the writer has no
invalid input to refuse.

In `bacnet-services`, `WhoIsRequest`, `IAmRequest` and `ReadPropertyRequest`
stage their members, at most ten, fifteen and fifteen octets, and make one
write. `ReadPropertyAckRef` stages its members and the `[3]` opening tag, at
most sixteen octets, and appends them, the borrowed value and the closing tag
through `write_parts`. Their `BytesMut` encoders delegate to these and keep
their bytes; tests compare them with the member-by-member `BytesMut` encoding
they replaced.

## Storage adapters

| Storage | Capacity report | Successful append |
| --- | --- | --- |
| `&mut [u8]` | Current slice length | Writes at the front and advances the slice to its unused tail |
| `heapless::Vec<u8, N>` | `N - len()` | Extends after existing elements; enabled by the `heapless` feature |
| `alloc::vec::Vec<u8>` | `None` | Extends after existing elements and may allocate; with `alloc` |
| `bytes::BytesMut` | `None` | Extends after existing bytes and may allocate; with `alloc` |

The bounded adapters preserve their bytes, length and remaining slice on a
failed append, and for `write_parts` on a value that doesn't fit, which tests
check for each. Growable adapters retain their ordinary allocation-failure
behavior; `SinkFull` is not an allocation-error abstraction. Existing tag,
scalar, timestamp and bounded reference `BytesMut` functions delegate through
that infallible sink adapter.

## Scope and feature limits

This migration adds no semantic validation or new raw/context variants.
PropertyValue/composite dispatch, lists, the other constructed codecs, the
separate constructed log-field timestamp helper and the remaining services
retain their `BytesMut` APIs, which need `alloc`; they move to sinks only on
demand. No whole-message atomicity is implied by composing encoder calls. A
decoder's configured length bound does not limit a header encoder.

With neither `std` nor `alloc`, `bacnet-encoding` keeps the sink contract,
the slice adapter (and `heapless` with its feature), every `_to_sink`
encoder above except the references, which need `alloc` for their types,
tag and primitive decoding, the borrowed views and the APDU header. Sink
storage selection and bounded staging do not establish physical-device
operation or a performance improvement.
