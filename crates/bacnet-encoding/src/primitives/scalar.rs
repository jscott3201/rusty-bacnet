//! Bounded scalar encoding, shared by growable-buffer and sink entrypoints.

use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
#[cfg(feature = "alloc")]
use bytes::BytesMut;

use crate::sink::{EncodeSink, SinkFull};
use crate::tags::{self, app_tag, TagClass};

/// Encode an unsigned integer using the minimum number of big-endian octets.
#[cfg(feature = "alloc")]
pub fn encode_unsigned(buf: &mut BytesMut, value: u64) {
    encode_unsigned_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a signed integer using minimum octets, two's complement, big-endian.
#[cfg(feature = "alloc")]
pub fn encode_signed(buf: &mut BytesMut, value: i32) {
    encode_signed_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an IEEE-754 single-precision float (big-endian, 4 bytes).
#[cfg(feature = "alloc")]
pub fn encode_real(buf: &mut BytesMut, value: f32) {
    encode_real_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an IEEE-754 double-precision float (big-endian, 8 bytes).
#[cfg(feature = "alloc")]
pub fn encode_double(buf: &mut BytesMut, value: f64) {
    encode_double_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Null.
#[cfg(feature = "alloc")]
pub fn encode_app_null(buf: &mut BytesMut) {
    encode_app_null_to_sink(buf).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Boolean.
///
/// The value is encoded in the tag's L/V/T bits with no content octets.
#[cfg(feature = "alloc")]
pub fn encode_app_boolean(buf: &mut BytesMut, value: bool) {
    encode_app_boolean_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Unsigned.
#[cfg(feature = "alloc")]
pub fn encode_app_unsigned(buf: &mut BytesMut, value: u64) {
    encode_app_unsigned_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Signed.
#[cfg(feature = "alloc")]
pub fn encode_app_signed(buf: &mut BytesMut, value: i32) {
    encode_app_signed_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Real (f32).
#[cfg(feature = "alloc")]
pub fn encode_app_real(buf: &mut BytesMut, value: f32) {
    encode_app_real_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Double (f64).
#[cfg(feature = "alloc")]
pub fn encode_app_double(buf: &mut BytesMut, value: f64) {
    encode_app_double_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Enumerated.
#[cfg(feature = "alloc")]
pub fn encode_app_enumerated(buf: &mut BytesMut, value: u32) {
    encode_app_enumerated_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Date.
#[cfg(feature = "alloc")]
pub fn encode_app_date(buf: &mut BytesMut, date: &Date) {
    encode_app_date_to_sink(buf, date).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged Time.
#[cfg(feature = "alloc")]
pub fn encode_app_time(buf: &mut BytesMut, time: &Time) {
    encode_app_time_to_sink(buf, time).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode an application-tagged ObjectIdentifier.
#[cfg(feature = "alloc")]
pub fn encode_app_object_id(buf: &mut BytesMut, oid: &ObjectIdentifier) {
    encode_app_object_id_to_sink(buf, oid).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-tagged Unsigned.
#[cfg(feature = "alloc")]
pub fn encode_ctx_unsigned(buf: &mut BytesMut, tag: u8, value: u64) {
    encode_ctx_unsigned_to_sink(buf, tag, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-tagged Signed.
#[cfg(feature = "alloc")]
pub fn encode_ctx_signed(buf: &mut BytesMut, tag: u8, value: i32) {
    encode_ctx_signed_to_sink(buf, tag, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-tagged Real (f32).
#[cfg(feature = "alloc")]
pub fn encode_ctx_real(buf: &mut BytesMut, tag: u8, value: f32) {
    encode_ctx_real_to_sink(buf, tag, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-tagged Double (f64).
#[cfg(feature = "alloc")]
pub fn encode_ctx_double(buf: &mut BytesMut, tag: u8, value: f64) {
    encode_ctx_double_to_sink(buf, tag, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-tagged Enumerated.
#[cfg(feature = "alloc")]
pub fn encode_ctx_enumerated(buf: &mut BytesMut, tag: u8, value: u32) {
    encode_ctx_enumerated_to_sink(buf, tag, value)
        .expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-tagged Boolean.
///
/// Context-tagged booleans use a 1-byte content octet (unlike application-tagged).
#[cfg(feature = "alloc")]
pub fn encode_ctx_boolean(buf: &mut BytesMut, tag: u8, value: bool) {
    encode_ctx_boolean_to_sink(buf, tag, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-tagged ObjectIdentifier.
#[cfg(feature = "alloc")]
pub fn encode_ctx_object_id(buf: &mut BytesMut, tag: u8, oid: &ObjectIdentifier) {
    encode_ctx_object_id_to_sink(buf, tag, oid).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-tagged Date.
#[cfg(feature = "alloc")]
pub fn encode_ctx_date(buf: &mut BytesMut, tag: u8, date: &Date) {
    encode_ctx_date_to_sink(buf, tag, date).expect("the BytesMut sink cannot return SinkFull");
}

/// Return the number of bytes needed to encode an unsigned value.
pub fn unsigned_len(value: u64) -> u32 {
    if value <= 0xFF {
        1
    } else if value <= 0xFFFF {
        2
    } else if value <= 0xFF_FFFF {
        3
    } else if value <= 0xFFFF_FFFF {
        4
    } else if value <= 0xFF_FFFF_FFFF {
        5
    } else if value <= 0xFFFF_FFFF_FFFF {
        6
    } else if value <= 0xFF_FFFF_FFFF_FFFF {
        7
    } else {
        8
    }
}

/// Return the number of bytes needed to encode a signed value.
pub fn signed_len(value: i32) -> u32 {
    if (-128..=127).contains(&value) {
        1
    } else if (-32768..=32767).contains(&value) {
        2
    } else if (-8_388_608..=8_388_607).contains(&value) {
        3
    } else {
        4
    }
}

/// Append an unsigned integer in the minimum big-endian width without a tag.
///
/// Uses one atomic write, with the [scalar sink contract](super).
pub fn encode_unsigned_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: u64,
) -> Result<usize, SinkFull> {
    write_scalar(
        sink,
        &value.to_be_bytes()[8 - unsigned_len(value) as usize..],
    )
}

/// Append a signed integer in the minimum two's-complement width without a tag.
///
/// Uses one atomic write, with the [scalar sink contract](super).
pub fn encode_signed_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: i32,
) -> Result<usize, SinkFull> {
    write_scalar(sink, &value.to_be_bytes()[4 - signed_len(value) as usize..])
}

/// Append the four IEEE-754 octets of a Real in big-endian order without a tag.
///
/// Uses one atomic write, with the [scalar sink contract](super).
pub fn encode_real_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: f32,
) -> Result<usize, SinkFull> {
    write_scalar(sink, &value.to_bits().to_be_bytes())
}

/// Append the eight IEEE-754 octets of a Double in big-endian order without a tag.
///
/// Uses one atomic write, with the [scalar sink contract](super).
pub fn encode_double_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: f64,
) -> Result<usize, SinkFull> {
    write_scalar(sink, &value.to_bits().to_be_bytes())
}

/// Append an application-tagged Null with no contents.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_null_to_sink<S: EncodeSink + ?Sized>(sink: &mut S) -> Result<usize, SinkFull> {
    write_tagged(sink, app_tag::NULL, TagClass::Application, 0, &[])
}

/// Append an application-tagged Boolean with its value in the header and no contents.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_boolean_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: bool,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::BOOLEAN,
        TagClass::Application,
        u32::from(value),
        &[],
    )
}

/// Append an application-tagged Unsigned with its minimum-width contents.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_unsigned_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: u64,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::UNSIGNED,
        TagClass::Application,
        unsigned_len(value),
        &value.to_be_bytes()[8 - unsigned_len(value) as usize..],
    )
}

/// Append an application-tagged Signed with its minimum-width contents.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_signed_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: i32,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::SIGNED,
        TagClass::Application,
        signed_len(value),
        &value.to_be_bytes()[4 - signed_len(value) as usize..],
    )
}

/// Append an application-tagged Real preserving every payload bit.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_real_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: f32,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::REAL,
        TagClass::Application,
        4,
        &value.to_bits().to_be_bytes(),
    )
}

/// Append an application-tagged Double preserving every payload bit.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_double_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: f64,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::DOUBLE,
        TagClass::Application,
        8,
        &value.to_bits().to_be_bytes(),
    )
}

/// Append an application-tagged Enumerated with its minimum-width contents.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_enumerated_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: u32,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::ENUMERATED,
        TagClass::Application,
        unsigned_len(u64::from(value)),
        &value.to_be_bytes()[4 - unsigned_len(u64::from(value)) as usize..],
    )
}

/// Append an application-tagged Date preserving its four fields without validation.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_date_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    date: &Date,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::DATE,
        TagClass::Application,
        4,
        &date.encode(),
    )
}

/// Append an application-tagged Time preserving its four fields without validation.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_time_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    time: &Time,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::TIME,
        TagClass::Application,
        4,
        &time.encode(),
    )
}

/// Append an application-tagged ObjectIdentifier preserving its 10-bit type and 22-bit instance.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
pub fn encode_app_object_id_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    oid: &ObjectIdentifier,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::OBJECT_IDENTIFIER,
        TagClass::Application,
        4,
        &oid.encode(),
    )
}

/// Append a context-tagged Unsigned with its minimum-width contents.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
/// The tag number keeps its existing unvalidated input behavior.
///
/// ```
/// use bacnet_encoding::primitives::encode_ctx_unsigned_to_sink;
///
/// let mut storage = [0xA5; 4];
/// let mut tail = &mut storage[..];
/// assert_eq!(encode_ctx_unsigned_to_sink(&mut tail, 15, 256)?, 4);
/// assert!(tail.is_empty());
/// assert_eq!(storage, [0xFA, 0x0F, 0x01, 0x00]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub fn encode_ctx_unsigned_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    value: u64,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        tag,
        TagClass::Context,
        unsigned_len(value),
        &value.to_be_bytes()[8 - unsigned_len(value) as usize..],
    )
}

/// Append a context-tagged Signed with its minimum-width contents.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
/// The tag number keeps its existing unvalidated input behavior.
pub fn encode_ctx_signed_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    value: i32,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        tag,
        TagClass::Context,
        signed_len(value),
        &value.to_be_bytes()[4 - signed_len(value) as usize..],
    )
}

/// Append a context-tagged Real preserving every payload bit.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
/// The tag number keeps its existing unvalidated input behavior.
pub fn encode_ctx_real_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    value: f32,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        tag,
        TagClass::Context,
        4,
        &value.to_bits().to_be_bytes(),
    )
}

/// Append a context-tagged Double preserving every payload bit.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
/// The tag number keeps its existing unvalidated input behavior.
pub fn encode_ctx_double_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    value: f64,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        tag,
        TagClass::Context,
        8,
        &value.to_bits().to_be_bytes(),
    )
}

/// Append a context-tagged Enumerated with its minimum-width contents.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
/// The tag number keeps its existing unvalidated input behavior.
pub fn encode_ctx_enumerated_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    value: u32,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        tag,
        TagClass::Context,
        unsigned_len(u64::from(value)),
        &value.to_be_bytes()[4 - unsigned_len(u64::from(value)) as usize..],
    )
}

/// Append a context-tagged Boolean with one content octet.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
/// The tag number keeps its existing unvalidated input behavior.
pub fn encode_ctx_boolean_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    value: bool,
) -> Result<usize, SinkFull> {
    write_tagged(sink, tag, TagClass::Context, 1, &[u8::from(value)])
}

/// Append a context-tagged ObjectIdentifier preserving its 10-bit type and 22-bit instance.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
/// The tag number keeps its existing unvalidated input behavior.
pub fn encode_ctx_object_id_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    oid: &ObjectIdentifier,
) -> Result<usize, SinkFull> {
    write_tagged(sink, tag, TagClass::Context, 4, &oid.encode())
}

/// Append a context-tagged Date preserving its four fields without validation.
///
/// Stages the whole value for one write, with the [scalar sink contract](super).
/// The tag number keeps its existing unvalidated input behavior.
pub fn encode_ctx_date_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    date: &Date,
) -> Result<usize, SinkFull> {
    write_tagged(sink, tag, TagClass::Context, 4, &date.encode())
}

// Scalar contents are at most eight bytes. Their header needs at most
// three: an extended context tag and an extended one-byte content length.
fn write_tagged<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    class: TagClass,
    length: u32,
    contents: &[u8],
) -> Result<usize, SinkFull> {
    let mut encoded = [0; 11];
    let header = tags::encode_tag_to_sink(&mut &mut encoded[..], tag, class, length)
        .expect("a scalar header fits the eleven-byte staging buffer");
    let used = header + contents.len();
    encoded[header..used].copy_from_slice(contents);
    write_scalar(sink, &encoded[..used])
}

fn write_scalar<S: EncodeSink + ?Sized>(sink: &mut S, encoded: &[u8]) -> Result<usize, SinkFull> {
    if let Some(available) = sink.remaining_capacity() {
        if available < encoded.len() {
            return Err(SinkFull {
                needed: encoded.len(),
                available,
            });
        }
    }
    sink.write(encoded)?;
    Ok(encoded.len())
}
