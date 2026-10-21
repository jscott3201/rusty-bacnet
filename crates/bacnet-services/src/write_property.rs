//! WriteProperty service per ASHRAE 135-2020 Clause 15.9.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{EncodeError, Error};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use bacnet_encoding::constructed::tagged::{
    decode_ctx_object_id, decode_ctx_unsigned, decode_optional_ctx, expect_end, expect_opening,
};
use bacnet_encoding::constructed::{extract_property_value, PropertyValueBoundary};

// ---------------------------------------------------------------------------
// WritePropertyRequest
// ---------------------------------------------------------------------------

/// WriteProperty-Request service parameters (Clause 15.9; production in Clause 21.2).
///
/// Five context-tagged members in order: object identifier `[0]`, property identifier `[1]`,
/// an optional Unsigned array index `[2]`, the value inside an opening/closing `[3]` pair (typed
/// by the property), and an optional priority `[4]`, an Unsigned limited to 1-16.
///
/// WriteProperty uses SimpleACK (no ACK struct needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePropertyRequest {
    /// Object to write to.
    pub object_identifier: ObjectIdentifier,
    /// Property to write.
    pub property_identifier: PropertyIdentifier,
    /// Array index of the element to write; `None` writes the whole property.
    pub property_array_index: Option<u32>,
    /// Application-tagged encoding of the value to write, opaque to this crate.
    pub property_value: Vec<u8>,
    /// Priority (1-16) for commandable properties. `None` omits it, and the responder then
    /// uses 16; a non-commandable property ignores it (Clause 15.9.1).
    pub priority: Option<u8>,
}

/// Validate an outbound property-write priority without inspecting commandability.
///
/// Omission is allowed; every supplied priority must be in 1..=16, including
/// NULL writes. Returns a local encoding error for invalid typed input.
pub fn validate_priority(priority: Option<u8>) -> Result<(), Error> {
    if let Some(priority) = priority {
        if !(1..=16).contains(&priority) {
            return Err(EncodeError::new("WriteProperty priority must be 1-16, got {}")
                .with_value(priority.into())
                .into());
        }
    }
    Ok(())
}

impl WritePropertyRequest {
    /// Encode transactionally: invalid priority leaves `buf` unchanged.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        validate_priority(self.priority)?;
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
        Ok(())
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] object-identifier
        let (object_identifier, offset) =
            decode_ctx_object_id(data, 0, 0, "WriteProperty object-id")?;

        // [1] property-identifier
        let (prop_raw, offset) =
            decode_ctx_unsigned::<u32>(data, offset, 1, "WriteProperty property-id")?;
        let property_identifier = PropertyIdentifier::from_raw(prop_raw);

        // [2] propertyArrayIndex (optional)
        let (property_array_index, offset) = decode_optional_ctx(
            data,
            offset,
            2,
            "WriteProperty array-index",
            decode_ctx_unsigned::<u32>,
        )?;

        // [3] propertyValue (opening/closing tag 3)
        let content = expect_opening(data, offset, 3, "WriteProperty property-value")?;
        let (value_bytes, offset) = extract_property_value(
            data,
            content,
            3,
            property_identifier,
            &[
                PropertyValueBoundary::End,
                PropertyValueBoundary::ContextToEnd(4),
            ],
        )?;
        let property_value = value_bytes.to_vec();

        // [4] priority (optional), and nothing after it. Read at full width
        // so a value past 16 draws the range error, not a width error.
        let (prio, end) = decode_optional_ctx(
            data,
            offset,
            4,
            "WriteProperty priority",
            decode_ctx_unsigned::<u64>,
        )?;
        expect_end(data, end, end, "WriteProperty")?;
        let mut priority = None;
        if let Some(prio) = prio {
            if !(1..=16).contains(&prio) {
                return Err(Error::Protocol {
                    class: ErrorClass::SERVICES,
                    code: ErrorCode::PARAMETER_OUT_OF_RANGE,
                });
            }
            priority = Some(prio as u8);
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
    use bacnet_encoding::tags::TagClass;
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
    fn outbound_priority_validation_is_transactional() {
        for priority in [0, 17, 255] {
            let request = WritePropertyRequest {
                object_identifier: object_id(),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: Some(0),
                property_value: vec![0],
                priority: Some(priority),
            };
            let mut output = BytesMut::from(&b"prefix"[..]);
            assert!(matches!(
                request.encode(&mut output),
                Err(Error::Encode(_))
            ));
            assert_eq!(
                &output[..],
                b"prefix",
                "invalid priority {priority} changed output"
            );
        }
    }

    #[test]
    fn outbound_priority_none_and_all_valid_values_preserve_bytes() {
        for priority in std::iter::once(None).chain((1..=16).map(Some)) {
            let request = WritePropertyRequest {
                object_identifier: object_id(),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: Some(0),
                property_value: vec![0],
                priority,
            };
            let mut output = BytesMut::from(&b"prefix"[..]);
            request.encode(&mut output).unwrap();
            let mut expected = b"prefix".to_vec();
            expected
                .extend_from_slice(&[0x0c, 0x00, 0x40, 0x00, 1, 0x19, 85, 0x29, 0, 0x3e, 0, 0x3f]);
            if let Some(priority) = priority {
                expected.extend_from_slice(&[0x49, priority]);
            }
            assert_eq!(output.as_ref(), expected);
            assert_eq!(WritePropertyRequest::decode(&output[6..]).unwrap(), request);
        }
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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
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
            .encode(&mut buf)
            .unwrap();
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
        .encode(&mut buf)
        .unwrap();
        tags::encode_tag(&mut buf, 4, TagClass::Context, 2);
        buf.extend_from_slice(&[0x00, 0x01]);
        assert_eq!(
            WritePropertyRequest::decode(&buf).unwrap().priority,
            Some(1)
        );
    }

    #[test]
    fn write_property_accepts_all_priorities_with_leading_zeros() {
        for priority in 1u8..=16 {
            for length in 1..=8 {
                let mut buf = encode_fields(0, 1, 85, None, None, false);
                tags::encode_tag(
                    &mut buf,
                    4,
                    TagClass::Context,
                    u32::try_from(length).unwrap(),
                );
                buf.extend_from_slice(&u64::from(priority).to_be_bytes()[8 - length..]);
                let request = WritePropertyRequest::decode(&buf).unwrap();
                assert_eq!(request.object_identifier, object_id());
                assert_eq!(
                    request.property_identifier,
                    PropertyIdentifier::PRESENT_VALUE
                );
                assert_eq!(request.property_array_index, None);
                assert_eq!(request.property_value, [0x00]);
                assert_eq!(request.priority, Some(priority));
            }
        }
    }

    #[test]
    fn write_property_priority_range_errors_are_typed_for_all_unsigned_widths() {
        let mut values = vec![0, 17, 255, 256, 65_536, 4_294_967_296, u64::MAX];
        for low_byte in 1..=16 {
            for high_bits in [0x100, 0x1_0000, 0x1_0000_0000, 0x8000_0000_0000_0000] {
                values.push(high_bits | low_byte);
            }
        }
        for value in values {
            let buf = encode_fields(0, 1, 85, None, Some((4, value)), false);
            let error = WritePropertyRequest::decode(&buf).unwrap_err();
            assert!(
                matches!(error, Error::Protocol { class, code }
                    if class == ErrorClass::SERVICES
                        && code == ErrorCode::PARAMETER_OUT_OF_RANGE),
                "priority {value} must fail semantic range validation, got {error:?}"
            );
        }
        for length in [0, 9] {
            let mut buf = encode_fields(0, 1, 85, None, None, false);
            tags::encode_tag(&mut buf, 4, TagClass::Context, length);
            buf.extend_from_slice(&vec![0; length as usize]);
            assert!(matches!(
                WritePropertyRequest::decode(&buf),
                Err(Error::Decoding(_))
            ));
        }
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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
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
        req.encode(&mut buf).unwrap();
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
