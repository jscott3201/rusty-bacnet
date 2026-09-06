//! BACnet ASN.1 tag encoding and decoding per ASHRAE 135-2020 Clause 20.2.1.
//!
//! BACnet uses a tag-length-value (TLV) encoding with two tag classes:
//! - **Application** tags identify the datatype (Null=0, Boolean=1, ..., ObjectIdentifier=12)
//! - **Context** tags identify a field within a constructed type (tag number = field index)
//!
//! Tags also support opening/closing markers for constructed (nested) values.

use bacnet_types::error::Error;
use bytes::{BufMut, BytesMut};

/// Tag class: application (datatype) or context (field identifier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TagClass {
    /// Application tag -- tag number identifies the BACnet datatype.
    Application = 0,
    /// Context-specific tag -- tag number identifies a field in a constructed type.
    Context = 1,
}

/// Application tag numbers.
pub mod app_tag {
    pub const NULL: u8 = 0;
    pub const BOOLEAN: u8 = 1;
    pub const UNSIGNED: u8 = 2;
    pub const SIGNED: u8 = 3;
    pub const REAL: u8 = 4;
    pub const DOUBLE: u8 = 5;
    pub const OCTET_STRING: u8 = 6;
    pub const CHARACTER_STRING: u8 = 7;
    pub const BIT_STRING: u8 = 8;
    pub const ENUMERATED: u8 = 9;
    pub const DATE: u8 = 10;
    pub const TIME: u8 = 11;
    pub const OBJECT_IDENTIFIER: u8 = 12;
}

/// A decoded BACnet tag header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tag {
    /// Tag number: datatype for application tags, field index for context tags.
    pub number: u8,
    /// Tag class (application or context).
    pub class: TagClass,
    /// Content length in bytes. For application booleans, this holds the raw
    /// L/V/T value (0 = false, nonzero = true) with no content octets.
    pub length: u32,
    /// Whether this is a context-specific opening tag (L/V/T = 6).
    pub is_opening: bool,
    /// Whether this is a context-specific closing tag (L/V/T = 7).
    pub is_closing: bool,
}

impl Tag {
    /// Check if this is an application boolean tag with value true.
    ///
    /// Application-tagged booleans encode the value in the tag's L/V/T
    /// field with no content octets.
    pub fn is_boolean_true(&self) -> bool {
        self.class == TagClass::Application && self.number == app_tag::BOOLEAN && self.length != 0
    }

    /// Check if this is a context tag matching the given number (not opening/closing).
    pub fn is_context(&self, number: u8) -> bool {
        self.class == TagClass::Context
            && self.number == number
            && !self.is_opening
            && !self.is_closing
    }

    /// Check if this is an opening tag matching the given number.
    pub fn is_opening_tag(&self, number: u8) -> bool {
        self.class == TagClass::Context && self.number == number && self.is_opening
    }

    /// Check if this is a closing tag matching the given number.
    pub fn is_closing_tag(&self, number: u8) -> bool {
        self.class == TagClass::Context && self.number == number && self.is_closing
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Encode a tag header into the buffer.
///
/// For application tags, `tag_number` identifies the datatype (0-12).
/// For context tags, `tag_number` is the field identifier (0-254).
pub fn encode_tag(buf: &mut BytesMut, tag_number: u8, class: TagClass, length: u32) {
    let cls_bit = (class as u8) << 3;

    if tag_number <= 14 && length <= 4 {
        buf.put_u8((tag_number << 4) | cls_bit | (length as u8));
        return;
    }

    let tag_nibble = if tag_number <= 14 {
        tag_number << 4
    } else {
        0xF0
    };

    if length <= 4 {
        buf.put_u8(tag_nibble | cls_bit | (length as u8));
        if tag_number > 14 {
            buf.put_u8(tag_number);
        }
        return;
    }

    buf.put_u8(tag_nibble | cls_bit | 5);
    if tag_number > 14 {
        buf.put_u8(tag_number);
    }

    if length <= 253 {
        buf.put_u8(length as u8);
    } else if length <= 65535 {
        buf.put_u8(254);
        buf.put_u16(length as u16);
    } else {
        buf.put_u8(255);
        buf.put_u32(length);
    }
}

/// Encode a context-specific opening tag.
pub fn encode_opening_tag(buf: &mut BytesMut, tag_number: u8) {
    if tag_number <= 14 {
        buf.put_u8((tag_number << 4) | 0x0E);
    } else {
        buf.put_u8(0xFE);
        buf.put_u8(tag_number);
    }
}

/// Encode a context-specific closing tag.
pub fn encode_closing_tag(buf: &mut BytesMut, tag_number: u8) {
    if tag_number <= 14 {
        buf.put_u8((tag_number << 4) | 0x0F);
    } else {
        buf.put_u8(0xFF);
        buf.put_u8(tag_number);
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Maximum tag length sanity limit (1 MB). BACnet APDUs are at most ~64KB
/// segmented; 1MB is generous. Prevents memory exhaustion from malformed packets.
const MAX_TAG_LENGTH: u32 = 1_048_576;

/// Maximum nesting depth for context tags to prevent stack overflow from crafted packets.
pub const MAX_CONTEXT_NESTING_DEPTH: usize = 32;

/// Decode a tag from `data` starting at `offset`.
///
/// Returns the decoded [`Tag`] and the new offset past the tag header.
pub fn decode_tag(data: &[u8], offset: usize) -> Result<(Tag, usize), Error> {
    if offset >= data.len() {
        return Err(Error::decoding(
            offset,
            "tag decode: offset beyond buffer length",
        ));
    }

    let initial = data[offset];
    let mut pos = offset + 1;

    let mut tag_number = (initial >> 4) & 0x0F;
    let class = if (initial >> 3) & 0x01 == 1 {
        TagClass::Context
    } else {
        TagClass::Application
    };
    let lvt = initial & 0x07;
    if class == TagClass::Application && lvt > 5 {
        return Err(Error::decoding(offset, "reserved application tag L/V/T"));
    }

    if tag_number == 0x0F {
        if pos >= data.len() {
            return Err(Error::decoding(pos, "truncated extended tag number"));
        }
        tag_number = data[pos];
        if !(15..=254).contains(&tag_number) {
            return Err(Error::decoding(
                pos,
                format!("invalid extended tag number {tag_number}"),
            ));
        }
        pos += 1;
    }

    if class == TagClass::Context {
        if lvt == 6 {
            return Ok((
                Tag {
                    number: tag_number,
                    class,
                    length: 0,
                    is_opening: true,
                    is_closing: false,
                },
                pos,
            ));
        }
        if lvt == 7 {
            return Ok((
                Tag {
                    number: tag_number,
                    class,
                    length: 0,
                    is_opening: false,
                    is_closing: true,
                },
                pos,
            ));
        }
    }

    let length = if lvt < 5 {
        lvt as u32
    } else {
        if pos >= data.len() {
            return Err(Error::decoding(pos, "truncated extended length"));
        }
        let ext = data[pos];
        pos += 1;

        match ext {
            0..=4 => {
                return Err(Error::decoding(
                    pos - 1,
                    format!("non-canonical extended tag length {ext}"),
                ));
            }
            5..=253 => ext as u32,
            254 => {
                if pos + 2 > data.len() {
                    return Err(Error::decoding(pos, "truncated 2-byte extended length"));
                }
                let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as u32;
                if len < 254 {
                    return Err(Error::decoding(
                        pos,
                        format!("non-canonical 2-byte extended tag length {len}"),
                    ));
                }
                pos += 2;
                len
            }
            255 => {
                if pos + 4 > data.len() {
                    return Err(Error::decoding(pos, "truncated 4-byte extended length"));
                }
                let len =
                    u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
                if len < 65_536 {
                    return Err(Error::decoding(
                        pos,
                        format!("non-canonical 4-byte extended tag length {len}"),
                    ));
                }
                pos += 4;
                len
            }
        }
    };

    if length > MAX_TAG_LENGTH {
        return Err(Error::decoding(
            offset,
            format!("tag length ({length}) exceeds sanity limit ({MAX_TAG_LENGTH})"),
        ));
    }

    Ok((
        Tag {
            number: tag_number,
            class,
            length,
            is_opening: false,
            is_closing: false,
        },
        pos,
    ))
}

/// Extract raw bytes enclosed by a context opening/closing tag pair.
///
/// Reads from `offset` (immediately after the opening tag) through the
/// matching closing tag, handling nested opening/closing tags.
///
/// Returns the enclosed bytes and the offset past the closing tag.
pub fn extract_context_value(
    data: &[u8],
    offset: usize,
    tag_number: u8,
) -> Result<(&[u8], usize), Error> {
    let value_start = offset;
    let mut pos = offset;
    let mut depth: usize = 1;
    let mut open_tags = [0u8; MAX_CONTEXT_NESTING_DEPTH];
    open_tags[0] = tag_number;

    while depth > 0 && pos < data.len() {
        let (tag, new_pos) = decode_tag(data, pos)?;

        if tag.is_opening {
            if depth == MAX_CONTEXT_NESTING_DEPTH {
                return Err(Error::decoding(
                    pos,
                    format!(
                        "context tag nesting depth exceeds maximum ({MAX_CONTEXT_NESTING_DEPTH})"
                    ),
                ));
            }
            open_tags[depth] = tag.number;
            depth += 1;
            pos = new_pos;
        } else if tag.is_closing {
            let expected = open_tags[depth - 1];
            if tag.number != expected {
                return Err(Error::decoding(
                    pos,
                    format!(
                        "closing tag {} does not match opening tag {expected}",
                        tag.number
                    ),
                ));
            }
            depth -= 1;
            if depth == 0 {
                let value_end = pos;
                return Ok((&data[value_start..value_end], new_pos));
            }
            pos = new_pos;
        } else if tag.class == TagClass::Application && tag.number == app_tag::BOOLEAN {
            pos = new_pos;
        } else {
            let content_end = new_pos
                .checked_add(tag.length as usize)
                .ok_or_else(|| Error::decoding(new_pos, "tag length overflow"))?;
            if content_end > data.len() {
                return Err(Error::decoding(
                    new_pos,
                    format!(
                        "tag data overflows buffer: need {} bytes at offset {new_pos}",
                        tag.length
                    ),
                ));
            }
            pos = content_end;
        }
    }

    Err(Error::decoding(
        offset,
        format!("missing closing tag {tag_number}"),
    ))
}

/// Extract raw bytes enclosed by a context opening/closing tag pair WITHOUT
/// parsing the enclosed content as BACnet tags.
///
/// [`extract_context_value`] walks the content as well-formed TLVs, which is
/// the correct way to skip an unknown *conformant* constructed value (Clause
/// 20.2.1.6 rule (d)). This raw scanner is only for compatibility payloads
/// whose contract explicitly permits non-TLV bytes. It is not safe for a
/// `SEQUENCE OF CHOICE`: a primitive value can contain a closing-tag octet.
/// The scan balances raw opening/closing octets for `tag_number`.
///
/// `offset` points just past the opening tag for `tag_number`. Returns the
/// enclosed bytes and the offset past the closing tag octet(s).
///
/// Known limitation: a payload that itself contains the target tag's opening
/// or closing octet pattern (e.g. as unrelated content bytes) unbalances the
/// walk; such payloads cannot round-trip through this path.
pub fn extract_raw_context<'a>(
    data: &'a [u8],
    offset: usize,
    tag_number: u8,
) -> Result<(&'a [u8], usize), Error> {
    let wide = tag_number > 14;
    let open_first = if wide { 0xFE } else { (tag_number << 4) | 0x0E };
    let close_first = if wide { 0xFF } else { (tag_number << 4) | 0x0F };
    let width = if wide { 2 } else { 1 };

    let value_start = offset;
    let mut pos = offset;
    let mut depth: usize = 1;

    while pos < data.len() {
        let b = data[pos];
        if b == open_first && (!wide || data.get(pos + 1) == Some(&tag_number)) {
            depth += 1;
            pos += width;
            continue;
        }
        if b == close_first && (!wide || data.get(pos + 1) == Some(&tag_number)) {
            depth -= 1;
            if depth == 0 {
                return Ok((&data[value_start..pos], pos + width));
            }
            pos += width;
            continue;
        }
        pos += 1;
    }

    Err(Error::decoding(
        value_start,
        format!("missing closing tag {tag_number}"),
    ))
}

/// Try to decode an optional context-tagged primitive value.
///
/// Peeks at the next tag; if it matches the expected context tag number,
/// returns the content slice and advances the offset. Otherwise returns
/// `(None, offset)` unchanged.
pub fn decode_optional_context(
    data: &[u8],
    offset: usize,
    tag_number: u8,
) -> Result<(Option<&[u8]>, usize), Error> {
    if offset >= data.len() {
        return Ok((None, offset));
    }

    let (tag, new_pos) = decode_tag(data, offset)?;
    if tag.is_context(tag_number) {
        let end = new_pos
            .checked_add(tag.length as usize)
            .ok_or_else(|| Error::decoding(new_pos, "tag length overflow"))?;
        if end > data.len() {
            return Err(Error::buffer_too_short(end, data.len()));
        }
        Ok((Some(&data[new_pos..end]), end))
    } else {
        Ok((None, offset))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_to_vec(tag_number: u8, class: TagClass, length: u32) -> Vec<u8> {
        let mut buf = BytesMut::with_capacity(8);
        encode_tag(&mut buf, tag_number, class, length);
        buf.to_vec()
    }

    #[test]
    fn encode_single_byte_application_tag() {
        // Tag 0 (Null), Application, length 0
        assert_eq!(encode_to_vec(0, TagClass::Application, 0), vec![0x00]);
        // Tag 2 (Unsigned), Application, length 1
        assert_eq!(encode_to_vec(2, TagClass::Application, 1), vec![0x21]);
        // Tag 4 (Real), Application, length 4
        assert_eq!(encode_to_vec(4, TagClass::Application, 4), vec![0x44]);
        // Tag 12 (ObjectIdentifier), Application, length 4
        assert_eq!(encode_to_vec(12, TagClass::Application, 4), vec![0xC4]);
    }

    #[test]
    fn encode_single_byte_context_tag() {
        // Context tag 0, length 1
        assert_eq!(encode_to_vec(0, TagClass::Context, 1), vec![0x09]);
        // Context tag 1, length 4
        assert_eq!(encode_to_vec(1, TagClass::Context, 4), vec![0x1C]);
        // Context tag 2, length 0: (2<<4) | (1<<3) | 0 = 0x28
        assert_eq!(encode_to_vec(2, TagClass::Context, 0), vec![0x28]);
    }

    #[test]
    fn encode_extended_tag_number() {
        // Tag 15 (extended), Application, length 1
        let encoded = encode_to_vec(15, TagClass::Application, 1);
        assert_eq!(encoded, vec![0xF1, 15]);

        // Tag 20 (extended), Context, length 2
        let encoded = encode_to_vec(20, TagClass::Context, 2);
        assert_eq!(encoded, vec![0xFA, 20]);
    }

    #[test]
    fn encode_extended_length() {
        // Tag 2, Application, length 100
        let encoded = encode_to_vec(2, TagClass::Application, 100);
        assert_eq!(encoded, vec![0x25, 100]);

        // Tag 2, Application, length 1000 (2-byte extended)
        let encoded = encode_to_vec(2, TagClass::Application, 1000);
        assert_eq!(encoded, vec![0x25, 254, 0x03, 0xE8]);

        // Tag 2, Application, length 100000 (4-byte extended)
        let encoded = encode_to_vec(2, TagClass::Application, 100000);
        assert_eq!(encoded, vec![0x25, 255, 0x00, 0x01, 0x86, 0xA0]);
    }

    #[test]
    fn encode_extended_tag_and_length() {
        // Tag 20, Context, length 100
        let encoded = encode_to_vec(20, TagClass::Context, 100);
        assert_eq!(encoded, vec![0xFD, 20, 100]);
    }

    #[test]
    fn encode_opening_closing_tags() {
        let mut buf = BytesMut::new();

        // Opening tag 0: 0x0E
        encode_opening_tag(&mut buf, 0);
        assert_eq!(&buf[..], &[0x0E]);

        buf.clear();
        // Closing tag 0: 0x0F
        encode_closing_tag(&mut buf, 0);
        assert_eq!(&buf[..], &[0x0F]);

        buf.clear();
        // Opening tag 3: 0x3E
        encode_opening_tag(&mut buf, 3);
        assert_eq!(&buf[..], &[0x3E]);

        buf.clear();
        // Extended opening tag 20: 0xFE, 20
        encode_opening_tag(&mut buf, 20);
        assert_eq!(&buf[..], &[0xFE, 20]);

        buf.clear();
        // Extended closing tag 20: 0xFF, 20
        encode_closing_tag(&mut buf, 20);
        assert_eq!(&buf[..], &[0xFF, 20]);
    }

    #[test]
    fn decode_single_byte_tag() {
        // Application tag 2, length 1: 0x21
        let (tag, pos) = decode_tag(&[0x21], 0).unwrap();
        assert_eq!(tag.number, 2);
        assert_eq!(tag.class, TagClass::Application);
        assert_eq!(tag.length, 1);
        assert!(!tag.is_opening);
        assert!(!tag.is_closing);
        assert_eq!(pos, 1);
    }

    #[test]
    fn decode_context_tag() {
        // Context tag 1, length 4: 0x1C
        let (tag, pos) = decode_tag(&[0x1C], 0).unwrap();
        assert_eq!(tag.number, 1);
        assert_eq!(tag.class, TagClass::Context);
        assert_eq!(tag.length, 4);
        assert_eq!(pos, 1);
    }

    #[test]
    fn decode_opening_closing_tags() {
        let (tag, _) = decode_tag(&[0x0E], 0).unwrap();
        assert!(tag.is_opening);
        assert_eq!(tag.number, 0);

        let (tag, _) = decode_tag(&[0x0F], 0).unwrap();
        assert!(tag.is_closing);
        assert_eq!(tag.number, 0);

        let (tag, _) = decode_tag(&[0x3E], 0).unwrap();
        assert!(tag.is_opening);
        assert_eq!(tag.number, 3);
    }

    #[test]
    fn decode_extended_tag_number() {
        // Extended tag 20, Application, length 1: 0xF1, 20
        let (tag, pos) = decode_tag(&[0xF1, 20], 0).unwrap();
        assert_eq!(tag.number, 20);
        assert_eq!(tag.class, TagClass::Application);
        assert_eq!(tag.length, 1);
        assert_eq!(pos, 2);
    }

    #[test]
    fn decode_extended_length() {
        // Tag 2, Application, length 100: 0x25, 100
        let (tag, pos) = decode_tag(&[0x25, 100], 0).unwrap();
        assert_eq!(tag.number, 2);
        assert_eq!(tag.length, 100);
        assert_eq!(pos, 2);

        // Tag 2, Application, length 1000: 0x25, 254, 0x03, 0xE8
        let (tag, pos) = decode_tag(&[0x25, 254, 0x03, 0xE8], 0).unwrap();
        assert_eq!(tag.number, 2);
        assert_eq!(tag.length, 1000);
        assert_eq!(pos, 4);

        // Tag 2, Application, length 100000: 0x25, 255, 0x00, 0x01, 0x86, 0xA0
        let (tag, pos) = decode_tag(&[0x25, 255, 0x00, 0x01, 0x86, 0xA0], 0).unwrap();
        assert_eq!(tag.number, 2);
        assert_eq!(tag.length, 100000);
        assert_eq!(pos, 6);
    }

    #[test]
    fn decode_tag_round_trip() {
        // Test that encode -> decode round-trips for various combinations
        let cases = [
            (0u8, TagClass::Application, 0u32),
            (1, TagClass::Application, 1),
            (4, TagClass::Application, 4),
            (12, TagClass::Application, 4),
            (0, TagClass::Context, 1),
            (3, TagClass::Context, 4),
            (15, TagClass::Application, 1),
            (20, TagClass::Context, 2),
            (2, TagClass::Application, 100),
            (2, TagClass::Application, 1000),
            (20, TagClass::Context, 100),
        ];

        for (tag_num, class, length) in cases {
            let encoded = encode_to_vec(tag_num, class, length);
            let (decoded, _) = decode_tag(&encoded, 0).unwrap();
            assert_eq!(
                decoded.number, tag_num,
                "tag number mismatch for ({tag_num}, {class:?}, {length})"
            );
            assert_eq!(
                decoded.class, class,
                "class mismatch for ({tag_num}, {class:?}, {length})"
            );
            assert_eq!(
                decoded.length, length,
                "length mismatch for ({tag_num}, {class:?}, {length})"
            );
        }
    }

    #[test]
    fn decode_tag_empty_buffer() {
        assert!(decode_tag(&[], 0).is_err());
    }

    #[test]
    fn decode_tag_truncated_extended() {
        // Extended tag number but no second byte
        assert!(decode_tag(&[0xF1], 0).is_err());
    }

    #[test]
    fn decode_tag_excessive_length() {
        // Craft a tag with length > 1MB
        let data = [0x25, 255, 0x10, 0x00, 0x00, 0x00]; // length = 0x10000000
        assert!(decode_tag(&data, 0).is_err());
    }

    #[test]
    fn boolean_tag_detection() {
        // Application boolean true: tag 1, LVT = 1
        let (tag, _) = decode_tag(&[0x11], 0).unwrap();
        assert!(tag.is_boolean_true());

        // Application boolean false: tag 1, LVT = 0
        let (tag, _) = decode_tag(&[0x10], 0).unwrap();
        assert!(!tag.is_boolean_true());

        // Not a boolean (tag 2): should not be detected as boolean
        let (tag, _) = decode_tag(&[0x21], 0).unwrap();
        assert!(!tag.is_boolean_true());
    }

    #[test]
    fn extract_context_value_simple() {
        // Opening tag 0, some data (tag 2, len 1, value 42), closing tag 0
        let data = [0x0E, 0x21, 42, 0x0F];
        let (value, pos) = extract_context_value(&data, 1, 0).unwrap();
        assert_eq!(value, &[0x21, 42]);
        assert_eq!(pos, 4);
    }

    // --- extract_raw_context ---

    #[test]
    fn extract_raw_context_non_tlv_payload() {
        // Payload bytes that are NOT well-formed TLVs must pass through
        // verbatim (this is the whole point of the raw scanner).
        let data = [0x9E, 0xDE, 0xAD, 0xBE, 0xEF, 0x9F];
        let (value, pos) = extract_raw_context(&data, 1, 9).unwrap();
        assert_eq!(value, &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(pos, 6);
    }

    #[test]
    fn extract_raw_context_nested_same_tag() {
        // opening 2, opening 2, payload, closing 2, more payload, closing 2
        let data = [0x2E, 0x2E, 0x01, 0x2F, 0x02, 0x2F];
        let (value, pos) = extract_raw_context(&data, 1, 2).unwrap();
        assert_eq!(value, &[0x2E, 0x01, 0x2F, 0x02]);
        assert_eq!(pos, 6);
    }

    #[test]
    fn extract_raw_context_extended_tag() {
        // Extended tag 200: 0xFE 0xC8 ... 0xFF 0xC8.
        let data = [0xFE, 200, 0x01, 0x02, 0xFF, 200];
        let (value, pos) = extract_raw_context(&data, 2, 200).unwrap();
        assert_eq!(value, &[0x01, 0x02]);
        assert_eq!(pos, 6);
    }

    #[test]
    fn extract_raw_context_missing_closing() {
        let data = [0x9E, 0xDE, 0xAD];
        assert!(extract_raw_context(&data, 1, 9).is_err());
    }

    #[test]
    fn extract_raw_context_truncated_extended_closing() {
        // 0xFF at the very end with no tag-number octet — not a closing.
        let data = [0xFE, 200, 0x01, 0x02, 0xFF];
        assert!(extract_raw_context(&data, 2, 200).is_err());
    }

    #[test]
    fn extract_context_value_nested() {
        // Opening tag 0, opening tag 1, data, closing tag 1, closing tag 0
        let data = [0x0E, 0x1E, 0x21, 42, 0x1F, 0x0F];
        let (value, pos) = extract_context_value(&data, 1, 0).unwrap();
        assert_eq!(value, &[0x1E, 0x21, 42, 0x1F]);
        assert_eq!(pos, 6);
    }

    #[test]
    fn extract_context_value_missing_close() {
        let data = [0x0E, 0x21, 42]; // No closing tag
        assert!(extract_context_value(&data, 1, 0).is_err());
    }

    #[test]
    fn tag_is_context_helper() {
        let (tag, _) = decode_tag(&[0x09], 0).unwrap(); // context 0, len 1
        assert!(tag.is_context(0));
        assert!(!tag.is_context(1));
    }

    #[test]
    fn decode_optional_context_present() {
        // Context tag 0, length 1, value byte 42
        let data = [0x09, 42];
        let (value, pos) = decode_optional_context(&data, 0, 0).unwrap();
        assert_eq!(value, Some(&[42u8][..]));
        assert_eq!(pos, 2);
    }

    #[test]
    fn decode_optional_context_absent() {
        // Context tag 1, but we're looking for tag 0
        let data = [0x19, 42];
        let (value, pos) = decode_optional_context(&data, 0, 0).unwrap();
        assert!(value.is_none());
        assert_eq!(pos, 0); // offset unchanged
    }

    #[test]
    fn decode_optional_context_empty_buffer() {
        let (value, pos) = decode_optional_context(&[], 0, 0).unwrap();
        assert!(value.is_none());
        assert_eq!(pos, 0);
    }

    // --- Edge case tests ---

    #[test]
    fn extract_context_value_mismatched_closing_tag() {
        // Opening tag 0, data, closing tag 1 (mismatch!)
        let data = [0x0E, 0x21, 42, 0x1F]; // open 0, data, close 1
        assert!(extract_context_value(&data, 1, 0).is_err());
    }

    #[test]
    fn extract_context_value_mismatched_nested_closing_tag() {
        // Opening tag 0, opening tag 1, closing tag 2, closing tag 0.
        let data = [0x0E, 0x1E, 0x2F, 0x0F];
        assert!(extract_context_value(&data, 1, 0).is_err());
    }

    #[test]
    fn extract_context_value_deeply_nested() {
        // Verify correct extraction with multiple nesting levels
        // open 0, open 1, open 2, data, close 2, close 1, close 0
        let data = [
            0x0E, // opening 0
            0x1E, // opening 1
            0x2E, // opening 2
            0x21, 42,   // app tag 2 len 1, value 42
            0x2F, // closing 2
            0x1F, // closing 1
            0x0F, // closing 0
        ];
        let (value, pos) = extract_context_value(&data, 1, 0).unwrap();
        assert_eq!(value, &data[1..7]); // everything between open 0 and close 0
        assert_eq!(pos, 8);
    }

    #[test]
    fn extract_context_value_nesting_depth_exceeded() {
        // Build a deeply nested structure that exceeds MAX_CONTEXT_NESTING_DEPTH
        let mut data = Vec::new();
        // We start at depth 1 (already inside opening tag), so we need
        // MAX_CONTEXT_NESTING_DEPTH more opening tags to exceed the limit
        for i in 0..MAX_CONTEXT_NESTING_DEPTH {
            let tag = (i % 15) as u8; // cycle through tag numbers 0-14
            data.push((tag << 4) | 0x0E); // opening tag
        }
        // The function starts at depth 1, so this should trigger the limit
        let result = extract_context_value(&data, 0, 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_tag_at_nonzero_offset() {
        // Verify decode works correctly when starting at a non-zero offset
        let data = [0xFF, 0xFF, 0x21, 0x00]; // garbage, then tag 2 len 1
        let (tag, pos) = decode_tag(&data, 2).unwrap();
        assert_eq!(tag.number, 2);
        assert_eq!(tag.class, TagClass::Application);
        assert_eq!(tag.length, 1);
        assert_eq!(pos, 3);
    }

    #[test]
    fn decode_tag_offset_beyond_buffer() {
        let data = [0x21];
        assert!(decode_tag(&data, 5).is_err());
    }

    #[test]
    fn decode_tag_truncated_2byte_extended_length() {
        // Extended length indicator 254 but only 1 byte follows
        let data = [0x25, 254, 0x03]; // need 2 bytes after 254
        assert!(decode_tag(&data, 0).is_err());
    }

    #[test]
    fn decode_tag_truncated_4byte_extended_length() {
        // Extended length indicator 255 but only 2 bytes follow
        let data = [0x25, 255, 0x00, 0x01];
        assert!(decode_tag(&data, 0).is_err());
    }

    #[test]
    fn extract_context_value_with_boolean_inside() {
        // Boolean tags have no content octets, which is a special case
        // Opening tag 0, boolean true (app tag 1, lvt=1), closing tag 0
        let data = [0x0E, 0x11, 0x0F]; // open 0, bool true, close 0
        let (value, pos) = extract_context_value(&data, 1, 0).unwrap();
        assert_eq!(value, &[0x11]); // just the boolean tag
        assert_eq!(pos, 3);
    }

    #[test]
    fn extract_context_value_tag_data_overflows_buffer() {
        // Opening tag 0, a data tag claiming 100 bytes of content but only 2 available
        let data = [0x0E, 0x25, 100, 0x01, 0x02, 0x0F];
        let result = extract_context_value(&data, 1, 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_optional_context_content_overflows() {
        // Context tag 0, length 4, but only 2 content bytes available
        let data = [0x0C, 0x01, 0x02]; // ctx 0, len 4, only 2 bytes
        assert!(decode_optional_context(&data, 0, 0).is_err());
    }

    #[test]
    fn opening_closing_tag_extended_round_trip() {
        let mut buf = BytesMut::new();
        // Extended tag numbers (>14) for opening/closing
        for tag_num in [15u8, 20, 100, 254] {
            buf.clear();
            encode_opening_tag(&mut buf, tag_num);
            let (tag, pos1) = decode_tag(&buf, 0).unwrap();
            assert!(tag.is_opening);
            assert_eq!(tag.number, tag_num);
            assert_eq!(pos1, 2); // extended = 2 bytes

            buf.clear();
            encode_closing_tag(&mut buf, tag_num);
            let (tag, pos2) = decode_tag(&buf, 0).unwrap();
            assert!(tag.is_closing);
            assert_eq!(tag.number, tag_num);
            assert_eq!(pos2, 2);
        }
    }
}
