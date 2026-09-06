//! `BACnetEventParameter` framing tests (#154): golden Clause-20.2 byte
//! vectors, per-alternative round-trips, and malformed/reserved negatives.

use super::*;

fn round_trip(value: &BACnetEventParameter) {
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, value).unwrap();
    let (decoded, end) = decode_event_parameter(&buf, 0).unwrap();
    assert_eq!(&decoded, value);
    assert_eq!(end, buf.len());
}

// --- Golden wire vectors (hand-computed per Clause 20.2.1.5/20.2.1.6) -------

#[test]
fn change_of_state_golden() {
    // change-of-state [1] { time-delay [0] 7, list-of-values [1] { [0] TRUE,
    // unsigned-value [11] 42 } }
    let value = BACnetEventParameter::ChangeOfState {
        time_delay: 7,
        list_of_values: vec![
            BACnetPropertyStates::BooleanValue(true),
            BACnetPropertyStates::UnsignedValue(42),
        ],
    };
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x1E, // opening [1]
            0x09, 0x07, // time-delay [0] Unsigned 7
            0x1E, // list-of-values [1] opening
            0x09, 0x01, // boolean-value [0] TRUE
            0xB9, 0x2A, // unsigned-value [11] 42
            0x1F, // list-of-values closing
            0x1F, // change-of-state closing
        ]
    );
    let (decoded, end) = decode_event_parameter(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn change_of_state_encode_rejects_malformed_proprietary_body_atomically() {
    let value = BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(64, vec![0xde]).unwrap(),
        )],
    };
    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(encode_event_parameter(&mut untouched, &value).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);
}

#[test]
fn change_of_value_increment_golden() {
    let value = BACnetEventParameter::ChangeOfValue {
        time_delay: 2,
        criteria: bacnet_types::constructed::ChangeOfValueCriteria::ReferencedPropertyIncrement(
            5.0,
        ),
    };
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x2E, // opening [2]
            0x09, 0x02, // time-delay [0] Unsigned 2
            0x1E, // cov-criteria [1] opening (CHOICE = explicitly tagged)
            0x1C, 0x40, 0xA0, 0x00, 0x00, // referenced-property-increment [1] REAL 5.0
            0x1F, // cov-criteria closing
            0x2F, // change-of-value closing
        ]
    );
    let (decoded, end) = decode_event_parameter(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn change_of_value_bitmask_golden() {
    let value = BACnetEventParameter::ChangeOfValue {
        time_delay: 2,
        criteria: bacnet_types::constructed::ChangeOfValueCriteria::Bitmask {
            unused_bits: 5,
            data: vec![0xE0],
        },
    };
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x2E, 0x09, 0x02, // opening [2], time-delay 2
            0x1E, // cov-criteria opening
            0x0A, 0x05, 0xE0, // bitmask [0] BIT STRING (2 octets content)
            0x1F, 0x2F,
        ]
    );
    let (decoded, end) = decode_event_parameter(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn floating_limit_golden() {
    let value = BACnetEventParameter::FloatingLimit {
        time_delay: 3,
        setpoint_reference: dopr_ai(5, 85),
        low_diff_limit: 1.0,
        high_diff_limit: 2.0,
        deadband: 0.5,
    };
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x4E, // opening [4]
            0x09, 0x03, // time-delay [0] Unsigned 3
            0x1E, // setpoint-reference [1] opening
            0x0C, 0x00, 0x00, 0x00, 0x05, // [0] object-identifier analog-input,5
            0x19, 0x55, // [1] property-identifier 85 (present-value)
            0x1F, // setpoint-reference closing
            0x2C, 0x3F, 0x80, 0x00, 0x00, // low-diff-limit [2] REAL 1.0
            0x3C, 0x40, 0x00, 0x00, 0x00, // high-diff-limit [3] REAL 2.0
            0x4C, 0x3F, 0x00, 0x00, 0x00, // deadband [4] REAL 0.5
            0x4F, // floating-limit closing
        ]
    );
    let (decoded, end) = decode_event_parameter(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn out_of_range_golden() {
    let value = BACnetEventParameter::OutOfRange {
        time_delay: 7,
        low_limit: 10.0,
        high_limit: 90.0,
        deadband: 2.0,
    };
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x5E, // opening [5]
            0x09, 0x07, // time-delay [0] Unsigned 7
            0x1C, 0x41, 0x20, 0x00, 0x00, // low-limit [1] REAL 10.0
            0x2C, 0x42, 0xB4, 0x00, 0x00, // high-limit [2] REAL 90.0
            0x3C, 0x40, 0x00, 0x00, 0x00, // deadband [3] REAL 2.0
            0x5F, // out-of-range closing
        ]
    );
    let (decoded, end) = decode_event_parameter(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn extended_golden() {
    let value = BACnetEventParameter::Extended {
        vendor_id: 42,
        extended_event_type: 99,
        // Application OCTET STRING containing a byte that resembles closing [2].
        parameters: vec![0x61, 0x2F],
    };
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x9E, // opening [9]
            0x09, 0x2A, // vendor-id [0] Unsigned 42
            0x19, 0x63, // extended-event-type [1] Unsigned 99
            0x2E, // parameters [2] opening
            0x61, 0x2F, 0x2F, // OCTET STRING 0x2F, parameters closing
            0x9F, // extended closing
        ]
    );
    let (decoded, end) = decode_event_parameter(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn preserved_event_parameters_accept_defined_character_sets() {
    for character_string in [
        vec![0x73, 0x01, 0x03, 0x52],
        vec![0x71, 0x02],
        vec![0x75, 0x05, 0x03, 0x00, 0x00, 0x00, 0x41],
    ] {
        round_trip(&BACnetEventParameter::Extended {
            vendor_id: 42,
            extended_event_type: 99,
            parameters: character_string.clone(),
        });
        round_trip(&BACnetEventParameter::Opaque {
            tag: 200,
            data: character_string,
        });
    }
}

// --- Per-modeled-alternative round-trips -------------------------------------

#[test]
fn change_of_bitstring_round_trip() {
    round_trip(&BACnetEventParameter::ChangeOfBitstring {
        time_delay: 5,
        bitmask: (5, vec![0xE0]),
        list_of_values: vec![(0, vec![0xFF]), (3, vec![0xA0])],
    });
}

#[test]
fn change_of_state_round_trip() {
    round_trip(&BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![
            BACnetPropertyStates::BinaryValue(1),
            BACnetPropertyStates::Reliability(0),
            BACnetPropertyStates::TimerState(2),
        ],
    });
}

#[test]
fn out_of_range_round_trip() {
    round_trip(&BACnetEventParameter::OutOfRange {
        time_delay: 65535,
        low_limit: -1.5,
        high_limit: 250.25,
        deadband: 0.0,
    });
}

#[test]
fn floating_limit_with_remote_reference_round_trip() {
    let mut reference = dopr_ai(5, 85);
    reference.property_array_index = Some(2);
    reference.device_identifier =
        Some(ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 100).unwrap());
    round_trip(&BACnetEventParameter::FloatingLimit {
        time_delay: 1,
        setpoint_reference: reference,
        low_diff_limit: 0.5,
        high_diff_limit: 0.5,
        deadband: 0.25,
    });
}

#[test]
fn extended_empty_parameters_round_trip() {
    round_trip(&BACnetEventParameter::Extended {
        vendor_id: 0,
        extended_event_type: 0,
        parameters: Vec::new(),
    });
}

#[test]
fn extended_reference_parameter_round_trip() {
    let mut parameters = BytesMut::new();
    tags::encode_opening_tag(&mut parameters, 0);
    encode_dopr_body(&mut parameters, &dopr_ai(5, 85));
    tags::encode_closing_tag(&mut parameters, 0);
    round_trip(&BACnetEventParameter::Extended {
        vendor_id: 42,
        extended_event_type: 99,
        parameters: parameters.to_vec(),
    });
}

#[test]
fn opaque_unmodeled_alternatives_preserved() {
    // change-of-life-safety [8] — a valid but unmodeled SEQUENCE alternative:
    // body bytes preserved verbatim through decode->encode.
    let wire_body = [0x09, 0x2A]; // one ctx-0 field standing in for the body
    let mut wire = vec![0x8E]; // opening [8]
    wire.extend_from_slice(&wire_body);
    wire.push(0x8F); // closing [8]
    let (decoded, end) = decode_event_parameter(&wire, 0).unwrap();
    assert_eq!(end, wire.len());
    assert_eq!(
        decoded,
        BACnetEventParameter::Opaque {
            tag: 8,
            data: wire_body.to_vec()
        }
    );
    let mut reencoded = BytesMut::new();
    encode_event_parameter(&mut reencoded, &decoded).unwrap();
    assert_eq!(
        reencoded.as_ref(),
        wire.as_slice(),
        "byte-identical preservation"
    );

    // buffer-ready [10], change-of-timer [22], vendor tag 200
    for tag in [10u8, 22, 200] {
        round_trip(&BACnetEventParameter::Opaque {
            tag,
            data: vec![0x21, 0x03],
        });
    }
}

#[test]
fn legacy_opaque_sentinel_stays_local_to_event_parameters() {
    let value = BACnetEventParameter::Opaque {
        tag: u8::MAX,
        data: vec![0xFF, 0x01, 0x02],
    };
    let mut encoded = BytesMut::new();
    encode_event_parameter(&mut encoded, &value).unwrap();

    let (tag, _) = tags::decode_tag(&encoded, 0).unwrap();
    assert_eq!(tag.class, tags::TagClass::Application);
    assert_eq!(tag.number, tags::app_tag::OCTET_STRING);
    let (decoded, end) = decode_event_parameter(&encoded, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, encoded.len());

    let historical = [0xFE, 0xFF, 1, 0xFF, 0xFF, 0x2F, 2, 0xFF, 0xFF];
    assert!(tags::decode_tag(&historical, 0).is_err());
    let (decoded, end) = decode_event_parameter(&historical, 0).unwrap();
    assert_eq!(
        decoded,
        BACnetEventParameter::Opaque {
            tag: u8::MAX,
            data: vec![1, 0xFF, 0xFF, 0x2F, 2],
        }
    );
    assert_eq!(end, historical.len());
}

// --- Negatives ----------------------------------------------------------------

#[test]
fn reserved_and_deprecated_tags_rejected() {
    // Clause 21 omits [6] (proprietary parallel) and [19]
    // (change-of-reliability has no parameters), deprecated [7], and
    // reserves [12]; decoding any of them is a hard error, not an Opaque.
    // Constructed form (opening tag):
    for tag in [6u8, 7, 12, 19] {
        let mut data = BytesMut::new();
        tags::encode_opening_tag(&mut data, tag);
        data.extend_from_slice(&[0x09, 0x01]);
        tags::encode_closing_tag(&mut data, tag);
        let err = decode_event_parameter(&data, 0).unwrap_err();
        assert!(
            format!("{err}").contains("omitted/deprecated/reserved"),
            "tag [{tag}]: unexpected error {err}"
        );
    }
    // Primitive form (zero-length contents under the reserved tag number).
    for tag in [6u8, 7, 12, 19] {
        let mut data = BytesMut::new();
        tags::encode_tag(&mut data, tag, crate::tags::TagClass::Context, 0);
        let err = decode_event_parameter(&data, 0).unwrap_err();
        assert!(
            format!("{err}").contains("omitted/deprecated/reserved"),
            "primitive tag [{tag}]: unexpected error {err}"
        );
    }
}

#[test]
fn event_parameter_choice_tag_forms_are_enforced() {
    // Sequence alternatives cannot use primitive context tags.
    assert!(decode_event_parameter(&[0x58], 0).is_err());
    assert!(decode_event_parameter(&[0x88], 0).is_err());

    // none [20] is primitive NULL only: no contents and no constructed form.
    assert!(decode_event_parameter(&[0xF9, 20, 0], 0).is_err());
    assert!(decode_event_parameter(&[0xFE, 20, 0xFF, 20], 0).is_err());

    for value in [
        BACnetEventParameter::Opaque {
            tag: 5,
            data: Vec::new(),
        },
        BACnetEventParameter::Opaque {
            tag: 20,
            data: vec![0],
        },
        BACnetEventParameter::Extended {
            vendor_id: 1,
            extended_event_type: 2,
            parameters: vec![0xde],
        },
        BACnetEventParameter::Opaque {
            tag: 200,
            data: vec![0xde],
        },
    ] {
        let mut untouched = BytesMut::from(&[0xaa][..]);
        assert!(encode_event_parameter(&mut untouched, &value).is_err());
        assert_eq!(untouched.as_ref(), &[0xaa]);
    }

    for data in [
        vec![0x01, 0x00],
        vec![0x12],
        vec![0x0e, 0x01, 0x00, 0x0f],
        vec![0x1e, 0x12, 0x1f],
    ] {
        for value in [
            BACnetEventParameter::Extended {
                vendor_id: 1,
                extended_event_type: 2,
                parameters: data.clone(),
            },
            BACnetEventParameter::Opaque {
                tag: 200,
                data: data.clone(),
            },
        ] {
            let mut untouched = BytesMut::from(&[0xaa][..]);
            assert!(encode_event_parameter(&mut untouched, &value).is_err());
            assert_eq!(untouched.as_ref(), &[0xaa]);
        }
    }
}

#[test]
fn event_parameter_encoder_accounts_for_outer_nesting_atomically() {
    let mut accepted_opaque_body = vec![0x0e; tags::MAX_CONTEXT_NESTING_DEPTH - 1];
    accepted_opaque_body.extend(vec![0x0f; tags::MAX_CONTEXT_NESTING_DEPTH - 1]);
    round_trip(&BACnetEventParameter::Opaque {
        tag: 200,
        data: accepted_opaque_body,
    });

    let accepted_property_depth = tags::MAX_CONTEXT_NESTING_DEPTH - 3;
    let mut accepted_property_body = vec![0x0e; accepted_property_depth];
    accepted_property_body.extend(vec![0x0f; accepted_property_depth]);
    round_trip(&BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(64, accepted_property_body).unwrap(),
        )],
    });

    let mut deepest_opaque_body = vec![0x0e; tags::MAX_CONTEXT_NESTING_DEPTH];
    deepest_opaque_body.extend(vec![0x0f; tags::MAX_CONTEXT_NESTING_DEPTH]);

    let property_body_depth = tags::MAX_CONTEXT_NESTING_DEPTH - 2;
    let mut deepest_property_body = vec![0x0e; property_body_depth];
    deepest_property_body.extend(vec![0x0f; property_body_depth]);
    let too_deep_property_state = BACnetPropertyStates::Other(
        BACnetProprietaryPropertyState::constructed(64, deepest_property_body).unwrap(),
    );

    for value in [
        BACnetEventParameter::Opaque {
            tag: 200,
            data: deepest_opaque_body,
        },
        BACnetEventParameter::ChangeOfState {
            time_delay: 0,
            list_of_values: vec![too_deep_property_state.clone()],
        },
    ] {
        let mut untouched = BytesMut::from(&[0xaa][..]);
        assert!(encode_event_parameter(&mut untouched, &value).is_err());
        assert_eq!(untouched.as_ref(), &[0xaa]);
    }

    let mut raw = BytesMut::new();
    tags::encode_opening_tag(&mut raw, 1);
    primitives::encode_ctx_unsigned(&mut raw, 0, 0);
    tags::encode_opening_tag(&mut raw, 1);
    encode_property_state(&mut raw, &too_deep_property_state).unwrap();
    tags::encode_closing_tag(&mut raw, 1);
    tags::encode_closing_tag(&mut raw, 1);
    assert!(decode_event_parameter(&raw, 0).is_err());
}

#[test]
fn truncated_and_unbalanced_rejected() {
    // Opening [5] without its closing tag.
    let data = [0x5E, 0x09, 0x07, 0x1C, 0x41, 0x20, 0x00, 0x00];
    assert!(decode_event_parameter(&data, 0).is_err());
    // Truncated mid-tag-header.
    assert!(decode_event_parameter(&[0x5E], 0).is_err());
    // Empty.
    assert!(decode_event_parameter(&[], 0).is_err());
    // Mismatched closing tag (closing [4] for an opening [5]).
    let data = [
        0x5E, 0x09, 0x07, 0x1C, 0x41, 0x20, 0x00, 0x00, 0x2C, 0x42, 0xB4, 0x00, 0x00, 0x3C, 0x40,
        0x00, 0x00, 0x00, 0x4F,
    ];
    assert!(decode_event_parameter(&data, 0).is_err());
    // Closing tag at top level.
    assert!(decode_event_parameter(&[0x5F], 0).is_err());
}

#[test]
fn wrong_inner_field_tag_rejected() {
    // out-of-range whose time-delay arrives under ctx tag [9] instead of [0].
    let data = [0x5E, 0x99, 0x07, 0x5F];
    assert!(decode_event_parameter(&data, 0).is_err());
    // floating-limit setpoint-reference must be a constructed [1] member:
    // a primitive ctx tag 1 there breaks the parse.
    let data = [
        0x4E, 0x09, 0x03, 0x11, 0x00, 0x2C, 0x3F, 0x80, 0x00, 0x00, 0x4F,
    ];
    assert!(decode_event_parameter(&data, 0).is_err());
}
