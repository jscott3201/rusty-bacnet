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
    let mut enrollment = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_VALUE).unwrap();
    assert_eq!(enrollment.enrollment_summary_capability_internal(), None);

    enrollment
        .set_object_property_reference(Some(monitored_reference()))
        .unwrap();
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

/// An enrollment can fault once it monitors something, whatever its
/// parameters (#1868): an unmodeled Extended one opts in with a reference
/// and stays out without one. Alert Enrollment never opts in.
#[test]
fn unreferenced_event_enrollment_and_alert_enrollment_opt_out() {
    let mut enrollment = EventEnrollmentObject::new(1, "EE-1", EventType::EXTENDED).unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::Extended {
        vendor_id: 1,
        extended_event_type: 2,
        parameters: Vec::new(),
    });
    assert_eq!(enrollment.enrollment_summary_capability_internal(), None);

    enrollment
        .set_object_property_reference(Some(monitored_reference()))
        .unwrap();
    assert_eq!(
        enrollment
            .enrollment_summary_capability_internal()
            .unwrap()
            .event_type,
        EventType::EXTENDED
    );

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
    let mut enrollment = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE).unwrap();
    enrollment
        .set_object_property_reference(Some(monitored_reference()))
        .unwrap();
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

/// An enrollment whose Event_Parameters were never set serves the `none`
/// alternative of the CHOICE: context tag 20 with a NULL body, which takes
/// the extended tag-number form `F8 14`. It evaluates reliability only
/// (Clause 12.12.7), so with a monitored reference it is listed with
/// Event_Type NONE (#1868).
#[test]
fn unset_event_parameters_serve_the_none_alternative() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(monitored_reference()))
        .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(vec![0xF8, 0x14]));
    let PropertyValue::ApplicationData(bytes) = &val else {
        unreachable!()
    };
    assert_eq!(
        bacnet_encoding::constructed::decode_event_parameter(bytes, 0).unwrap(),
        (BACnetEventParameter::None, 2)
    );
    assert_eq!(
        ee.enrollment_summary_capability_internal()
            .unwrap()
            .event_type,
        EventType::NONE
    );
    // Without a reference it can't fault, so it stays out.
    ee.set_object_property_reference(None).unwrap();
    assert_eq!(ee.enrollment_summary_capability_internal(), None);

    // The served value writes back as itself.
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        val.clone(),
        None,
    )
    .unwrap();
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
            .unwrap(),
        val
    );
}
