use super::super::*;
use crate::event::{EventStateChange, EventTransition};
use bacnet_types::enums::EventType;

fn monitored_reference() -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }
}

#[test]
fn configured_event_enrollment_reports_stored_type_and_shared_commit_coordinate() {
    let mut enrollment =
        EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    assert_eq!(enrollment.enrollment_summary_capability_internal(), None);

    enrollment.set_object_property_reference(Some(monitored_reference()));
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: bacnet_types::constructed::ChangeOfValueCriteria::ReferencedPropertyIncrement(
            1.0,
        ),
    });
    let capability = enrollment
        .enrollment_summary_capability_internal()
        .expect("supported configured enrollment must opt in");
    assert_eq!(capability.event_type, EventType::CHANGE_OF_VALUE);
    assert_eq!(capability.last_transition, None);

    enrollment
        .commit_event_transition_internal(EventTransitionCommit {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::OFFNORMAL,
            },
            coordinate: EventTransition::ToOffnormal,
            ack_required: false,
            timestamp: BACnetTimeStamp::SequenceNumber(7),
            message_text: None,
        })
        .unwrap();
    assert_eq!(
        enrollment
            .enrollment_summary_capability_internal()
            .unwrap()
            .last_transition,
        Some(EventTransition::ToOffnormal)
    );
}

#[test]
fn unsupported_or_unreferenced_event_enrollment_and_alert_enrollment_opt_out() {
    let mut enrollment =
        EventEnrollmentObject::new(1, "EE-1", EventType::EXTENDED.to_raw()).unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::Extended {
        vendor_id: 1,
        extended_event_type: 2,
        parameters: Vec::new(),
    });
    assert_eq!(enrollment.enrollment_summary_capability_internal(), None);

    enrollment.set_object_property_reference(Some(monitored_reference()));
    assert_eq!(enrollment.enrollment_summary_capability_internal(), None);

    let alert = AlertEnrollmentObject::new(
        1,
        "AE-1",
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
    )
    .unwrap();
    assert_eq!(alert.enrollment_summary_capability_internal(), None);
}

#[test]
fn detection_enable_is_separate_from_configured_capability() {
    let mut enrollment =
        EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    enrollment.set_object_property_reference(Some(monitored_reference()));
    enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 0.0,
        high_limit: 10.0,
        deadband: 1.0,
    });
    enrollment
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

    let capability = enrollment
        .enrollment_summary_capability_internal()
        .expect("handler, not capability, applies detection-enable exclusion");
    assert_eq!(capability.event_type, EventType::OUT_OF_RANGE);
    assert_eq!(capability.last_transition, None);
}
