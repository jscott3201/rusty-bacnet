use super::*;
use bacnet_types::constructed::PropertyReference;
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
    ack.encode(&mut buf).unwrap();
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
    ack.encode(&mut buf).unwrap();
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
    // The inner `ff ff 4f` could end the value, so the encoder refuses it
    // (#2376) and leaves the buffer as it was.
    let mut buf = BytesMut::from(&[0xaa][..]);
    assert!(ack.encode(&mut buf).is_err());
    assert_eq!(buf.as_ref(), [0xaa]);

    // From an older peer it still decodes: `02` after that pair is no valid
    // next field.
    let wire = [
        0x0c, 0x02, 0x40, 0x00, 0x01, 0x1e, 0x29, 0x53, 0x4e, 0xfe, 0xff, 1, 0xff, 0xff, 0x4f, 2,
        0xff, 0xff, 0x4f, 0x29, 0x11, 0x4e, 0x21, 8, 0x4f, 0x1f,
    ];
    assert_eq!(ReadPropertyMultipleACK::decode(&wire).unwrap(), ack);
}

#[test]
fn an_older_peers_legacy_result_with_two_possible_ends_is_read_to_the_first() {
    // An older peer's value holding `ff ff 4f` and then what looks like a
    // next result: either pair could end it, and the first does.
    let wire = [
        0x0c, 0x02, 0x40, 0x00, 0x01, 0x1e, 0x29, 0x53, 0x4e, 0xfe, 0xff, 0xaa, 0xff, 0xff, 0x4f,
        0x29, 0x53, 0x4e, 0xfe, 0xff, 0xbb, 0xff, 0xff, 0x4f, 0x1f,
    ];
    let legacy = |marker| ReadResultElement {
        property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
        property_array_index: None,
        property_value: Some(vec![0xfe, 0xff, marker, 0xff, 0xff]),
        error: None,
    };
    let ack = ReadPropertyMultipleACK::decode(&wire).unwrap();
    assert_eq!(
        ack.list_of_read_access_results[0].list_of_results,
        [legacy(0xaa), legacy(0xbb)]
    );
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

    // The shared application reader names the member and the width.
    for (error_class, error_code, field) in [(65_536, 0, "class"), (0, 65_537, "code")] {
        let encoded = encode_pair(error_class, error_code);
        let error = decode_error_pair(&encoded, 0).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("RPM error {field}: ENUMERATED exceeds u16")),
            "unexpected error for error {field}: {error}"
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
                "RPM error {field}: expected application-tagged ENUMERATED"
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
    ack.encode(&mut buf).unwrap();
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
    ack.encode(&mut buf).unwrap();
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
    ack.encode(&mut buf).unwrap();
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

#[test]
fn request_codec_is_the_shared_specification_codec_back_to_back() {
    // AV-2 Present_Value, then Priority_Array[16]: the octets of one
    // specification, as a Group's List_Of_Group_Members element carries it.
    let spec = ReadAccessSpecification {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap(),
        list_of_property_references: vec![
            PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            },
            PropertyReference {
                property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                property_array_index: Some(16),
            },
        ],
    };
    let mut octets = BytesMut::new();
    encode_read_access_specification(&mut octets, &spec);
    assert_eq!(
        &octets[..],
        &[0x0C, 0, 0x80, 0, 2, 0x1E, 0x09, 85, 0x09, 87, 0x19, 16, 0x1F]
    );
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![spec.clone(), spec],
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    assert_eq!(&encoded[..], &[&octets[..], &octets[..]].concat()[..]);
    assert_eq!(
        ReadPropertyMultipleRequest::decode(&encoded).unwrap(),
        request
    );
}

#[test]
fn read_access_result_encodes_alone_as_in_the_ack() {
    let result = ReadAccessResult {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 9).unwrap(),
        list_of_results: vec![ReadResultElement {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: None,
            error: Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
        }],
    };
    let mut alone = BytesMut::new();
    result.encode(&mut alone).unwrap();
    assert_eq!(
        &alone[..],
        &[0x0C, 0, 0, 0, 9, 0x1E, 0x29, 85, 0x5E, 0x91, 1, 0x91, 31, 0x5F, 0x1F]
    );
    let mut ack = BytesMut::new();
    ReadPropertyMultipleACK {
        list_of_read_access_results: vec![result],
    }
    .encode(&mut ack).unwrap();
    assert_eq!(ack, alone);
}
