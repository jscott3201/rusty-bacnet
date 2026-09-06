//! `BACnetFaultParameter` framing tests (#154): golden wire vectors,
//! per-alternative round-trips, and malformed-input negatives.

use super::*;
use bacnet_types::constructed::FaultParameters;

fn round_trip(value: &FaultParameters) {
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, value).unwrap();
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(&decoded, value);
    assert_eq!(end, buf.len());
}

// --- Golden wire vectors ------------------------------------------------------

#[test]
fn fault_none_golden_primitive_null() {
    // none [0] NULL — a primitive context tag 0 with no contents: 0x08.
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, &FaultParameters::FaultNone).unwrap();
    assert_eq!(buf.as_ref(), &[0x08]);
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(decoded, FaultParameters::FaultNone);
    assert_eq!(end, 1);
}

#[test]
fn fault_out_of_range_golden() {
    // min/max-normal-value are inner CHOICEs explicitly tagged [0]/[1]
    // around application-tagged alternatives; f64 selects `double` (tag 5).
    let value = FaultParameters::FaultOutOfRange {
        min_normal: 10.0,
        max_normal: 20.0,
    };
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x6E, // opening [6]
            0x0E, // min-normal-value [0] opening
            // Double 10.0 — 8 contents octets use the extended-length form
            // (tag 0x55, then one length octet 0x08).
            0x55, 0x08, 0x40, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, // min closing
            0x1E, // max-normal-value [1] opening
            0x55, 0x08, 0x40, 0x34, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Double 20.0
            0x1F, // max closing
            0x6F, // fault-out-of-range closing
        ]
    );
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn fault_state_golden() {
    let value = FaultParameters::FaultState {
        fault_values: vec![
            BACnetPropertyStates::BooleanValue(false),
            BACnetPropertyStates::UnsignedValue(3),
        ],
    };
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x4E, // opening [4]
            0x0E, // list-of-fault-values [0] opening
            0x09, 0x00, // boolean-value [0] FALSE
            0xB9, 0x03, // unsigned-value [11] 3
            0x0F, // list closing
            0x4F, // fault-state closing
        ]
    );
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn fault_state_encode_rejects_malformed_proprietary_body_atomically() {
    let value = FaultParameters::FaultState {
        fault_values: vec![BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(64, vec![0xde]).unwrap(),
        )],
    };
    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(encode_fault_parameters(&mut untouched, &value).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);
}

#[test]
fn fault_state_encoder_accounts_for_outer_nesting_atomically() {
    let accepted_depth = tags::MAX_CONTEXT_NESTING_DEPTH - 3;
    let mut accepted_body = vec![0x0e; accepted_depth];
    accepted_body.extend(vec![0x0f; accepted_depth]);
    round_trip(&FaultParameters::FaultState {
        fault_values: vec![BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(64, accepted_body).unwrap(),
        )],
    });

    let body_depth = tags::MAX_CONTEXT_NESTING_DEPTH - 2;
    let mut body = vec![0x0e; body_depth];
    body.extend(vec![0x0f; body_depth]);
    let too_deep_state =
        BACnetPropertyStates::Other(BACnetProprietaryPropertyState::constructed(64, body).unwrap());
    let value = FaultParameters::FaultState {
        fault_values: vec![too_deep_state.clone()],
    };

    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(encode_fault_parameters(&mut untouched, &value).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);

    let mut raw = BytesMut::new();
    tags::encode_opening_tag(&mut raw, 4);
    tags::encode_opening_tag(&mut raw, 0);
    encode_property_state(&mut raw, &too_deep_state).unwrap();
    tags::encode_closing_tag(&mut raw, 0);
    tags::encode_closing_tag(&mut raw, 4);
    assert!(decode_fault_parameters(&raw, 0).is_err());
}

#[test]
fn fault_character_string_golden() {
    let value = FaultParameters::FaultCharacterString {
        fault_values: vec!["alarm".to_string()],
    };
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x1E, // opening [1]
            0x0E, // list-of-fault-values [0] opening
            0x75, 0x06, 0x00, b'a', b'l', b'a', b'r', b'm', // CharacterString "alarm"
            0x0F, 0x1F,
        ]
    );
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn fault_extended_golden() {
    let value = FaultParameters::FaultExtended {
        vendor_id: 42,
        extended_fault_type: 7,
        // Application OCTET STRING containing a byte that resembles closing [2].
        parameters: vec![0x61, 0x2F],
    };
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x2E, // opening [2]
            0x09, 0x2A, // vendor-id [0] Unsigned 42
            0x19, 0x07, // extended-fault-type [1] Unsigned 7
            0x2E, 0x61, 0x2F, 0x2F, // parameters [2]
            0x2F, // closing
        ]
    );
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn fault_extended_rejects_malformed_parameters_atomically() {
    let value = FaultParameters::FaultExtended {
        vendor_id: 42,
        extended_fault_type: 7,
        parameters: vec![0xde],
    };
    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(encode_fault_parameters(&mut untouched, &value).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);
}

#[test]
fn fault_extended_reference_parameter_round_trip() {
    let mut parameters = BytesMut::new();
    tags::encode_opening_tag(&mut parameters, 0);
    encode_dopr_body(&mut parameters, &dopr_ai(5, 85));
    tags::encode_closing_tag(&mut parameters, 0);
    round_trip(&FaultParameters::FaultExtended {
        vendor_id: 42,
        extended_fault_type: 7,
        parameters: parameters.to_vec(),
    });
}

#[test]
fn fault_extended_accepts_ucs4_character_strings() {
    round_trip(&FaultParameters::FaultExtended {
        vendor_id: 42,
        extended_fault_type: 7,
        parameters: vec![0x75, 0x05, 0x03, 0x00, 0x00, 0x00, 0x41],
    });
}

#[test]
fn fault_extended_rejects_malformed_application_forms_atomically() {
    for parameters in [
        vec![0x01, 0x00],
        vec![0x12],
        vec![0x0e, 0x01, 0x00, 0x0f],
        vec![0x1e, 0x12, 0x1f],
    ] {
        let value = FaultParameters::FaultExtended {
            vendor_id: 42,
            extended_fault_type: 7,
            parameters,
        };
        let mut untouched = BytesMut::from(&[0xaa][..]);
        assert!(encode_fault_parameters(&mut untouched, &value).is_err());
        assert_eq!(untouched.as_ref(), &[0xaa]);
    }
}

#[test]
fn fault_life_safety_golden() {
    let value = FaultParameters::FaultLifeSafety {
        fault_values: vec![1, 2, 3],
        mode_for_reference: dopr_ai(1, 85),
    };
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x3E, // opening [3]
            0x0E, // list-of-fault-values [0] opening
            0x91, 0x01, 0x91, 0x02, 0x91, 0x03, // application ENUMERATED 1,2,3
            0x0F, // list closing
            0x1E, // mode-property-reference [1] opening
            0x0C, 0x00, 0x00, 0x00, 0x01, // [0] object-identifier analog-input,1
            0x19, 0x55, // [1] property-identifier 85
            0x1F, // reference closing
            0x3F, // fault-life-safety closing
        ]
    );
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn fault_status_flags_golden() {
    let value = FaultParameters::FaultStatusFlags {
        reference: dopr_ai(1, 111),
    };
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x5E, 0x0E, // openings [5],[0]
            0x0C, 0x00, 0x00, 0x00, 0x01, // object-identifier analog-input,1
            0x19, 0x6F, // property-identifier 111 (status-flags)
            0x0F, 0x5F,
        ]
    );
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

#[test]
fn fault_listed_golden_full_reference() {
    let mut reference = dopr_ai(1, 85);
    reference.property_array_index = Some(3);
    reference.device_identifier = Some(
        bacnet_types::primitives::ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 8)
            .unwrap(),
    );
    let value = FaultParameters::FaultListed {
        reference: reference.clone(),
    };
    let mut buf = BytesMut::new();
    encode_fault_parameters(&mut buf, &value).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x7E, 0x0E, // openings [7],[0]
            0x0C, 0x00, 0x00, 0x00, 0x01, // [0] object-identifier
            0x19, 0x55, // [1] property-identifier
            0x29, 0x03, // [2] property-array-index 3
            0x3C, 0x02, 0x00, 0x00, 0x08, // [3] device-identifier device,8
            0x0F, 0x7F,
        ]
    );
    let (decoded, end) = decode_fault_parameters(&buf, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, buf.len());
}

// --- Round-trips across all modeled alternatives -----------------------------

#[test]
fn fault_out_of_range_alternative_tag_forms_accepted() {
    // The inner CHOICEs discover REAL=4 / Unsigned=2 / Double=5 / INTEGER=3
    // by application tag; all four forms decode (f64-valued in the model).
    fn mk(min: &[u8], max: &[u8]) -> Vec<u8> {
        let mut v = vec![0x6E, 0x0E];
        v.extend_from_slice(min);
        v.extend_from_slice(&[0x0F, 0x1E]);
        v.extend_from_slice(max);
        v.extend_from_slice(&[0x1F, 0x6F]);
        v
    }
    let mut enc = |f: &dyn Fn(&mut BytesMut)| {
        let mut b = BytesMut::new();
        f(&mut b);
        b.to_vec()
    };
    let double10 = enc(&|b| crate::primitives::encode_app_double(b, 10.0));
    let real10 = enc(&|b| crate::primitives::encode_app_real(b, 10.0));
    let (v, _) = decode_fault_parameters(&mk(&double10, &real10), 0).unwrap();
    assert_eq!(
        v,
        FaultParameters::FaultOutOfRange {
            min_normal: 10.0,
            max_normal: 10.0
        }
    );
    let unsigned5 = enc(&|b| crate::primitives::encode_app_unsigned(b, 5));
    let signed_neg5 = enc(&|b| crate::primitives::encode_app_signed(b, -5));
    let (v, _) = decode_fault_parameters(&mk(&unsigned5, &signed_neg5), 0).unwrap();
    assert_eq!(
        v,
        FaultParameters::FaultOutOfRange {
            min_normal: 5.0,
            max_normal: -5.0
        }
    );
}

#[test]
fn fault_all_modeled_alternatives_round_trip() {
    round_trip(&FaultParameters::FaultNone);
    round_trip(&FaultParameters::FaultCharacterString {
        fault_values: vec!["a".to_string(), "b".to_string()],
    });
    round_trip(&FaultParameters::FaultExtended {
        vendor_id: 65535,
        extended_fault_type: u32::MAX,
        parameters: Vec::new(),
    });
    round_trip(&FaultParameters::FaultLifeSafety {
        fault_values: vec![0, 8],
        mode_for_reference: dopr_ai(9, 85),
    });
    round_trip(&FaultParameters::FaultState {
        fault_values: vec![BACnetPropertyStates::LifeSafetyState(2)],
    });
    round_trip(&FaultParameters::FaultOutOfRange {
        min_normal: -1.25,
        max_normal: 1.25,
    });
    round_trip(&FaultParameters::FaultStatusFlags {
        reference: dopr_ai(9, 112),
    });
    round_trip(&FaultParameters::FaultListed {
        reference: dopr_ai(9, 85),
    });
}

// --- Negatives ----------------------------------------------------------------

#[test]
fn fault_unknown_tag_rejected() {
    // [8] does not exist in fault-parameter's CHOICE (it tops out at [7]).
    let data = [0x8E, 0x8F];
    assert!(decode_fault_parameters(&data, 0).is_err());
}

#[test]
fn fault_none_with_contents_rejected() {
    // none [0] NULL must have zero contents.
    let data = [0x09, 0x00];
    assert!(decode_fault_parameters(&data, 0).is_err());
}

#[test]
fn fault_truncated_and_unbalanced_rejected() {
    // fault-out-of-range opening without any closing.
    let data = [0x6E, 0x0E, 0x55, 0x40, 0x24, 0x00, 0x00];
    assert!(decode_fault_parameters(&data, 0).is_err());
    // Truncated tag header / empty.
    assert!(decode_fault_parameters(&[0x6E], 0).is_err());
    assert!(decode_fault_parameters(&[], 0).is_err());
    // Application-tagged value where the CHOICE tag belongs.
    assert!(decode_fault_parameters(&[0x21, 0x00], 0).is_err());
}
