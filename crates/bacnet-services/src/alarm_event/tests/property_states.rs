use super::*;
use bacnet_types::constructed::{BACnetExtendedPropertyState, BACnetProprietaryPropertyState};

fn encode_change_of_state(state: BACnetPropertyStates) -> BytesMut {
    let mut encoded = BytesMut::new();
    NotificationParameters::ChangeOfState {
        new_state: state,
        status_flags: 0b1000,
    }
    .encode(&mut encoded)
    .unwrap();
    encoded
}

fn raw_change_of_state(tag: u8, content: &[u8]) -> BytesMut {
    let mut encoded = BytesMut::new();
    tags::encode_opening_tag(&mut encoded, 1);
    tags::encode_opening_tag(&mut encoded, 0);
    tags::encode_tag(
        &mut encoded,
        tag,
        tags::TagClass::Context,
        content.len() as u32,
    );
    encoded.extend_from_slice(content);
    tags::encode_closing_tag(&mut encoded, 0);
    primitives::encode_ctx_bit_string(&mut encoded, 1, 4, &[0x80]);
    tags::encode_closing_tag(&mut encoded, 1);
    encoded
}

#[test]
fn change_of_state_uses_clause_21_property_state_tags() {
    let variants = [
        BACnetPropertyStates::RestartReason(1),
        BACnetPropertyStates::DoorAlarmState(1),
        BACnetPropertyStates::LightingInProgress(1),
        BACnetPropertyStates::IntegerValue(-1),
        BACnetPropertyStates::TimerState(1),
        BACnetPropertyStates::LiftCarDirection(1),
        BACnetPropertyStates::AuditOperation(1),
        BACnetPropertyStates::ExtendedValue(BACnetExtendedPropertyState::new(255, 7).unwrap()),
        BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::primitive(64, vec![0xde]).unwrap(),
        ),
        BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(65, vec![0x21, 0x07]).unwrap(),
        ),
    ];

    for state in variants {
        let encoded = encode_change_of_state(state.clone());
        assert_eq!(
            NotificationParameters::decode(&encoded, 0).unwrap(),
            NotificationParameters::ChangeOfState {
                new_state: state,
                status_flags: 0b1000,
            }
        );
    }
}

#[test]
fn change_of_state_property_values_enforce_u32() {
    let unsigned_tags = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        27, 28, 30, 31, 32, 33, 34, 36, 37, 38, 39, 40, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52,
        53, 54, 55, 56, 57, 58, 59, 60, 63,
    ];

    for tag in unsigned_tags {
        assert!(
            NotificationParameters::decode(&raw_change_of_state(tag, &[1, 0, 0, 0, 0]), 0).is_err()
        );
        assert!(NotificationParameters::decode(&raw_change_of_state(tag, &[0xff; 8]), 0).is_err());

        let decoded = NotificationParameters::decode(
            &raw_change_of_state(tag, &[0, 0xff, 0xff, 0xff, 0xff]),
            0,
        )
        .unwrap();
        let NotificationParameters::ChangeOfState { new_state, .. } = decoded else {
            unreachable!();
        };
        let expected = if tag == 63 {
            Some(u32::MAX % 100_000)
        } else {
            Some(u32::MAX)
        };
        assert_eq!(new_state.as_u32(), expected, "context tag {tag}");
    }
}

#[test]
fn change_of_state_rejects_malformed_and_reserved_property_states() {
    for content in [&[][..], &[2], &[0, 0]] {
        assert!(NotificationParameters::decode(&raw_change_of_state(0, content), 0).is_err());
    }
    for tag in [26, 29, 35, 61, 62] {
        assert!(NotificationParameters::decode(&raw_change_of_state(tag, &[0]), 0).is_err());
    }

    let proprietary = NotificationParameters::decode(&raw_change_of_state(64, &[0xde]), 0).unwrap();
    assert!(matches!(
        proprietary,
        NotificationParameters::ChangeOfState {
            new_state: BACnetPropertyStates::Other(value),
            ..
        } if value.tag() == 64 && value.data() == [0xde] && !value.is_constructed()
    ));

    let proprietary = NotificationParameters::decode(
        &encode_change_of_state(BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(65, vec![0x21, 0x07]).unwrap(),
        )),
        0,
    )
    .unwrap();
    assert!(matches!(
        proprietary,
        NotificationParameters::ChangeOfState {
            new_state: BACnetPropertyStates::Other(value),
            ..
        } if value.tag() == 65 && value.data() == [0x21, 0x07] && value.is_constructed()
    ));

    let malformed = NotificationParameters::ChangeOfState {
        new_state: BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(64, vec![0xde]).unwrap(),
        ),
        status_flags: 0,
    };
    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(malformed.encode(&mut untouched).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);
}

#[test]
fn change_of_state_encoder_accounts_for_outer_nesting_atomically() {
    let accepted_depth = tags::MAX_CONTEXT_NESTING_DEPTH - 3;
    let mut accepted_body = vec![0x0e; accepted_depth];
    accepted_body.extend(vec![0x0f; accepted_depth]);
    let accepted = NotificationParameters::ChangeOfState {
        new_state: BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(64, accepted_body).unwrap(),
        ),
        status_flags: 0,
    };
    let mut encoded = BytesMut::new();
    accepted.encode(&mut encoded).unwrap();
    assert_eq!(
        NotificationParameters::decode(&encoded, 0).unwrap(),
        accepted
    );

    let body_depth = tags::MAX_CONTEXT_NESTING_DEPTH - 2;
    let mut body = vec![0x0e; body_depth];
    body.extend(vec![0x0f; body_depth]);
    let too_deep_state =
        BACnetPropertyStates::Other(BACnetProprietaryPropertyState::constructed(64, body).unwrap());
    let value = NotificationParameters::ChangeOfState {
        new_state: too_deep_state.clone(),
        status_flags: 0,
    };

    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(value.encode(&mut untouched).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);

    let mut raw = BytesMut::new();
    tags::encode_opening_tag(&mut raw, 1);
    tags::encode_opening_tag(&mut raw, 0);
    bacnet_encoding::constructed::encode_property_state(&mut raw, &too_deep_state).unwrap();
    tags::encode_closing_tag(&mut raw, 0);
    primitives::encode_ctx_bit_string(&mut raw, 1, 4, &[0]);
    tags::encode_closing_tag(&mut raw, 1);
    assert!(NotificationParameters::decode(&raw, 0).is_err());
}
