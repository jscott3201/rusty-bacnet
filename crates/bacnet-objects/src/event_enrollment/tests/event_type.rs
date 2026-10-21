//! Event_Type follows Event_Parameters (Clause 12.12.5, #1868).

use super::super::*;

/// Event_Type follows Event_Parameters (Clause 12.12.5, #1868): NONE while
/// they are unset, then the type of the alternative written.
#[test]
fn read_event_type_follows_event_parameters() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::COMMAND_FAILURE).unwrap();
    let event_type = |ee: &EventEnrollmentObject| {
        ee.read_property(PropertyIdentifier::EVENT_TYPE, None)
            .unwrap()
    };
    assert_eq!(event_type(&ee), PropertyValue::Enumerated(20));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 0.0,
        high_limit: 10.0,
        deadband: 1.0,
    });
    assert_eq!(event_type(&ee), PropertyValue::Enumerated(5));
    ee.set_event_parameters(BACnetEventParameter::Extended {
        vendor_id: 1,
        extended_event_type: 2,
        parameters: Vec::new(),
    });
    assert_eq!(event_type(&ee), PropertyValue::Enumerated(9));
}

/// The legacy octet layout names no algorithm, so it runs, and serves, the
/// constructor's type, a proprietary one included.
#[test]
fn legacy_octet_parameters_serve_the_constructor_event_type() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::from_raw(600)).unwrap();
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::OctetString(vec![1, 2]),
        None,
    )
    .unwrap();
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(600)
    );
}

/// An opaque alternative at tag 64 or above names no standard event type,
/// so Event_Type isn't derived from it; tag 13 (access-event) is.
#[test]
fn opaque_tags_from_64_up_serve_the_constructor_event_type() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE).unwrap();
    for (tag, expected) in [(13, 13), (63, 63), (64, 5), (200, 5)] {
        ee.set_event_parameters(BACnetEventParameter::Opaque {
            tag,
            data: vec![0x09, 0x00],
        });
        assert_eq!(
            ee.read_property(PropertyIdentifier::EVENT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(expected),
            "tag {tag}"
        );
    }
}
