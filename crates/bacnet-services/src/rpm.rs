//! ReadPropertyMultiple service per ASHRAE 135-2020 Clause 15.7.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::{
    extract_property_value, PropertyReference, PropertyValueBoundary, MAX_DECODED_ITEMS,
};

// ---------------------------------------------------------------------------
// ReadPropertyMultipleRequest
// ---------------------------------------------------------------------------

/// A single object + list of property references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAccessSpecification {
    pub object_identifier: ObjectIdentifier,
    pub list_of_property_references: Vec<PropertyReference>,
}

/// ReadPropertyMultiple-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyMultipleRequest {
    pub list_of_read_access_specs: Vec<ReadAccessSpecification>,
}

impl ReadPropertyMultipleRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        for spec in &self.list_of_read_access_specs {
            // [0] object-identifier
            primitives::encode_ctx_object_id(buf, 0, &spec.object_identifier);
            // [1] list-of-property-references (opening/closing)
            tags::encode_opening_tag(buf, 1);
            for prop_ref in &spec.list_of_property_references {
                prop_ref.encode(buf);
            }
            tags::encode_closing_tag(buf, 1);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut specs = Vec::new();

        while offset < data.len() {
            if specs.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(
                    offset,
                    "RPM request exceeds max decoded items",
                ));
            }

            // [0] object-identifier
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_context(0) {
                return Err(Error::decoding(
                    offset,
                    "RPM request expected context tag 0",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "RPM request truncated at object-id"));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // [1] list-of-property-references (opening tag 1)
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(Error::decoding(
                    offset,
                    "RPM request expected opening tag 1",
                ));
            }
            offset = tag_end;

            let mut prop_refs = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::decoding(offset, "RPM request missing closing tag 1"));
                }
                if prop_refs.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::decoding(offset, "RPM property refs exceeds max"));
                }
                // Check for closing tag 1
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = tag_end;
                    break;
                }
                // Decode property reference starting from current offset (not tag_end)
                let (pr, new_offset) = PropertyReference::decode(data, offset)?;
                prop_refs.push(pr);
                offset = new_offset;
            }

            specs.push(ReadAccessSpecification {
                object_identifier,
                list_of_property_references: prop_refs,
            });
        }

        Ok(Self {
            list_of_read_access_specs: specs,
        })
    }
}

// ---------------------------------------------------------------------------
// ReadPropertyMultipleACK
// ---------------------------------------------------------------------------

/// A single result element: success (value) or failure (error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadResultElement {
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    /// Success: raw application-tagged value bytes. Mutually exclusive with `error`.
    pub property_value: Option<Vec<u8>>,
    /// Failure: (ErrorClass, ErrorCode). Mutually exclusive with `property_value`.
    pub error: Option<(ErrorClass, ErrorCode)>,
}

/// Results for a single object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAccessResult {
    pub object_identifier: ObjectIdentifier,
    pub list_of_results: Vec<ReadResultElement>,
}

/// ReadPropertyMultiple-ACK service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyMultipleACK {
    pub list_of_read_access_results: Vec<ReadAccessResult>,
}

impl ReadPropertyMultipleACK {
    pub fn encode(&self, buf: &mut BytesMut) {
        for result in &self.list_of_read_access_results {
            // [0] object-identifier
            primitives::encode_ctx_object_id(buf, 0, &result.object_identifier);
            // [1] list-of-results (opening/closing)
            tags::encode_opening_tag(buf, 1);
            for elem in &result.list_of_results {
                // [2] property-identifier
                primitives::encode_ctx_unsigned(buf, 2, elem.property_identifier.to_raw() as u64);
                // [3] property-array-index (optional)
                if let Some(idx) = elem.property_array_index {
                    primitives::encode_ctx_unsigned(buf, 3, idx as u64);
                }
                if let Some(ref value) = elem.property_value {
                    // [4] property-value (opening/closing)
                    tags::encode_opening_tag(buf, 4);
                    buf.extend_from_slice(value);
                    tags::encode_closing_tag(buf, 4);
                } else if let Some((class, code)) = elem.error {
                    // [5] property-access-error (opening/closing)
                    tags::encode_opening_tag(buf, 5);
                    primitives::encode_app_enumerated(buf, class.to_raw() as u32);
                    primitives::encode_app_enumerated(buf, code.to_raw() as u32);
                    tags::encode_closing_tag(buf, 5);
                }
            }
            tags::encode_closing_tag(buf, 1);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut results = Vec::new();

        while offset < data.len() {
            if results.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(offset, "RPM ACK exceeds max decoded items"));
            }

            // [0] object-identifier
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_context(0) {
                return Err(Error::decoding(offset, "RPM ACK expected context tag 0"));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "RPM ACK truncated at object-id"));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // [1] list-of-results (opening tag 1)
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(Error::decoding(offset, "RPM ACK expected opening tag 1"));
            }
            offset = tag_end;

            let mut elements = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::decoding(offset, "RPM ACK missing closing tag 1"));
                }
                if elements.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::decoding(offset, "RPM ACK results exceeds max"));
                }
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = tag_end;
                    break;
                }

                // [2] property-identifier
                if !tag.is_context(2) {
                    return Err(Error::decoding(offset, "RPM ACK expected context tag 2"));
                }
                let end = tag_end + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(tag_end, "RPM ACK truncated at property-id"));
                }
                let prop_raw = primitives::decode_unsigned(&data[tag_end..end])?;
                let prop_raw = u32::try_from(prop_raw)
                    .map_err(|_| Error::decoding(tag_end, "RPM ACK property-id exceeds u32"))?;
                let property_identifier = PropertyIdentifier::from_raw(prop_raw);
                offset = end;

                // [3] property-array-index (optional)
                let mut array_index = None;
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_context(3) {
                    let end = tag_end + tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(tag_end, "RPM ACK truncated at array-index"));
                    }
                    let value = primitives::decode_unsigned(&data[tag_end..end])?;
                    array_index = Some(u32::try_from(value).map_err(|_| {
                        Error::decoding(tag_end, "RPM ACK array-index exceeds u32")
                    })?);
                    offset = end;
                    let (tag, tag_end) = tags::decode_tag(data, offset)?;
                    if tag.is_opening_tag(4) {
                        let (value_bytes, new_offset) = extract_property_value(
                            data,
                            tag_end,
                            4,
                            property_identifier,
                            &[
                                PropertyValueBoundary::Context(2),
                                PropertyValueBoundary::Closing(1),
                            ],
                        )?;
                        elements.push(ReadResultElement {
                            property_identifier,
                            property_array_index: array_index,
                            property_value: Some(value_bytes.to_vec()),
                            error: None,
                        });
                        offset = new_offset;
                    } else if tag.is_opening_tag(5) {
                        let (error_class, error_code, new_offset) =
                            decode_error_pair(data, tag_end)?;
                        elements.push(ReadResultElement {
                            property_identifier,
                            property_array_index: array_index,
                            property_value: None,
                            error: Some((error_class, error_code)),
                        });
                        offset = new_offset;
                    } else {
                        return Err(Error::decoding(offset, "RPM ACK expected tag 4 or 5"));
                    }
                } else if tag.is_opening_tag(4) {
                    // [4] property-value
                    let (value_bytes, new_offset) = extract_property_value(
                        data,
                        tag_end,
                        4,
                        property_identifier,
                        &[
                            PropertyValueBoundary::Context(2),
                            PropertyValueBoundary::Closing(1),
                        ],
                    )?;
                    elements.push(ReadResultElement {
                        property_identifier,
                        property_array_index: array_index,
                        property_value: Some(value_bytes.to_vec()),
                        error: None,
                    });
                    offset = new_offset;
                } else if tag.is_opening_tag(5) {
                    // [5] property-access-error
                    let (error_class, error_code, new_offset) = decode_error_pair(data, tag_end)?;
                    elements.push(ReadResultElement {
                        property_identifier,
                        property_array_index: array_index,
                        property_value: None,
                        error: Some((error_class, error_code)),
                    });
                    offset = new_offset;
                } else {
                    return Err(Error::decoding(offset, "RPM ACK expected tag 3, 4, or 5"));
                }
            }

            results.push(ReadAccessResult {
                object_identifier,
                list_of_results: elements,
            });
        }

        Ok(Self {
            list_of_read_access_results: results,
        })
    }
}

/// Decode an error-class + error-code pair from inside opening/closing tag 5,
/// followed by consuming the closing tag.
fn decode_error_pair(data: &[u8], offset: usize) -> Result<(ErrorClass, ErrorCode, usize), Error> {
    // error-class: app-tagged enumerated
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            "RPM error class: expected application-tagged enumerated",
        ));
    }
    let end = pos + tag.length as usize;
    if end > data.len() {
        return Err(Error::decoding(pos, "RPM error truncated at error-class"));
    }
    let error_class_raw = primitives::decode_unsigned(&data[pos..end])?;
    let error_class_raw = u16::try_from(error_class_raw).map_err(|_| {
        Error::decoding(
            pos,
            format!("RPM error class {error_class_raw} exceeds u16"),
        )
    })?;
    let error_class = ErrorClass::from_raw(error_class_raw);
    let mut offset = end;

    // error-code: app-tagged enumerated
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            "RPM error code: expected application-tagged enumerated",
        ));
    }
    let end = pos + tag.length as usize;
    if end > data.len() {
        return Err(Error::decoding(pos, "RPM error truncated at error-code"));
    }
    let error_code_raw = primitives::decode_unsigned(&data[pos..end])?;
    let error_code_raw = u16::try_from(error_code_raw).map_err(|_| {
        Error::decoding(pos, format!("RPM error code {error_code_raw} exceeds u16"))
    })?;
    let error_code = ErrorCode::from_raw(error_code_raw);
    offset = end;

    // closing tag 5
    let (tag, tag_end) = tags::decode_tag(data, offset)?;
    if !tag.is_closing_tag(5) {
        return Err(Error::decoding(offset, "RPM error expected closing tag 5"));
    }

    Ok((error_class, error_code, tag_end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn ack_with_property_fields(property: &[u8], array_index: Option<&[u8]>) -> BytesMut {
        let mut buf = BytesMut::new();
        let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        primitives::encode_ctx_object_id(&mut buf, 0, &object);
        tags::encode_opening_tag(&mut buf, 1);
        primitives::encode_ctx_octet_string(&mut buf, 2, property);
        if let Some(array_index) = array_index {
            primitives::encode_ctx_octet_string(&mut buf, 3, array_index);
        }
        tags::encode_opening_tag(&mut buf, 4);
        primitives::encode_app_null(&mut buf);
        tags::encode_closing_tag(&mut buf, 4);
        tags::encode_closing_tag(&mut buf, 1);
        buf
    }

    #[test]
    fn request_single_object_round_trip() {
        let req = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_property_references: vec![
                    PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::OBJECT_NAME,
                        property_array_index: None,
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = ReadPropertyMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn request_multi_object_round_trip() {
        let req = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![
                ReadAccessSpecification {
                    object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                    list_of_property_references: vec![PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    }],
                },
                ReadAccessSpecification {
                    object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 3).unwrap(),
                    list_of_property_references: vec![PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    }],
                },
            ],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = ReadPropertyMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn ack_success_round_trip() {
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_results: vec![ReadResultElement {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: Some(vec![0x44, 0x42, 0x90, 0x00, 0x00]),
                    error: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = ReadPropertyMultipleACK::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn ack_mixed_success_error_round_trip() {
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_results: vec![
                    ReadResultElement {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        property_value: Some(vec![0x44, 0x42, 0x90, 0x00, 0x00]),
                        error: None,
                    },
                    ReadResultElement {
                        property_identifier: PropertyIdentifier::from_raw(9999),
                        property_array_index: None,
                        property_value: None,
                        error: Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let decoded = ReadPropertyMultipleACK::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn legacy_event_parameters_do_not_consume_the_next_result() {
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap(),
                list_of_results: vec![
                    ReadResultElement {
                        property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
                        property_array_index: None,
                        property_value: Some(vec![0xfe, 0xff, 1, 0xff, 0xff, 0x4f, 2, 0xff, 0xff]),
                        error: None,
                    },
                    ReadResultElement {
                        property_identifier: PropertyIdentifier::NOTIFICATION_CLASS,
                        property_array_index: None,
                        property_value: Some(vec![0x21, 8]),
                        error: None,
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);

        assert_eq!(ReadPropertyMultipleACK::decode(&buf).unwrap(), ack);
    }

    #[test]
    fn ambiguous_legacy_event_parameters_result_is_rejected() {
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap(),
                list_of_results: vec![ReadResultElement {
                    property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
                    property_array_index: None,
                    property_value: Some(vec![
                        0xfe, 0xff, 0xaa, 0xff, 0xff, 0x4f, 0x29, 0x53, 0x4e, 0xfe, 0xff, 0xbb,
                        0xff, 0xff,
                    ]),
                    error: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);

        assert!(ReadPropertyMultipleACK::decode(&buf).is_err());
    }

    #[test]
    fn error_pair_values_must_be_enumerated_and_fit_u16() {
        let encode_pair = |error_class, error_code| {
            let mut buf = BytesMut::new();
            primitives::encode_app_enumerated(&mut buf, error_class);
            primitives::encode_app_enumerated(&mut buf, error_code);
            tags::encode_closing_tag(&mut buf, 5);
            buf
        };

        for (error_class, error_code, field, value) in
            [(65_536, 0, "class", 65_536), (0, 65_537, "code", 65_537)]
        {
            let encoded = encode_pair(error_class, error_code);
            let error = decode_error_pair(&encoded, 0).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("RPM error {field} {value}")),
                "unexpected error for error {field} {value}: {error}"
            );
        }

        let encoded = encode_pair(u16::MAX as u32, u16::MAX as u32);
        let (error_class, error_code, consumed) = decode_error_pair(&encoded, 0).unwrap();
        assert_eq!(error_class.to_raw(), u16::MAX);
        assert_eq!(error_code.to_raw(), u16::MAX);
        assert_eq!(consumed, encoded.len());

        let mut leading_zero = BytesMut::from(&[0x93, 0, 0, 2, 0x93, 0, 0, 32][..]);
        tags::encode_closing_tag(&mut leading_zero, 5);
        let (error_class, error_code, consumed) = decode_error_pair(&leading_zero, 0).unwrap();
        assert_eq!(error_class, ErrorClass::PROPERTY);
        assert_eq!(error_code, ErrorCode::UNKNOWN_PROPERTY);
        assert_eq!(consumed, leading_zero.len());

        for (tag_offset, field) in [(0, "class"), (2, "code")] {
            let mut encoded = encode_pair(2, 32);
            encoded[tag_offset] = 0x21; // Application Unsigned with one content octet.
            let error = decode_error_pair(&encoded, 0).unwrap_err();
            assert!(
                error.to_string().contains(&format!(
                    "RPM error {field}: expected application-tagged enumerated"
                )),
                "unexpected error for error {field} tag: {error}"
            );
        }
    }

    #[test]
    fn ack_property_values_must_fit_u32() {
        let max_with_leading_zero = [0, 0xFF, 0xFF, 0xFF, 0xFF];
        let encoded =
            ack_with_property_fields(&max_with_leading_zero, Some(&max_with_leading_zero));
        let decoded = ReadPropertyMultipleACK::decode(&encoded).unwrap();
        let result = &decoded.list_of_read_access_results[0].list_of_results[0];
        assert_eq!(result.property_identifier.to_raw(), u32::MAX);
        assert_eq!(result.property_array_index, Some(u32::MAX));

        for overflow in [u32::MAX as u64 + 1, u64::MAX] {
            let overflow = overflow.to_be_bytes();
            let property = ack_with_property_fields(&overflow, None);
            assert!(ReadPropertyMultipleACK::decode(&property).is_err());

            let index = ack_with_property_fields(&[1], Some(&overflow));
            assert!(ReadPropertyMultipleACK::decode(&index).is_err());
        }
    }

    #[test]
    fn rpm_requires_object_context_tag_zero() {
        let mut encoded = ack_with_property_fields(&[85], None);
        encoded[0] = 0x1C;
        assert!(ReadPropertyMultipleACK::decode(&encoded).is_err());

        let mut request = BytesMut::new();
        let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        primitives::encode_ctx_object_id(&mut request, 1, &object);
        tags::encode_opening_tag(&mut request, 1);
        tags::encode_closing_tag(&mut request, 1);
        assert!(ReadPropertyMultipleRequest::decode(&request).is_err());
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_rpm_request_truncated_1_byte() {
        let req = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(ReadPropertyMultipleRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_rpm_request_truncated_3_bytes() {
        let req = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(ReadPropertyMultipleRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_rpm_request_truncated_half() {
        let req = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(ReadPropertyMultipleRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_rpm_request_invalid_tag() {
        assert!(ReadPropertyMultipleRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_rpm_ack_truncated_1_byte() {
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_results: vec![ReadResultElement {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: Some(vec![0x44, 0x42, 0x90, 0x00, 0x00]),
                    error: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(ReadPropertyMultipleACK::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_rpm_ack_truncated_3_bytes() {
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_results: vec![ReadResultElement {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: Some(vec![0x44, 0x42, 0x90, 0x00, 0x00]),
                    error: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        assert!(ReadPropertyMultipleACK::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_rpm_ack_truncated_half() {
        let ack = ReadPropertyMultipleACK {
            list_of_read_access_results: vec![ReadAccessResult {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                list_of_results: vec![ReadResultElement {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: Some(vec![0x44, 0x42, 0x90, 0x00, 0x00]),
                    error: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(ReadPropertyMultipleACK::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_rpm_ack_invalid_tag() {
        assert!(ReadPropertyMultipleACK::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
