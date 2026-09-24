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
fn request_empty_nested_list_is_transactional() {
    let object = ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap();
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![
            ReadAccessSpecification {
                object_identifier: object,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: Some(0),
                }],
            },
            ReadAccessSpecification {
                object_identifier: object,
                list_of_property_references: vec![],
            },
        ],
    };
    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(request.encode(&mut output).is_err());
    assert_eq!(
        &output[..],
        b"prefix",
        "invalid request must not append a partial prefix"
    );
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
    req.encode(&mut buf).unwrap();
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
    req.encode(&mut buf).unwrap();
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
                    0xfe, 0xff, 0xaa, 0xff, 0xff, 0x4f, 0x29, 0x53, 0x4e, 0xfe, 0xff, 0xbb, 0xff,
                    0xff,
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
    let encoded = ack_with_property_fields(&max_with_leading_zero, Some(&max_with_leading_zero));
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
    req.encode(&mut buf).unwrap();
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
    req.encode(&mut buf).unwrap();
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
    req.encode(&mut buf).unwrap();
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

#[test]
fn request_empty_outer_rejects_and_explicit_zero_index_matches_wire_vector() {
    let mut bytes = BytesMut::from(&b"prefix"[..]);
    assert!(ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![]
    }
    .encode(&mut bytes)
    .is_err());
    assert_eq!(&bytes[..], b"prefix");
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap(),
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: Some(0),
            }],
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    assert_eq!(
        &bytes[6..],
        &[0x0c, 2, 0, 0, 10, 0x1e, 0x09, 77, 0x19, 0, 0x1f]
    );
}
