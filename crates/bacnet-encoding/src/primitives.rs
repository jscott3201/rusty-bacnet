//! Application-tagged primitive encode/decode per ASHRAE 135-2020 Clause 20.2.
//!
//! Provides both raw value codecs (no tag header) and application/context-tagged
//! convenience functions for all BACnet primitive types.

use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};
use bytes::{BufMut, BytesMut};

use crate::tags::{self, app_tag, TagClass};

// ===========================================================================
// Raw value codecs (no tag header)
// ===========================================================================

// --- Unsigned Integer (Clause 20.2.4) ---

/// Encode an unsigned integer using the minimum number of big-endian octets.
pub fn encode_unsigned(buf: &mut BytesMut, value: u64) {
    if value <= 0xFF {
        buf.put_u8(value as u8);
    } else if value <= 0xFFFF {
        buf.put_u16(value as u16);
    } else if value <= 0xFF_FFFF {
        buf.put_u8((value >> 16) as u8);
        buf.put_u16(value as u16);
    } else if value <= 0xFFFF_FFFF {
        buf.put_u32(value as u32);
    } else if value <= 0xFF_FFFF_FFFF {
        buf.put_u8((value >> 32) as u8);
        buf.put_u32(value as u32);
    } else if value <= 0xFFFF_FFFF_FFFF {
        buf.put_u16((value >> 32) as u16);
        buf.put_u32(value as u32);
    } else if value <= 0xFF_FFFF_FFFF_FFFF {
        buf.put_u8((value >> 48) as u8);
        buf.put_u16((value >> 32) as u16);
        buf.put_u32(value as u32);
    } else {
        buf.put_u64(value);
    }
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

/// Decode an unsigned integer from big-endian bytes (1-8 bytes).
pub fn decode_unsigned(data: &[u8]) -> Result<u64, Error> {
    if data.is_empty() || data.len() > 8 {
        return Err(Error::Decoding {
            offset: 0,
            message: format!("unsigned requires 1-8 bytes, got {}", data.len()),
        });
    }
    let mut value: u64 = 0;
    for &b in data {
        value = (value << 8) | b as u64;
    }
    Ok(value)
}

// --- Signed Integer (Clause 20.2.5) ---

/// Encode a signed integer using minimum octets, two's complement, big-endian.
pub fn encode_signed(buf: &mut BytesMut, value: i32) {
    let n = signed_len(value);
    let bytes = value.to_be_bytes();
    buf.put_slice(&bytes[4 - n as usize..]);
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

/// Decode a signed integer from two's-complement big-endian bytes (1-4 bytes).
pub fn decode_signed(data: &[u8]) -> Result<i32, Error> {
    if data.is_empty() || data.len() > 4 {
        return Err(Error::Decoding {
            offset: 0,
            message: format!("signed requires 1-4 bytes, got {}", data.len()),
        });
    }
    let sign_extend = if data[0] & 0x80 != 0 { 0xFF } else { 0x00 };
    let mut bytes = [sign_extend; 4];
    bytes[4 - data.len()..].copy_from_slice(data);
    Ok(i32::from_be_bytes(bytes))
}

// --- Real (Clause 20.2.6) ---

/// Encode an IEEE-754 single-precision float (big-endian, 4 bytes).
pub fn encode_real(buf: &mut BytesMut, value: f32) {
    buf.put_f32(value);
}

/// Decode an IEEE-754 single-precision float from 4 big-endian bytes.
pub fn decode_real(data: &[u8]) -> Result<f32, Error> {
    if data.len() < 4 {
        return Err(Error::buffer_too_short(4, data.len()));
    }
    Ok(f32::from_be_bytes([data[0], data[1], data[2], data[3]]))
}

// --- Double (Clause 20.2.7) ---

/// Encode an IEEE-754 double-precision float (big-endian, 8 bytes).
pub fn encode_double(buf: &mut BytesMut, value: f64) {
    buf.put_f64(value);
}

/// Decode an IEEE-754 double-precision float from 8 big-endian bytes.
pub fn decode_double(data: &[u8]) -> Result<f64, Error> {
    if data.len() < 8 {
        return Err(Error::buffer_too_short(8, data.len()));
    }
    let bytes: [u8; 8] = [
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ];
    Ok(f64::from_be_bytes(bytes))
}

// --- Character String (Clause 20.2.9) ---

/// Character set identifiers per Clause 20.2.9.
pub mod charset {
    pub const UTF8: u8 = 0;
    pub const JIS_X0201: u8 = 1;
    pub const JIS_C6226: u8 = 2;
    pub const UCS4: u8 = 3;
    pub const UCS2: u8 = 4;
    pub const ISO_8859_1: u8 = 5;
}

/// Encode a UTF-8 character string with leading charset byte.
pub fn encode_character_string(buf: &mut BytesMut, value: &str) {
    buf.put_u8(charset::UTF8);
    buf.put_slice(value.as_bytes());
}

/// Return the encoded length of a character string (charset byte + UTF-8 bytes).
pub fn character_string_len(value: &str) -> Result<u32, Error> {
    u32::try_from(value.len())
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| Error::Encoding("CharacterString too long for BACnet encoding".into()))
}

/// Decode a character string from content bytes.
///
/// The first byte is the charset identifier. Supported charsets:
/// - 0 (UTF-8)
/// - 4 (UCS-2, big-endian)
/// - 5 (ISO-8859-1)
///
/// Charsets 1 (DBCS/JIS X 0201), 2 (JIS C 6226), and 3 (UCS-4)
/// return an error.
pub fn decode_character_string(data: &[u8]) -> Result<String, Error> {
    if data.is_empty() {
        return Err(Error::Decoding {
            offset: 0,
            message: "CharacterString requires at least 1 byte for charset".into(),
        });
    }
    let charset_id = data[0];
    let payload = &data[1..];
    match charset_id {
        charset::UTF8 => String::from_utf8(payload.to_vec()).map_err(|e| Error::Decoding {
            offset: 1,
            message: format!("invalid UTF-8: {e}"),
        }),
        charset::UCS2 => {
            // UCS-2 big-endian → UTF-8
            if !payload.len().is_multiple_of(2) {
                return Err(Error::Decoding {
                    offset: 1,
                    message: "UCS-2 data must have even length".into(),
                });
            }
            let mut s = String::new();
            for (i, chunk) in payload.chunks_exact(2).enumerate() {
                let code_point = u16::from_be_bytes([chunk[0], chunk[1]]);
                if let Some(c) = char::from_u32(code_point as u32) {
                    s.push(c);
                } else {
                    return Err(Error::Decoding {
                        offset: 1 + i * 2,
                        message: "invalid UCS-2 code point".into(),
                    });
                }
            }
            Ok(s)
        }
        charset::ISO_8859_1 => {
            // ISO-8859-1 maps 1:1 to Unicode code points 0-255
            Ok(payload.iter().map(|&b| b as char).collect())
        }
        charset::JIS_X0201 | charset::JIS_C6226 | charset::UCS4 => Err(Error::Decoding {
            offset: 0,
            message: format!("unsupported charset: {charset_id}"),
        }),
        other => Err(Error::Decoding {
            offset: 0,
            message: format!("unknown charset: {other}"),
        }),
    }
}

// --- Bit String (Clause 20.2.10) ---

/// Encode a bit string: leading unused-bits count followed by data bytes.
pub fn encode_bit_string(buf: &mut BytesMut, unused_bits: u8, data: &[u8]) {
    buf.put_u8(unused_bits);
    buf.put_slice(data);
}

/// Decode a bit string from content bytes.
///
/// Returns `(unused_bits, data)`.
pub fn decode_bit_string(data: &[u8]) -> Result<(u8, Vec<u8>), Error> {
    if data.is_empty() {
        return Err(Error::Decoding {
            offset: 0,
            message: "BitString requires at least 1 byte for unused-bits count".into(),
        });
    }
    let unused = data[0];
    if unused > 7 {
        return Err(Error::Decoding {
            offset: 0,
            message: format!("BitString unused_bits must be 0-7, got {unused}"),
        });
    }
    Ok((unused, data[1..].to_vec()))
}

// ===========================================================================
// Application-tagged encode helpers
// ===========================================================================

/// Encode an application-tagged Null.
pub fn encode_app_null(buf: &mut BytesMut) {
    tags::encode_tag(buf, app_tag::NULL, TagClass::Application, 0);
}

/// Encode an application-tagged Boolean.
///
/// Per Clause 20.2.3, the value is encoded in the tag's L/V/T bits
/// with no content octets.
pub fn encode_app_boolean(buf: &mut BytesMut, value: bool) {
    tags::encode_tag(
        buf,
        app_tag::BOOLEAN,
        TagClass::Application,
        if value { 1 } else { 0 },
    );
}

/// Encode an application-tagged Unsigned.
pub fn encode_app_unsigned(buf: &mut BytesMut, value: u64) {
    let len = unsigned_len(value);
    tags::encode_tag(buf, app_tag::UNSIGNED, TagClass::Application, len);
    encode_unsigned(buf, value);
}

/// Encode an application-tagged Signed.
pub fn encode_app_signed(buf: &mut BytesMut, value: i32) {
    let len = signed_len(value);
    tags::encode_tag(buf, app_tag::SIGNED, TagClass::Application, len);
    encode_signed(buf, value);
}

/// Encode an application-tagged Real (f32).
pub fn encode_app_real(buf: &mut BytesMut, value: f32) {
    tags::encode_tag(buf, app_tag::REAL, TagClass::Application, 4);
    encode_real(buf, value);
}

/// Encode an application-tagged Double (f64).
pub fn encode_app_double(buf: &mut BytesMut, value: f64) {
    tags::encode_tag(buf, app_tag::DOUBLE, TagClass::Application, 8);
    encode_double(buf, value);
}

/// Encode an application-tagged OctetString.
pub fn encode_app_octet_string(buf: &mut BytesMut, data: &[u8]) {
    tags::encode_tag(
        buf,
        app_tag::OCTET_STRING,
        TagClass::Application,
        data.len() as u32,
    );
    buf.put_slice(data);
}

/// Encode an application-tagged CharacterString (UTF-8).
pub fn encode_app_character_string(buf: &mut BytesMut, value: &str) -> Result<(), Error> {
    let len = character_string_len(value)?;
    tags::encode_tag(buf, app_tag::CHARACTER_STRING, TagClass::Application, len);
    encode_character_string(buf, value);
    Ok(())
}

/// Encode an application-tagged BitString.
pub fn encode_app_bit_string(buf: &mut BytesMut, unused_bits: u8, data: &[u8]) {
    let len = 1 + data.len() as u32;
    tags::encode_tag(buf, app_tag::BIT_STRING, TagClass::Application, len);
    encode_bit_string(buf, unused_bits, data);
}

/// Encode an application-tagged Enumerated.
pub fn encode_app_enumerated(buf: &mut BytesMut, value: u32) {
    let len = unsigned_len(value as u64);
    tags::encode_tag(buf, app_tag::ENUMERATED, TagClass::Application, len);
    encode_unsigned(buf, value as u64);
}

/// Encode an application-tagged Date.
pub fn encode_app_date(buf: &mut BytesMut, date: &Date) {
    tags::encode_tag(buf, app_tag::DATE, TagClass::Application, 4);
    buf.put_slice(&date.encode());
}

/// Encode an application-tagged Time.
pub fn encode_app_time(buf: &mut BytesMut, time: &Time) {
    tags::encode_tag(buf, app_tag::TIME, TagClass::Application, 4);
    buf.put_slice(&time.encode());
}

/// Encode an application-tagged ObjectIdentifier.
pub fn encode_app_object_id(buf: &mut BytesMut, oid: &ObjectIdentifier) {
    tags::encode_tag(buf, app_tag::OBJECT_IDENTIFIER, TagClass::Application, 4);
    buf.put_slice(&oid.encode());
}

// ===========================================================================
// Context-tagged encode helpers
// ===========================================================================

/// Encode a context-tagged Unsigned.
pub fn encode_ctx_unsigned(buf: &mut BytesMut, tag: u8, value: u64) {
    let len = unsigned_len(value);
    tags::encode_tag(buf, tag, TagClass::Context, len);
    encode_unsigned(buf, value);
}

/// Encode a context-tagged Signed.
pub fn encode_ctx_signed(buf: &mut BytesMut, tag: u8, value: i32) {
    let len = signed_len(value);
    tags::encode_tag(buf, tag, TagClass::Context, len);
    encode_signed(buf, value);
}

/// Encode a context-tagged Real (f32).
pub fn encode_ctx_real(buf: &mut BytesMut, tag: u8, value: f32) {
    tags::encode_tag(buf, tag, TagClass::Context, 4);
    encode_real(buf, value);
}

/// Encode a context-tagged Double (f64).
pub fn encode_ctx_double(buf: &mut BytesMut, tag: u8, value: f64) {
    tags::encode_tag(buf, tag, TagClass::Context, 8);
    encode_double(buf, value);
}

/// Encode a context-tagged Enumerated.
pub fn encode_ctx_enumerated(buf: &mut BytesMut, tag: u8, value: u32) {
    let len = unsigned_len(value as u64);
    tags::encode_tag(buf, tag, TagClass::Context, len);
    encode_unsigned(buf, value as u64);
}

/// Encode a context-tagged Boolean.
///
/// Context-tagged booleans use a 1-byte content octet (unlike application-tagged).
pub fn encode_ctx_boolean(buf: &mut BytesMut, tag: u8, value: bool) {
    tags::encode_tag(buf, tag, TagClass::Context, 1);
    buf.put_u8(if value { 1 } else { 0 });
}

/// Encode a context-tagged ObjectIdentifier.
pub fn encode_ctx_object_id(buf: &mut BytesMut, tag: u8, oid: &ObjectIdentifier) {
    tags::encode_tag(buf, tag, TagClass::Context, 4);
    buf.put_slice(&oid.encode());
}

/// Encode a context-tagged OctetString.
pub fn encode_ctx_octet_string(buf: &mut BytesMut, tag: u8, data: &[u8]) {
    tags::encode_tag(buf, tag, TagClass::Context, data.len() as u32);
    buf.put_slice(data);
}

/// Encode a context-tagged CharacterString (UTF-8).
pub fn encode_ctx_character_string(buf: &mut BytesMut, tag: u8, value: &str) -> Result<(), Error> {
    let len = character_string_len(value)?;
    tags::encode_tag(buf, tag, TagClass::Context, len);
    encode_character_string(buf, value);
    Ok(())
}

/// Encode a context-tagged Date.
pub fn encode_ctx_date(buf: &mut BytesMut, tag: u8, date: &Date) {
    tags::encode_tag(buf, tag, TagClass::Context, 4);
    buf.put_slice(&date.encode());
}

/// Encode a context-tagged BitString.
pub fn encode_ctx_bit_string(buf: &mut BytesMut, tag: u8, unused_bits: u8, data: &[u8]) {
    let len = 1 + data.len() as u32;
    tags::encode_tag(buf, tag, TagClass::Context, len);
    encode_bit_string(buf, unused_bits, data);
}

// ===========================================================================
// Application-tagged decode (dispatches by tag number)
// ===========================================================================

/// Decode a single application-tagged value from `data` at `offset`.
///
/// Returns the decoded `PropertyValue` and the new offset past the consumed bytes.
pub fn decode_application_value(
    data: &[u8],
    offset: usize,
) -> Result<(PropertyValue, usize), Error> {
    let (tag, new_offset) = tags::decode_tag(data, offset)?;
    if tag.class != TagClass::Application {
        return Err(Error::decoding(
            offset,
            format!("expected application tag, got context tag {}", tag.number),
        ));
    }
    if tag.is_opening || tag.is_closing {
        return Err(Error::decoding(offset, "unexpected opening/closing tag"));
    }

    let content_start = new_offset;
    let content_len = tag.length as usize;
    let content_end = content_start
        .checked_add(content_len)
        .ok_or_else(|| Error::decoding(content_start, "length overflow"))?;

    // For boolean, content_len is actually the value (0 or 1), not a byte count
    if tag.number == app_tag::BOOLEAN {
        return Ok((PropertyValue::Boolean(tag.length != 0), content_start));
    }

    if data.len() < content_end {
        return Err(Error::buffer_too_short(content_end, data.len()));
    }

    let content = &data[content_start..content_end];

    let value = match tag.number {
        app_tag::NULL => PropertyValue::Null,
        app_tag::UNSIGNED => PropertyValue::Unsigned(decode_unsigned(content)?),
        app_tag::SIGNED => PropertyValue::Signed(decode_signed(content)?),
        app_tag::REAL => PropertyValue::Real(decode_real(content)?),
        app_tag::DOUBLE => PropertyValue::Double(decode_double(content)?),
        app_tag::OCTET_STRING => PropertyValue::OctetString(content.to_vec()),
        app_tag::CHARACTER_STRING => {
            PropertyValue::CharacterString(decode_character_string(content)?)
        }
        app_tag::BIT_STRING => {
            let (unused, bits) = decode_bit_string(content)?;
            PropertyValue::BitString {
                unused_bits: unused,
                data: bits,
            }
        }
        app_tag::ENUMERATED => PropertyValue::Enumerated(decode_unsigned(content)? as u32),
        app_tag::DATE => PropertyValue::Date(Date::decode(content)?),
        app_tag::TIME => PropertyValue::Time(Time::decode(content)?),
        app_tag::OBJECT_IDENTIFIER => {
            PropertyValue::ObjectIdentifier(ObjectIdentifier::decode(content)?)
        }
        other => {
            return Err(Error::decoding(
                offset,
                format!("unknown application tag number {other}"),
            ));
        }
    };

    Ok((value, content_end))
}

/// Encode a `PropertyValue` as an application-tagged value.
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
    }
    Ok(())
}

// ===========================================================================
// BACnetTimeStamp encode/decode (Clause 20.2.1.5)
// ===========================================================================

/// Encode a BACnetTimeStamp wrapped in a context opening/closing tag pair.
///
/// The outer tag_number is the context tag of the field that holds the
/// timestamp (e.g., 3 for EventNotification timeStamp). Inside, the
/// CHOICE variant is encoded with its own context tag (0=Time, 1=Unsigned,
/// 2=DateTime).
pub fn encode_timestamp(buf: &mut BytesMut, tag_number: u8, ts: &BACnetTimeStamp) {
    tags::encode_opening_tag(buf, tag_number);
    match ts {
        BACnetTimeStamp::Time(t) => {
            tags::encode_tag(buf, 0, TagClass::Context, 4);
            buf.put_slice(&t.encode());
        }
        BACnetTimeStamp::SequenceNumber(n) => {
            encode_ctx_unsigned(buf, 1, *n);
        }
        BACnetTimeStamp::DateTime { date, time } => {
            tags::encode_opening_tag(buf, 2);
            encode_app_date(buf, date);
            encode_app_time(buf, time);
            tags::encode_closing_tag(buf, 2);
        }
    }
    tags::encode_closing_tag(buf, tag_number);
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
    // Expect opening tag for tag_number
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if !tag.is_opening_tag(tag_number) {
        return Err(Error::decoding(
            offset,
            format!("expected opening tag {tag_number} for BACnetTimeStamp"),
        ));
    }

    // Peek at the inner choice tag
    let (inner_tag, inner_pos) = tags::decode_tag(data, pos)?;

    let (ts, after_inner) = if inner_tag.is_context(0) {
        // Time choice (context tag 0, 4 bytes)
        let end = inner_pos + inner_tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(inner_pos, "BACnetTimeStamp Time truncated"));
        }
        let t = Time::decode(&data[inner_pos..end])?;
        (BACnetTimeStamp::Time(t), end)
    } else if inner_tag.is_context(1) {
        // SequenceNumber choice (context tag 1)
        let end = inner_pos + inner_tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(
                inner_pos,
                "BACnetTimeStamp SequenceNumber truncated",
            ));
        }
        let n = decode_unsigned(&data[inner_pos..end])?;
        (BACnetTimeStamp::SequenceNumber(n), end)
    } else if inner_tag.is_opening_tag(2) {
        // DateTime choice (opening tag 2, app-tagged Date + Time, closing tag 2)
        // Decode application-tagged Date
        let (date_tag, date_pos) = tags::decode_tag(data, inner_pos)?;
        if date_tag.class != TagClass::Application || date_tag.number != app_tag::DATE {
            return Err(Error::decoding(
                inner_pos,
                "BACnetTimeStamp DateTime expected Date",
            ));
        }
        let date_end = date_pos + date_tag.length as usize;
        if date_end > data.len() {
            return Err(Error::decoding(
                date_pos,
                "BACnetTimeStamp DateTime Date truncated",
            ));
        }
        let date = Date::decode(&data[date_pos..date_end])?;

        // Decode application-tagged Time
        let (time_tag, time_pos) = tags::decode_tag(data, date_end)?;
        if time_tag.class != TagClass::Application || time_tag.number != app_tag::TIME {
            return Err(Error::decoding(
                date_end,
                "BACnetTimeStamp DateTime expected Time",
            ));
        }
        let time_end = time_pos + time_tag.length as usize;
        if time_end > data.len() {
            return Err(Error::decoding(
                time_pos,
                "BACnetTimeStamp DateTime Time truncated",
            ));
        }
        let time = Time::decode(&data[time_pos..time_end])?;

        // Expect closing tag 2
        let (close_tag, close_pos) = tags::decode_tag(data, time_end)?;
        if !close_tag.is_closing_tag(2) {
            return Err(Error::decoding(
                time_end,
                "BACnetTimeStamp DateTime missing closing tag 2",
            ));
        }
        (BACnetTimeStamp::DateTime { date, time }, close_pos)
    } else {
        return Err(Error::decoding(
            pos,
            "BACnetTimeStamp: unexpected inner choice tag",
        ));
    };

    // Expect closing tag for tag_number
    let (close, final_pos) = tags::decode_tag(data, after_inner)?;
    if !close.is_closing_tag(tag_number) {
        return Err(Error::decoding(
            after_inner,
            format!("expected closing tag {tag_number} for BACnetTimeStamp"),
        ));
    }

    Ok((ts, final_pos))
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn encode_to_vec<F: FnOnce(&mut BytesMut)>(f: F) -> Vec<u8> {
        let mut buf = BytesMut::new();
        f(&mut buf);
        buf.to_vec()
    }

    // --- Raw unsigned ---

    #[test]
    fn unsigned_encode_decode_1byte() {
        let mut buf = BytesMut::new();
        encode_unsigned(&mut buf, 42);
        assert_eq!(&buf[..], &[42]);
        assert_eq!(decode_unsigned(&buf).unwrap(), 42);
    }

    #[test]
    fn unsigned_encode_decode_2bytes() {
        let mut buf = BytesMut::new();
        encode_unsigned(&mut buf, 0x1234);
        assert_eq!(&buf[..], &[0x12, 0x34]);
        assert_eq!(decode_unsigned(&buf).unwrap(), 0x1234);
    }

    #[test]
    fn unsigned_encode_decode_3bytes() {
        let mut buf = BytesMut::new();
        encode_unsigned(&mut buf, 0x12_3456);
        assert_eq!(&buf[..], &[0x12, 0x34, 0x56]);
        assert_eq!(decode_unsigned(&buf).unwrap(), 0x12_3456);
    }

    #[test]
    fn unsigned_encode_decode_4bytes() {
        let mut buf = BytesMut::new();
        encode_unsigned(&mut buf, 0xDEAD_BEEF);
        assert_eq!(&buf[..], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(decode_unsigned(&buf).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn unsigned_zero() {
        let mut buf = BytesMut::new();
        encode_unsigned(&mut buf, 0);
        assert_eq!(&buf[..], &[0]);
        assert_eq!(decode_unsigned(&buf).unwrap(), 0);
    }

    #[test]
    fn unsigned_max_u64() {
        let mut buf = BytesMut::new();
        encode_unsigned(&mut buf, u64::MAX);
        assert_eq!(buf.len(), 8);
        assert_eq!(decode_unsigned(&buf).unwrap(), u64::MAX);
    }

    // --- Raw signed ---

    #[test]
    fn signed_encode_decode_positive() {
        let mut buf = BytesMut::new();
        encode_signed(&mut buf, 42);
        assert_eq!(&buf[..], &[42]);
        assert_eq!(decode_signed(&buf).unwrap(), 42);
    }

    #[test]
    fn signed_encode_decode_negative() {
        let mut buf = BytesMut::new();
        encode_signed(&mut buf, -1);
        assert_eq!(&buf[..], &[0xFF]);
        assert_eq!(decode_signed(&buf).unwrap(), -1);
    }

    #[test]
    fn signed_encode_decode_neg128() {
        let mut buf = BytesMut::new();
        encode_signed(&mut buf, -128);
        assert_eq!(&buf[..], &[0x80]);
        assert_eq!(decode_signed(&buf).unwrap(), -128);
    }

    #[test]
    fn signed_encode_decode_neg129() {
        let mut buf = BytesMut::new();
        encode_signed(&mut buf, -129);
        assert_eq!(&buf[..], &[0xFF, 0x7F]);
        assert_eq!(decode_signed(&buf).unwrap(), -129);
    }

    #[test]
    fn signed_encode_decode_min() {
        let mut buf = BytesMut::new();
        encode_signed(&mut buf, i32::MIN);
        assert_eq!(buf.len(), 4);
        assert_eq!(decode_signed(&buf).unwrap(), i32::MIN);
    }

    #[test]
    fn signed_encode_decode_max() {
        let mut buf = BytesMut::new();
        encode_signed(&mut buf, i32::MAX);
        assert_eq!(buf.len(), 4);
        assert_eq!(decode_signed(&buf).unwrap(), i32::MAX);
    }

    // --- Real / Double ---

    #[test]
    fn real_round_trip() {
        let mut buf = BytesMut::new();
        encode_real(&mut buf, 72.5);
        assert_eq!(decode_real(&buf).unwrap(), 72.5);
    }

    #[test]
    fn double_round_trip() {
        let mut buf = BytesMut::new();
        encode_double(&mut buf, core::f64::consts::PI);
        assert_eq!(decode_double(&buf).unwrap(), core::f64::consts::PI);
    }

    // --- Character string ---

    #[test]
    fn character_string_round_trip() {
        let mut buf = BytesMut::new();
        encode_character_string(&mut buf, "hello");
        let decoded = decode_character_string(&buf).unwrap();
        assert_eq!(decoded, "hello");
    }

    #[test]
    fn character_string_empty() {
        let mut buf = BytesMut::new();
        encode_character_string(&mut buf, "");
        let decoded = decode_character_string(&buf).unwrap();
        assert_eq!(decoded, "");
    }

    // --- Bit string ---

    #[test]
    fn bit_string_round_trip() {
        let mut buf = BytesMut::new();
        encode_bit_string(&mut buf, 3, &[0b1010_0000]);
        let (unused, data) = decode_bit_string(&buf).unwrap();
        assert_eq!(unused, 3);
        assert_eq!(data, vec![0b1010_0000]);
    }

    #[test]
    fn bit_string_invalid_unused() {
        assert!(decode_bit_string(&[8]).is_err());
    }

    // --- Application-tagged encode/decode round trips ---

    #[test]
    fn app_null_round_trip() {
        let bytes = encode_to_vec(encode_app_null);
        let (val, offset) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Null);
        assert_eq!(offset, bytes.len());
    }

    #[test]
    fn app_boolean_true_round_trip() {
        let bytes = encode_to_vec(|buf| encode_app_boolean(buf, true));
        let (val, _) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn app_boolean_false_round_trip() {
        let bytes = encode_to_vec(|buf| encode_app_boolean(buf, false));
        let (val, _) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn app_unsigned_round_trip() {
        for &v in &[0u64, 1, 255, 256, 65535, 65536, 0xFFFF_FFFF, u64::MAX] {
            let bytes = encode_to_vec(|buf| encode_app_unsigned(buf, v));
            let (val, end) = decode_application_value(&bytes, 0).unwrap();
            assert_eq!(val, PropertyValue::Unsigned(v), "failed for {v}");
            assert_eq!(end, bytes.len());
        }
    }

    #[test]
    fn app_signed_round_trip() {
        for &v in &[0i32, 1, -1, 127, -128, 128, -129, i32::MIN, i32::MAX] {
            let bytes = encode_to_vec(|buf| encode_app_signed(buf, v));
            let (val, end) = decode_application_value(&bytes, 0).unwrap();
            assert_eq!(val, PropertyValue::Signed(v), "failed for {v}");
            assert_eq!(end, bytes.len());
        }
    }

    #[test]
    fn app_real_round_trip() {
        let bytes = encode_to_vec(|buf| encode_app_real(buf, 72.5));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Real(72.5));
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn app_double_round_trip() {
        let bytes = encode_to_vec(|buf| encode_app_double(buf, core::f64::consts::PI));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Double(core::f64::consts::PI));
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn app_octet_string_round_trip() {
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let bytes = encode_to_vec(|buf| encode_app_octet_string(buf, &data));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::OctetString(data));
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn app_character_string_round_trip() {
        let bytes = encode_to_vec(|buf| encode_app_character_string(buf, "BACnet").unwrap());
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::CharacterString("BACnet".into()));
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn app_enumerated_round_trip() {
        let bytes = encode_to_vec(|buf| encode_app_enumerated(buf, 8));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Enumerated(8));
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn app_date_round_trip() {
        let date = Date {
            year: 124,
            month: 6,
            day: 15,
            day_of_week: 6,
        };
        let bytes = encode_to_vec(|buf| encode_app_date(buf, &date));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Date(date));
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn app_time_round_trip() {
        let time = Time {
            hour: 14,
            minute: 30,
            second: 0,
            hundredths: 0,
        };
        let bytes = encode_to_vec(|buf| encode_app_time(buf, &time));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Time(time));
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn app_object_id_round_trip() {
        let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
        let bytes = encode_to_vec(|buf| encode_app_object_id(buf, &oid));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::ObjectIdentifier(oid));
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn app_bit_string_round_trip() {
        let bytes = encode_to_vec(|buf| encode_app_bit_string(buf, 4, &[0xF0]));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(
            val,
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0xF0],
            }
        );
        assert_eq!(end, bytes.len());
    }

    // --- PropertyValue encode/decode round trip ---

    #[test]
    fn property_value_encode_decode_all_types() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let values = vec![
            PropertyValue::Null,
            PropertyValue::Boolean(true),
            PropertyValue::Boolean(false),
            PropertyValue::Unsigned(12345),
            PropertyValue::Signed(-42),
            PropertyValue::Real(72.5),
            PropertyValue::Double(3.125),
            PropertyValue::OctetString(vec![1, 2, 3]),
            PropertyValue::CharacterString("test".into()),
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0xFF],
            },
            PropertyValue::Enumerated(8),
            PropertyValue::Date(Date {
                year: 124,
                month: 1,
                day: 1,
                day_of_week: 1,
            }),
            PropertyValue::Time(Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            }),
            PropertyValue::ObjectIdentifier(oid),
        ];

        for original in &values {
            let bytes = encode_to_vec(|buf| encode_property_value(buf, original).unwrap());
            let (decoded, end) = decode_application_value(&bytes, 0).unwrap();
            assert_eq!(&decoded, original, "round-trip failed for {original:?}");
            assert_eq!(end, bytes.len(), "offset mismatch for {original:?}");
        }
    }

    // --- Context-tagged encode ---

    #[test]
    fn ctx_unsigned_encoding() {
        let bytes = encode_to_vec(|buf| encode_ctx_unsigned(buf, 1, 42));
        // Context tag 1, length 1, value 42
        assert_eq!(bytes, vec![0x19, 42]);
    }

    #[test]
    fn ctx_boolean_encoding() {
        let bytes = encode_to_vec(|buf| encode_ctx_boolean(buf, 0, true));
        // Context tag 0, length 1, value 1
        assert_eq!(bytes, vec![0x09, 0x01]);
    }

    #[test]
    fn ctx_object_id_encoding() {
        let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
        let bytes = encode_to_vec(|buf| encode_ctx_object_id(buf, 2, &oid));
        // Context tag 2, length 4: (2<<4) | (1<<3) | 4 = 0x2C
        let mut expected = vec![0x2C];
        expected.extend_from_slice(&oid.encode());
        assert_eq!(bytes, expected);
    }

    // --- BACnetTimeStamp encode/decode ---

    #[test]
    fn timestamp_sequence_number_round_trip() {
        let ts = BACnetTimeStamp::SequenceNumber(42);
        let mut buf = BytesMut::new();
        encode_timestamp(&mut buf, 3, &ts);
        let (decoded, end) = decode_timestamp(&buf, 0, 3).unwrap();
        assert_eq!(decoded, ts);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn timestamp_time_round_trip() {
        let ts = BACnetTimeStamp::Time(Time {
            hour: 14,
            minute: 30,
            second: 45,
            hundredths: 50,
        });
        let mut buf = BytesMut::new();
        encode_timestamp(&mut buf, 3, &ts);
        let (decoded, end) = decode_timestamp(&buf, 0, 3).unwrap();
        assert_eq!(decoded, ts);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn timestamp_datetime_round_trip() {
        let ts = BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 2,
                day: 28,
                day_of_week: 6,
            },
            time: Time {
                hour: 10,
                minute: 15,
                second: 0,
                hundredths: 0,
            },
        };
        let mut buf = BytesMut::new();
        encode_timestamp(&mut buf, 5, &ts);
        let (decoded, end) = decode_timestamp(&buf, 0, 5).unwrap();
        assert_eq!(decoded, ts);
        assert_eq!(end, buf.len());
    }

    #[test]
    fn ucs2_decode_ascii() {
        // UCS-2 BE for "AB": 0x00 0x41 0x00 0x42
        let data = [charset::UCS2, 0x00, 0x41, 0x00, 0x42];
        let result = decode_character_string(&data).unwrap();
        assert_eq!(result, "AB");
    }

    #[test]
    fn ucs2_decode_non_ascii() {
        // UCS-2 BE for "é" (U+00E9): 0x00 0xE9
        let data = [charset::UCS2, 0x00, 0xE9];
        let result = decode_character_string(&data).unwrap();
        assert_eq!(result, "é");
    }

    #[test]
    fn unsupported_charset_errors() {
        for &cs in &[charset::JIS_X0201, charset::JIS_C6226, charset::UCS4] {
            let data = [cs, 0x41, 0x42];
            let result = decode_character_string(&data);
            assert!(result.is_err(), "charset {cs} should return an error");
        }
    }
}
