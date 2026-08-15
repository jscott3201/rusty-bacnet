//! WriteProperty service per ASHRAE 135-2020 Clause 15.9.

use bacnet_encoding::primitives;
use bacnet_encoding::tags::{self, TagClass};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::{extract_property_value, PropertyValueBoundary};

fn decode_context_u32(
    data: &[u8],
    offset: usize,
    tag_number: u8,
    context: &str,
) -> Result<(u32, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if !tag.is_context(tag_number) {
        return Err(Error::decoding(
            offset,
            format!("{context} expected context tag {tag_number}"),
        ));
    }
    let end = pos + tag.length as usize;
    if end > data.len() {
        return Err(Error::decoding(pos, format!("{context} truncated")));
    }
    let raw = primitives::decode_unsigned(&data[pos..end])?;
    let value = u32::try_from(raw)
        .map_err(|_| Error::decoding(pos, format!("{context} {raw} exceeds u32")))?;
    Ok((value, end))
}

// ---------------------------------------------------------------------------
// WritePropertyRequest
// ---------------------------------------------------------------------------

/// WriteProperty-Request service parameters.
///
/// ```text
/// WriteProperty-Request ::= SEQUENCE {
///     objectIdentifier    [0] BACnetObjectIdentifier,
///     propertyIdentifier  [1] BACnetPropertyIdentifier,
///     propertyArrayIndex  [2] Unsigned OPTIONAL,
///     propertyValue       [3] ABSTRACT-SYNTAX.&TYPE,
///     priority            [4] Unsigned (1..16) OPTIONAL
/// }
/// ```
///
/// WriteProperty uses SimpleACK (no ACK struct needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePropertyRequest {
    pub object_identifier: ObjectIdentifier,
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    pub property_value: Vec<u8>,
    pub priority: Option<u8>,
}

impl WritePropertyRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_ctx_object_id(buf, 0, &self.object_identifier);
        primitives::encode_ctx_unsigned(buf, 1, self.property_identifier.to_raw() as u64);
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 2, idx as u64);
        }
        tags::encode_opening_tag(buf, 3);
        buf.extend_from_slice(&self.property_value);
        tags::encode_closing_tag(buf, 3);
        if let Some(prio) = self.priority {
            primitives::encode_ctx_unsigned(buf, 4, prio as u64);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] object-identifier
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(0) {
            return Err(Error::decoding(
                offset,
                "WriteProperty expected context tag 0 for object-id",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "WriteProperty truncated at object-id"));
        }
        let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
        offset = end;

        // [1] property-identifier
        let (prop_raw, end) = decode_context_u32(data, offset, 1, "WriteProperty property-id")?;
        let property_identifier = PropertyIdentifier::from_raw(prop_raw);
        offset = end;

        // [2] propertyArrayIndex (optional)
        let mut property_array_index = None;
        let (tag, _) = tags::decode_tag(data, offset)?;
        if tag.class == TagClass::Context && tag.number == 2 && !tag.is_opening && !tag.is_closing {
            let (index, end) = decode_context_u32(data, offset, 2, "WriteProperty array-index")?;
            property_array_index = Some(index);
            offset = end;
        }

        // [3] propertyValue (opening/closing tag 3)
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if !tag.is_opening_tag(3) {
            return Err(Error::decoding(
                offset,
                "WriteProperty expected opening tag 3",
            ));
        }
        let (value_bytes, new_offset) = extract_property_value(
            data,
            tag_end,
            3,
            property_identifier,
            &[
                PropertyValueBoundary::End,
                PropertyValueBoundary::ContextToEnd(4),
            ],
        )?;
        let property_value = value_bytes.to_vec();
        offset = new_offset;

        // [4] priority (optional)
        let mut priority = None;
        if offset < data.len() {
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_context(4) {
                return Err(Error::decoding(
                    offset,
                    "WriteProperty expected context tag 4 for priority",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "WriteProperty truncated at priority"));
            }
            let prio = primitives::decode_unsigned(&data[pos..end])?;
            if !(1..=16).contains(&prio) {
                return Err(Error::decoding(
                    pos,
                    format!("WriteProperty priority {prio} out of range 1-16"),
                ));
            }
            if end != data.len() {
                return Err(Error::decoding(end, "WriteProperty has trailing data"));
            }
            priority =
                Some(u8::try_from(prio).map_err(|_| {
                    Error::decoding(pos, "WriteProperty priority conversion failed")
                })?);
        }

        Ok(Self {
            object_identifier,
            property_identifier,
            property_array_index,
            property_value,
            priority,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn object_id() -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
    }

    fn encode_context_value(buf: &mut BytesMut, tag_number: u8, value: u64, leading_zero: bool) {
        if leading_zero {
            tags::encode_tag(buf, tag_number, TagClass::Context, 5);
            buf.extend_from_slice(&value.to_be_bytes()[3..]);
        } else {
            primitives::encode_ctx_unsigned(buf, tag_number, value);
        }
    }

    fn encode_fields(
        object_tag: u8,
        property_tag: u8,
        property: u64,
        index: Option<(u8, u64)>,
        priority: Option<(u8, u64)>,
        leading_zero: bool,
    ) -> BytesMut {
        let mut buf = BytesMut::new();
        primitives::encode_ctx_object_id(&mut buf, object_tag, &object_id());
        encode_context_value(&mut buf, property_tag, property, leading_zero);
        if let Some((tag_number, value)) = index {
            encode_context_value(&mut buf, tag_number, value, leading_zero);
        }
        tags::encode_opening_tag(&mut buf, 3);
        primitives::encode_app_null(&mut buf);
        tags::encode_closing_tag(&mut buf, 3);
        if let Some((tag_number, value)) = priority {
            primitives::encode_ctx_unsigned(&mut buf, tag_number, value);
        }
        buf
    }

    #[test]
    fn request_round_trip() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WritePropertyRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_with_all_fields() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: Some(5),
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: Some(8),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WritePropertyRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn write_property_values_must_fit_u32() {
        let maximum = u64::from(u32::MAX);
        let encoded = encode_fields(0, 1, maximum, Some((2, maximum)), None, true);
        let request = WritePropertyRequest::decode(&encoded).unwrap();
        assert_eq!(request.property_identifier.to_raw(), u32::MAX);
        assert_eq!(request.property_array_index, Some(u32::MAX));

        for value in [maximum + 1, u64::MAX] {
            let property = encode_fields(0, 1, value, None, None, false);
            assert!(WritePropertyRequest::decode(&property).is_err());

            let index = encode_fields(0, 1, 1, Some((2, value)), None, false);
            assert!(WritePropertyRequest::decode(&index).is_err());
        }
    }

    #[test]
    fn write_property_requires_mandatory_context_tags() {
        let wrong_object = encode_fields(1, 1, 1, None, None, false);
        assert!(WritePropertyRequest::decode(&wrong_object).is_err());

        let wrong_property = encode_fields(0, 0, 1, None, None, false);
        assert!(WritePropertyRequest::decode(&wrong_property).is_err());

        let mut application_property = BytesMut::new();
        primitives::encode_ctx_object_id(&mut application_property, 0, &object_id());
        primitives::encode_app_unsigned(&mut application_property, 1);
        assert!(WritePropertyRequest::decode(&application_property).is_err());
    }

    #[test]
    fn write_property_rejects_malformed_priority_and_trailing_fields() {
        let wrong_priority = encode_fields(0, 1, 1, None, Some((5, 1)), false);
        assert!(WritePropertyRequest::decode(&wrong_priority).is_err());

        let mut malformed_priority = encode_fields(0, 1, 1, None, None, false);
        tags::encode_opening_tag(&mut malformed_priority, 4);
        assert!(WritePropertyRequest::decode(&malformed_priority).is_err());

        let mut trailing = encode_fields(0, 1, 1, None, Some((4, 8)), false);
        primitives::encode_ctx_unsigned(&mut trailing, 5, 1);
        assert!(WritePropertyRequest::decode(&trailing).is_err());
    }

    #[test]
    fn priority_validation() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x91, 0x01], // enumerated 1 (active)
            priority: Some(16),               // max valid
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = WritePropertyRequest::decode(&buf).unwrap();
        assert_eq!(decoded.priority, Some(16));

        for content in [
            &[0x00][..],
            &[0x11][..],
            &[0x01, 0x01][..],
            &[0x01, 0x10][..],
        ] {
            let mut buf = BytesMut::new();
            WritePropertyRequest {
                priority: None,
                ..req.clone()
            }
            .encode(&mut buf);
            tags::encode_tag(
                &mut buf,
                4,
                TagClass::Context,
                u32::try_from(content.len()).unwrap(),
            );
            buf.extend_from_slice(content);
            assert!(
                WritePropertyRequest::decode(&buf).is_err(),
                "priority content {content:02X?} must be rejected"
            );
        }

        let mut buf = BytesMut::new();
        WritePropertyRequest {
            priority: None,
            ..req
        }
        .encode(&mut buf);
        tags::encode_tag(&mut buf, 4, TagClass::Context, 2);
        buf.extend_from_slice(&[0x00, 0x01]);
        assert_eq!(
            WritePropertyRequest::decode(&buf).unwrap().priority,
            Some(1)
        );
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_write_property_empty_input() {
        assert!(WritePropertyRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_write_property_truncated_1_byte() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(WritePropertyRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_write_property_truncated_2_bytes() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(WritePropertyRequest::decode(&buf[..2]).is_err());
    }

    #[test]
    fn test_decode_write_property_truncated_3_bytes() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(WritePropertyRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_write_property_truncated_half() {
        let req = WritePropertyRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
            priority: Some(8),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(WritePropertyRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_write_property_invalid_tag() {
        assert!(WritePropertyRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_write_property_oversized_length() {
        // Tag with oversized length field
        assert!(WritePropertyRequest::decode(&[0x05, 0xFF]).is_err());
    }
}
