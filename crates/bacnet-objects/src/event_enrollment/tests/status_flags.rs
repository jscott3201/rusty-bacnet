use super::super::*;
use bacnet_types::enums::Reliability;

#[test]
fn event_enrollment_status_flags_follow_event_state_and_force_out_of_service_false() {
    let mut enrollment = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    enrollment
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    enrollment.set_event_state(EventState::HIGH_LIMIT.to_raw());

    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b1000_0000],
        }
    );

    enrollment.set_event_state(EventState::NORMAL.to_raw());
    enrollment.reliability = Reliability::NO_SENSOR.to_raw();
    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0100_0000],
        }
    );

    enrollment.reliability = Reliability::NO_FAULT_DETECTED.to_raw();
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
