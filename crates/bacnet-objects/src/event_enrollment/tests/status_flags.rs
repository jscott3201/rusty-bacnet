use super::super::*;
use bacnet_types::enums::{ErrorCode, Reliability};

#[test]
fn event_enrollment_status_flags_follow_event_state_and_force_out_of_service_false() {
    let mut enrollment =
        EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    // Table 12-14 has no Out_Of_Service (#1064), so nothing can set the flag.
    assert!(matches!(
        enrollment.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        ),
        Err(Error::Protocol { code, .. }) if code == ErrorCode::UNKNOWN_PROPERTY
    ));
    enrollment.set_event_state(EventState::HIGH_LIMIT);

    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b1000_0000],
        }
    );

    enrollment.set_event_state(EventState::NORMAL);
    enrollment.reliability = Reliability::NO_SENSOR;
    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0100_0000],
        }
    );

    enrollment.reliability = Reliability::NO_FAULT_DETECTED;
    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        }
    );
}
