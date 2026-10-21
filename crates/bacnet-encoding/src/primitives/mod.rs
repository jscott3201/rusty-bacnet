//! Application-tagged primitive encode/decode per ASHRAE 135-2020 Clause 20.2.
//!
//! Provides both raw value codecs (no tag header) and application/context-tagged
//! convenience functions for all BACnet primitive types.
//!
//! The bounded scalar encoders also have `_to_sink` entrypoints accepting
//! [`crate::sink::EncodeSink`]. Each appends its complete value, including any
//! tag header, in one atomic write and returns the appended byte count. Known
//! capacity shortages never call the sink; a custom sink's error is unchanged.
//! The scalar staging buffer is at most eleven bytes. These functions preserve
//! the existing encoders' input semantics and do not add value validation.
//!
//! [`encode_timestamp_choice_to_sink`] and [`encode_timestamp_to_sink`] extend
//! the same one-write contract to complete timestamps, with twelve-byte and
//! sixteen-byte staging bounds respectively. These bounds describe encoded
//! output, not the full call stack.
//!
//! The CharacterString, OCTET STRING and BIT STRING `_to_sink` encoders write
//! a header and a caller's payload as one value with
//! [`EncodeSink::write_parts`](crate::sink::EncodeSink::write_parts).
//! [`CharacterStringRef`] and [`ApplicationValueRef`] read strings and
//! application-tagged values without copying them. Those, the scalar
//! decoders and the sink encoders need no allocator; the `BytesMut` encoders
//! and the owned `PropertyValue` decoders need `alloc`.

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::{string::String, vec::Vec};

use bacnet_types::error::{DecodeError, DecodingKind, Error};
#[cfg(feature = "alloc")]
use bacnet_types::primitives::PropertyValue;
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};
#[cfg(feature = "alloc")]
use bytes::{BufMut, BytesMut};

#[cfg(feature = "alloc")]
use crate::constructed::tagged::misplaced_tag;
use crate::constructed::tagged::{misplaced_kind, unclosed_kind};
#[cfg(feature = "alloc")]
use crate::tags::TagClass;
use crate::tags::{self, app_tag};

mod scalar;
mod strings;
mod timestamp;
mod view;

pub use strings::{
    encode_app_bit_string_to_sink, encode_app_character_string_to_sink,
    encode_app_octet_string_to_sink, encode_bit_string_to_sink, encode_character_string_to_sink,
    encode_ctx_bit_string_to_sink, encode_ctx_character_string_to_sink,
    encode_ctx_octet_string_to_sink,
};
pub use timestamp::{encode_timestamp_choice_to_sink, encode_timestamp_to_sink};
pub use view::{decode_application_value_ref, ApplicationValueRef, CharacterStringRef};

#[cfg(feature = "alloc")]
pub use scalar::{
    encode_app_boolean, encode_app_date, encode_app_double, encode_app_enumerated, encode_app_null,
    encode_app_object_id, encode_app_real, encode_app_signed, encode_app_time, encode_app_unsigned,
    encode_ctx_boolean, encode_ctx_date, encode_ctx_double, encode_ctx_enumerated,
    encode_ctx_object_id, encode_ctx_real, encode_ctx_signed, encode_ctx_unsigned, encode_double,
    encode_real, encode_signed, encode_unsigned,
};
pub use scalar::{
    encode_app_boolean_to_sink, encode_app_date_to_sink, encode_app_double_to_sink,
    encode_app_enumerated_to_sink, encode_app_null_to_sink, encode_app_object_id_to_sink,
    encode_app_real_to_sink, encode_app_signed_to_sink, encode_app_time_to_sink,
    encode_app_unsigned_to_sink, encode_ctx_boolean_to_sink, encode_ctx_date_to_sink,
    encode_ctx_double_to_sink, encode_ctx_enumerated_to_sink, encode_ctx_object_id_to_sink,
    encode_ctx_real_to_sink, encode_ctx_signed_to_sink, encode_ctx_unsigned_to_sink,
    encode_double_to_sink, encode_real_to_sink, encode_signed_to_sink, encode_unsigned_to_sink,
    signed_len, unsigned_len,
};

/// A fault in contents decoded at offset 0 of the slice they were given,
/// naming `value`.
#[cold]
fn contents_fault(kind: DecodingKind, label: &'static str, value: u64) -> Error {
    DecodeError::new(kind, 0, label).with_value(value).into()
}

// ===========================================================================
// Raw value codecs (no tag header)
// ===========================================================================

// --- Unsigned Integer ---

/// Decode an unsigned integer from big-endian bytes (1-8 bytes).
pub fn decode_unsigned(data: &[u8]) -> Result<u64, Error> {
    if data.is_empty() || data.len() > 8 {
        return Err(contents_fault(
            DecodingKind::InvalidEncoding,
            "unsigned requires 1-8 bytes, got {}",
            data.len() as u64,
        ));
    }
    let mut value: u64 = 0;
    for &b in data {
        value = (value << 8) | b as u64;
    }
    Ok(value)
}

/// Decode an unsigned integer that fits in a `u8` from 1-8 big-endian bytes.
/// Leading zero bytes are accepted; values above `u8::MAX` are rejected.
pub fn decode_unsigned_u8(data: &[u8]) -> Result<u8, Error> {
    let value = decode_unsigned(data)?;
    u8::try_from(value).map_err(|_| {
        contents_fault(
            DecodingKind::OutOfRange,
            "unsigned value {} exceeds u8",
            value,
        )
    })
}

/// Decode an unsigned integer that fits in a `u16` from 1-8 big-endian bytes.
/// Leading zero bytes are accepted; values above `u16::MAX` are rejected.
pub fn decode_unsigned_u16(data: &[u8]) -> Result<u16, Error> {
    let value = decode_unsigned(data)?;
    u16::try_from(value).map_err(|_| {
        contents_fault(
            DecodingKind::OutOfRange,
            "unsigned value {} exceeds u16",
            value,
        )
    })
}

/// Decode an unsigned integer that fits in a `u32` from 1-8 big-endian bytes.
/// Leading zero bytes are accepted; values above `u32::MAX` are rejected.
pub fn decode_unsigned_u32(data: &[u8]) -> Result<u32, Error> {
    let value = decode_unsigned(data)?;
    u32::try_from(value).map_err(|_| {
        contents_fault(
            DecodingKind::OutOfRange,
            "unsigned value {} exceeds u32",
            value,
        )
    })
}

// --- Signed Integer ---

/// Decode a signed integer from two's-complement big-endian bytes (1-4 bytes).
pub fn decode_signed(data: &[u8]) -> Result<i32, Error> {
    if data.is_empty() || data.len() > 4 {
        return Err(contents_fault(
            DecodingKind::InvalidEncoding,
            "signed requires 1-4 bytes, got {}",
            data.len() as u64,
        ));
    }
    let sign_extend = if data[0] & 0x80 != 0 { 0xFF } else { 0x00 };
    let mut bytes = [sign_extend; 4];
    bytes[4 - data.len()..].copy_from_slice(data);
    Ok(i32::from_be_bytes(bytes))
}

#[cfg(feature = "alloc")]
pub(crate) fn decode_signed_canonical(data: &[u8]) -> Result<i32, Error> {
    if data.len() > 1
        && ((data[0] == 0 && data[1] & 0x80 == 0) || (data[0] == 0xFF && data[1] & 0x80 != 0))
    {
        return Err(Error::decoding(0, "signed contains a redundant sign octet"));
    }
    decode_signed(data)
}

// --- Real ---

/// Decode an IEEE-754 single-precision float from 4 big-endian bytes.
pub fn decode_real(data: &[u8]) -> Result<f32, Error> {
    if data.len() != 4 {
        return Err(contents_fault(
            DecodingKind::InvalidEncoding,
            "Real expects exactly 4 bytes, got {}",
            data.len() as u64,
        ));
    }
    Ok(f32::from_be_bytes([data[0], data[1], data[2], data[3]]))
}

// --- Double ---

/// Decode an IEEE-754 double-precision float from 8 big-endian bytes.
pub fn decode_double(data: &[u8]) -> Result<f64, Error> {
    if data.len() != 8 {
        return Err(contents_fault(
            DecodingKind::InvalidEncoding,
            "Double expects exactly 8 bytes, got {}",
            data.len() as u64,
        ));
    }
    let bytes: [u8; 8] = [
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ];
    Ok(f64::from_be_bytes(bytes))
}

// --- Character String ---

/// Character set identifiers.
pub mod charset {
    /// ISO 10646 (UTF-8) — X'00'
    pub const UTF8: u8 = 0;
    /// IBM/Microsoft DBCS — X'01'
    pub const IBM_MICROSOFT_DBCS: u8 = 1;
    /// JIS X 0208 — X'02'
    pub const JIS_X_0208: u8 = 2;
    /// ISO 10646 (UCS-4) — X'03'
    pub const UCS4: u8 = 3;
    /// ISO 10646 (UCS-2) — X'04'
    pub const UCS2: u8 = 4;
    /// ISO 8859-1 — X'05'
    pub const ISO_8859_1: u8 = 5;
}

/// Encode a UTF-8 character string with leading charset byte. Delegates to
/// [`encode_character_string_to_sink`].
#[cfg(feature = "alloc")]
pub fn encode_character_string(buf: &mut BytesMut, value: &str) {
    encode_character_string_to_sink(buf, value).expect("the BytesMut sink cannot return SinkFull");
}

/// Return the encoded length of a character string (charset byte + UTF-8
/// bytes). Only with `alloc`: the sink encoders refuse an over-long string
/// with [`SinkFull`](crate::sink::SinkFull) instead.
#[cfg(feature = "alloc")]
pub fn character_string_len(value: &str) -> Result<u32, Error> {
    u32::try_from(value.len())
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(character_string_too_long)
}

/// A CharacterString whose contents a header's 32-bit length can't express.
#[cfg(feature = "alloc")]
#[cold]
fn character_string_too_long() -> Error {
    Error::encode("CharacterString too long for BACnet encoding")
}

/// Decode a character string from content bytes.
///
/// The first byte is the charset identifier. Supported charsets:
/// - 0 (UTF-8)
/// - 4 (UCS-2, big-endian)
/// - 5 (ISO-8859-1)
///
/// Other charsets return an error. Only with `alloc`;
/// [`CharacterStringRef::decode`] reads the contents without converting
/// them.
#[cfg(feature = "alloc")]
pub fn decode_character_string(data: &[u8]) -> Result<String, Error> {
    // Character sets the standard defines but this decoder doesn't convert.
    if let Some(
        &unsupported @ (charset::IBM_MICROSOFT_DBCS | charset::JIS_X_0208 | charset::UCS4),
    ) = data.first()
    {
        return Err(contents_fault(
            DecodingKind::Unsupported,
            "unsupported charset: {}",
            unsupported.into(),
        ));
    }
    // The structure is checked as the borrowed view checks it (#1778), so
    // the conversions below can't fail.
    view::check_character_string(data)?;
    let payload = &data[1..];
    Ok(match data[0] {
        charset::UCS2 => payload
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|c| char::from_u32(u16::from_be_bytes(*c).into()))
            .collect(),
        charset::ISO_8859_1 => payload.iter().map(|&octet| char::from(octet)).collect(),
        _ => core::str::from_utf8(payload).unwrap_or_default().into(),
    })
}

/// UTF-8 contents that aren't valid, reported at the first contents octet
/// after the character set, naming how many octets before the fault were
/// valid.
#[cold]
fn invalid_utf8(error: core::str::Utf8Error) -> Error {
    DecodeError::new(
        DecodingKind::InvalidEncoding,
        1,
        "invalid UTF-8 after {} valid octets",
    )
    .with_value(error.valid_up_to() as u64)
    .into()
}

/// A character set Clause 20.2.9 doesn't define.
#[cold]
fn unknown_charset(charset: u8) -> Error {
    contents_fault(
        DecodingKind::InvalidEncoding,
        "unknown charset: {}",
        charset.into(),
    )
}

// --- Bit String ---

/// Encode a bit string: leading unused-bits count followed by data bytes.
/// Delegates to [`encode_bit_string_to_sink`].
#[cfg(feature = "alloc")]
pub fn encode_bit_string(buf: &mut BytesMut, unused_bits: u8, data: &[u8]) {
    encode_bit_string_to_sink(buf, unused_bits, data)
        .expect("the BytesMut sink cannot return SinkFull");
}

/// Decode a bit string from content bytes.
///
/// Returns `(unused_bits, data)`. Only with `alloc`; [`decode_bit_string_ref`]
/// borrows the data instead.
#[cfg(feature = "alloc")]
pub fn decode_bit_string(data: &[u8]) -> Result<(u8, Vec<u8>), Error> {
    let (unused, bits) = decode_bit_string_ref(data)?;
    Ok((unused, bits.to_vec()))
}

/// Decode a bit string from content bytes without copying its data octets.
///
/// Returns `(unused_bits, data)`, `data` borrowed from the contents.
pub fn decode_bit_string_ref(data: &[u8]) -> Result<(u8, &[u8]), Error> {
    let Some((&unused, bits)) = data.split_first() else {
        return Err(Error::decoding(
            0,
            "BitString requires at least 1 byte for unused-bits count",
        ));
    };
    if unused > 7 {
        return Err(contents_fault(
            DecodingKind::InvalidEncoding,
            "BitString unused_bits must be 0-7, got {}",
            unused.into(),
        ));
    }
    Ok((unused, bits))
}

// ===========================================================================
// Application-tagged encode helpers
// ===========================================================================

/// Encode an application-tagged OctetString. It keeps its own code rather
/// than delegating to [`encode_app_octet_string_to_sink`]: past `u32::MAX`
/// octets it saturates the length, where the sink encoder refuses.
#[cfg(feature = "alloc")]
pub fn encode_app_octet_string(buf: &mut BytesMut, data: &[u8]) {
    let data_len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    tags::encode_tag(buf, app_tag::OCTET_STRING, TagClass::Application, data_len);
    buf.put_slice(data);
}

/// Encode an application-tagged CharacterString (UTF-8). Delegates to
/// [`encode_app_character_string_to_sink`]; a string too long for a header's
/// 32-bit length is [`Error::Encode`], with nothing written.
#[cfg(feature = "alloc")]
pub fn encode_app_character_string(buf: &mut BytesMut, value: &str) -> Result<(), Error> {
    // A BytesMut never refuses a write, so the only refusal is the length.
    encode_app_character_string_to_sink(buf, value).map_err(|_| character_string_too_long())?;
    Ok(())
}

/// Encode an application-tagged BitString. It keeps its own code rather than
/// delegating to [`encode_app_bit_string_to_sink`]: past `u32::MAX` octets it
/// saturates the length, where the sink encoder refuses.
#[cfg(feature = "alloc")]
pub fn encode_app_bit_string(buf: &mut BytesMut, unused_bits: u8, data: &[u8]) {
    let data_len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let len = data_len.saturating_add(1);
    tags::encode_tag(buf, app_tag::BIT_STRING, TagClass::Application, len);
    encode_bit_string(buf, unused_bits, data);
}

// ===========================================================================
// Context-tagged encode helpers
// ===========================================================================

/// Encode a context-tagged OctetString. It keeps its own code rather than
/// delegating to [`encode_ctx_octet_string_to_sink`]: past `u32::MAX` octets
/// it saturates the length, where the sink encoder refuses.
#[cfg(feature = "alloc")]
pub fn encode_ctx_octet_string(buf: &mut BytesMut, tag: u8, data: &[u8]) {
    let data_len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    tags::encode_tag(buf, tag, TagClass::Context, data_len);
    buf.put_slice(data);
}

/// Encode a context-tagged CharacterString (UTF-8). Delegates to
/// [`encode_ctx_character_string_to_sink`]; a string too long for a header's
/// 32-bit length is [`Error::Encode`], with nothing written.
#[cfg(feature = "alloc")]
pub fn encode_ctx_character_string(buf: &mut BytesMut, tag: u8, value: &str) -> Result<(), Error> {
    // A BytesMut never refuses a write, so the only refusal is the length.
    encode_ctx_character_string_to_sink(buf, tag, value)
        .map_err(|_| character_string_too_long())?;
    Ok(())
}

/// Encode a context-tagged BitString. It keeps its own code rather than
/// delegating to [`encode_ctx_bit_string_to_sink`]: past `u32::MAX` octets it
/// saturates the length, where the sink encoder refuses.
#[cfg(feature = "alloc")]
pub fn encode_ctx_bit_string(buf: &mut BytesMut, tag: u8, unused_bits: u8, data: &[u8]) {
    let data_len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let len = data_len.saturating_add(1);
    tags::encode_tag(buf, tag, TagClass::Context, len);
    encode_bit_string(buf, unused_bits, data);
}

// ===========================================================================
// Application-tagged decode (dispatches by tag number)
// ===========================================================================

/// The label for an application tag of a fixed-size type whose header gives
/// another length, or `None` for a tag of no fixed size.
pub(crate) fn fixed_length_label(number: u8) -> Option<(&'static str, u32)> {
    Some(match number {
        app_tag::REAL => ("application REAL has {} contents octets, expected 4", 4),
        app_tag::DOUBLE => ("application Double has {} contents octets, expected 8", 8),
        app_tag::DATE => ("application Date has {} contents octets, expected 4", 4),
        app_tag::TIME => ("application Time has {} contents octets, expected 4", 4),
        app_tag::OBJECT_IDENTIFIER => (
            "application BACnetObjectIdentifier has {} contents octets, expected 4",
            4,
        ),
        _ => return None,
    })
}

/// Decode a single application-tagged value from `data` at `offset`.
///
/// A leading **context-tagged** element decodes to
/// [`PropertyValue::ApplicationData`] holding the complete tagged element
/// verbatim (tag header(s) through the matching closing tag for constructed
/// elements) — such bytes belong to a context-tagged ASN.1 production
/// (e.g. `BACnetEventParameter`) whose property-level framed codec must
/// interpret them. Application-tagged content is decoded into the typed
/// variants as before.
///
/// Errors follow the rule of
/// [`constructed::tagged`](crate::constructed::tagged): a fixed-size value
/// (REAL, Double, Date, Time, object identifier) whose header gives another
/// length is [`Error::Decoding`], and contents that run past the end of
/// `data` are [`Error::BufferTooShort`].
///
/// Returns the decoded `PropertyValue` and the new offset past the consumed
/// bytes. Only with `alloc`; [`decode_application_value_ref`] reads an
/// application-tagged value without copying it.
#[cfg(feature = "alloc")]
pub fn decode_application_value(
    data: &[u8],
    offset: usize,
) -> Result<(PropertyValue, usize), Error> {
    let (tag, new_offset) = tags::decode_tag(data, offset)?;
    if tag.class == TagClass::Context {
        if tag.is_opening {
            let (_content, after) = tags::extract_context_value(data, new_offset, tag.number)?;
            return Ok((
                PropertyValue::ApplicationData(data[offset..after].to_vec()),
                after,
            ));
        }
        if tag.is_closing {
            return Err(Error::invalid_tag(offset, "unexpected closing tag"));
        }
        let content_end = new_offset
            .checked_add(tag.length as usize)
            .ok_or_else(|| Error::decoding(new_offset, "length overflow"))?;
        if data.len() < content_end {
            return Err(Error::buffer_too_short(content_end, data.len()));
        }
        return Ok((
            PropertyValue::ApplicationData(data[offset..content_end].to_vec()),
            content_end,
        ));
    }
    let (value, end) = view::decode_application_primitive::<false>(data, offset, tag, new_offset)?;
    let value = match value {
        ApplicationValueRef::Null => PropertyValue::Null,
        ApplicationValueRef::Boolean(value) => PropertyValue::Boolean(value),
        ApplicationValueRef::Unsigned(value) => PropertyValue::Unsigned(value),
        ApplicationValueRef::Signed(value) => PropertyValue::Signed(value),
        ApplicationValueRef::Real(value) => PropertyValue::Real(value),
        ApplicationValueRef::Double(value) => PropertyValue::Double(value),
        ApplicationValueRef::OctetString(octets) => PropertyValue::OctetString(octets.to_vec()),
        ApplicationValueRef::CharacterString(text) => {
            PropertyValue::CharacterString(decode_character_string(text.contents())?)
        }
        ApplicationValueRef::BitString { unused_bits, data } => PropertyValue::BitString {
            unused_bits,
            data: data.to_vec(),
        },
        ApplicationValueRef::Enumerated(value) => PropertyValue::Enumerated(value),
        ApplicationValueRef::Date(value) => PropertyValue::Date(value),
        ApplicationValueRef::Time(value) => PropertyValue::Time(value),
        ApplicationValueRef::ObjectIdentifier(value) => PropertyValue::ObjectIdentifier(value),
    };
    Ok((value, end))
}

/// Check that an application-tagged value starts at `offset` and is structurally well formed,
/// returning the offset just past it.
///
/// Unlike [`decode_application_value`] this accepts a CharacterString in any character set
/// defined by Clause 20.2.9 (including DBCS and JIS X 0208, which [`decode_application_value`]
/// rejects) and does not build a [`PropertyValue`]. Errors carry absolute offsets into
/// `data`. Only with `alloc`.
#[cfg(feature = "alloc")]
pub fn validate_application_value(data: &[u8], offset: usize) -> Result<usize, Error> {
    let (tag, content_start) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application || tag.is_opening || tag.is_closing {
        return Err(misplaced_tag(
            data,
            &tag,
            None,
            offset,
            "expected an application-tagged value",
        ));
    }
    if tag.number != app_tag::CHARACTER_STRING && tag.number != app_tag::SIGNED {
        return decode_application_value(data, offset).map(|(_, end)| end);
    }

    let content_end = content_start
        .checked_add(tag.length as usize)
        .ok_or_else(|| Error::decoding(content_start, "length overflow"))?;
    if content_end > data.len() {
        return Err(Error::buffer_too_short(content_end, data.len()));
    }
    let content = &data[content_start..content_end];
    if tag.number == app_tag::SIGNED {
        decode_signed_canonical(content)?;
        return Ok(content_end);
    }
    view::check_character_string(content).map_err(view::rebased(content_start))?;
    Ok(content_end)
}

/// Encode a `PropertyValue` as an application-tagged value. Only with
/// `alloc`.
#[cfg(feature = "alloc")]
pub fn encode_property_value(buf: &mut BytesMut, value: &PropertyValue) -> Result<(), Error> {
    match value {
        PropertyValue::Null => encode_app_null(buf),
        PropertyValue::Boolean(v) => encode_app_boolean(buf, *v),
        PropertyValue::Unsigned(v) => encode_app_unsigned(buf, *v),
        PropertyValue::Signed(v) => encode_app_signed(buf, *v),
        PropertyValue::Real(v) => encode_app_real(buf, *v),
        PropertyValue::Double(v) => encode_app_double(buf, *v),
        PropertyValue::OctetString(v) => encode_app_octet_string(buf, v),
        PropertyValue::CharacterString(v) => encode_app_character_string(buf, v)?,
        PropertyValue::BitString { unused_bits, data } => {
            encode_app_bit_string(buf, *unused_bits, data)
        }
        PropertyValue::Enumerated(v) => encode_app_enumerated(buf, *v),
        PropertyValue::Date(v) => encode_app_date(buf, v),
        PropertyValue::Time(v) => encode_app_time(buf, v),
        PropertyValue::ObjectIdentifier(v) => encode_app_object_id(buf, v),
        PropertyValue::List(values) => {
            for v in values {
                encode_property_value(buf, v)?;
            }
        }
        PropertyValue::ApplicationData(bytes) => {
            // Already-encoded application-layer content (a context-tagged
            // CHOICE/SEQUENCE production) is emitted verbatim.
            buf.put_slice(bytes);
        }
    }
    Ok(())
}

// ===========================================================================
// BACnetTimeStamp encode/decode
// ===========================================================================

/// Largest encodable `sequence-number` value: the production constrains it to
/// `Unsigned (0..65535)`.
///
/// `BACnetTimeStamp` (ASHRAE 135-2020 Clause 21) is a three-way CHOICE: a Time
/// under context tag 0, this bounded sequence number under tag 1, or a
/// `BACnetDateTime` under tag 2.
pub const MAX_TIMESTAMP_SEQUENCE_NUMBER: u64 = 65535;

/// Encode a `BACnetTimeStamp` as a bare CHOICE element (no enclosing field
/// tag). Delegates to [`encode_timestamp_choice_to_sink`] through the
/// growable buffer; the existing result type is retained.
///
/// Used where a production lists `BACnetTimeStamp` as a bare CHOICE item —
/// e.g. the `eventTimeStamps [3] SEQUENCE OF BACnetTimeStamp` inside
/// GetEventInformation-ACK. [`encode_timestamp`] wraps this in
/// the enclosing field's context tag pair instead.
///
/// Tag forms per Clause 20.2.1.5: `time [0]` is the CHOICE's only alternative
/// over a *primitive* base type, so it encodes as a context-specific
/// *primitive* tag 0 of length 4 holding the raw `Time` octets (the
/// opening-tag-0 wrapper around an application-tagged `Time` is NOT
/// conformant); `sequence-number [1]` is a primitive context tag 1 (contents
/// constrained to `0..=65535` by the public variant's `u16` payload);
/// `datetime [2]` tags the *constructed* `BACnetDateTime`, so it encodes as opening tag 2 /
/// application-tagged `Date` / application-tagged `Time` / closing tag 2.
#[cfg(feature = "alloc")]
pub fn encode_timestamp_choice(buf: &mut BytesMut, ts: &BACnetTimeStamp) -> Result<(), Error> {
    encode_timestamp_choice_to_sink(buf, ts).expect("the BytesMut sink cannot return SinkFull");
    Ok(())
}

/// Decode a bare-CHOICE `BACnetTimeStamp` starting at `offset`.
///
/// Returns the decoded timestamp and the new offset past its encoding. Only
/// the Clause 20.2.1.5-conformant tag forms described on
/// `encode_timestamp_choice` are accepted; `sequence-number` contents
/// beyond `0..=65535` are rejected. Errors follow the rule of
/// [`constructed::tagged`](crate::constructed::tagged): a Time or Date with
/// the wrong length is [`Error::Decoding`], contents that run past the end of
/// `data` are [`Error::BufferTooShort`].
pub fn decode_timestamp_choice(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetTimeStamp, usize), Error> {
    use crate::constructed::tagged;
    const WHAT: &str = "BACnetTimeStamp";

    let (inner_tag, inner_pos) = tags::decode_tag(data, offset)?;
    if inner_tag.is_context(0) {
        let (octets, end) = tagged::decode_ctx_fixed(data, offset, 0, tagged::Fixed::TIME, WHAT)?;
        return Ok((BACnetTimeStamp::Time(Time::decode(octets)?), end));
    }
    if inner_tag.is_context(1) {
        let (octets, end) = tagged::decode_ctx_primitive(data, offset, 1, WHAT)?;
        let n = decode_unsigned(octets)?;
        if n > MAX_TIMESTAMP_SEQUENCE_NUMBER {
            return Err(DecodeError::new(
                DecodingKind::OutOfRange,
                inner_pos,
                "BACnetTimeStamp sequence-number {} exceeds 65535",
            )
            .with_value(n)
            .into());
        }
        let n = u16::try_from(n).expect("BACnetTimeStamp sequence range checked above");
        return Ok((BACnetTimeStamp::SequenceNumber(n), end));
    }
    if inner_tag.is_opening_tag(2) {
        const DATE_TIME: &str = "BACnetTimeStamp DateTime";
        let (date, date_end) =
            tagged::decode_app_fixed(data, inner_pos, app_tag::DATE, 4, DATE_TIME)?;
        let (time, time_end) =
            tagged::decode_app_fixed(data, date_end, app_tag::TIME, 4, DATE_TIME)?;
        let end = tagged::expect_closing(data, time_end, 2, DATE_TIME)?;
        let date_time = BACnetTimeStamp::DateTime {
            date: Date::decode(date)?,
            time: Time::decode(time)?,
        };
        return Ok((date_time, end));
    }
    Err(DecodeError::new(
        misplaced_kind(data, offset, &inner_tag, None),
        offset,
        "BACnetTimeStamp: unexpected inner choice tag",
    )
    .into())
}

/// Encode a BACnetTimeStamp wrapped in a context opening/closing tag pair.
///
/// Delegates to [`encode_timestamp_to_sink`] through the growable buffer,
/// retaining the existing result type.
///
/// The outer `tag_number` is the context tag of the enclosing field.
/// Inside, the CHOICE variant uses its own context tag (0=Time,
/// 1=SequenceNumber, 2=DateTime) via [`encode_timestamp_choice`].
#[cfg(feature = "alloc")]
pub fn encode_timestamp(
    buf: &mut BytesMut,
    tag_number: u8,
    ts: &BACnetTimeStamp,
) -> Result<(), Error> {
    encode_timestamp_to_sink(buf, tag_number, ts)
        .expect("the BytesMut sink cannot return SinkFull");
    Ok(())
}

/// Decode a BACnetTimeStamp from inside a context opening/closing tag pair.
///
/// `data` should point to the start of the outer opening tag for `tag_number`.
/// Returns the decoded timestamp and the new offset past the outer closing tag.
pub fn decode_timestamp(
    data: &[u8],
    offset: usize,
    tag_number: u8,
) -> Result<(BACnetTimeStamp, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if !tag.is_opening_tag(tag_number) {
        return Err(DecodeError::new(
            misplaced_kind(data, offset, &tag, Some(tag_number)),
            offset,
            "expected opening tag {} for BACnetTimeStamp",
        )
        .with_value(tag_number.into())
        .into());
    }

    let (ts, after_inner) = decode_timestamp_choice(data, pos)?;

    let (close, final_pos) = tags::decode_tag(data, after_inner)?;
    if !close.is_closing_tag(tag_number) {
        return Err(DecodeError::new(
            unclosed_kind(&close),
            after_inner,
            "expected closing tag {} for BACnetTimeStamp",
        )
        .with_value(tag_number.into())
        .into());
    }

    Ok((ts, final_pos))
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(all(test, feature = "alloc"))]
mod tests;

#[cfg(all(test, feature = "alloc"))]
mod length_first_tests;
