//! AlertEnrollmentObject tests.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;

// AlertEnrollmentObject tests
// -----------------------------------------------------------------------

#[test]
fn alert_enrollment_create() {
    let ae = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    assert_eq!(
        ae.object_identifier().object_type(),
        ObjectType::ALERT_ENROLLMENT
    );
    assert_eq!(ae.object_identifier().instance_number(), 1);
    assert_eq!(ae.object_name(), "AE-1");
}

#[test]
fn alert_enrollment_object_type() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::ALERT_ENROLLMENT.to_raw())
    );
}

#[test]
fn alert_enrollment_present_value_default() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0));
}

#[test]
fn alert_enrollment_event_detection_enable() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn alert_enrollment_write_event_detection_enable() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.event_state = EventState::OFFNORMAL.to_raw();
    ae.acked_transitions = 0;
    ae.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
    assert_eq!(
        ae.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        ae.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        }
    );
}

#[test]
fn alert_enrollment_public_detection_flag_projects_disabled_initial_state() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.event_state = EventState::OFFNORMAL.to_raw();
    ae.acked_transitions = 0;

    ae.event_detection_enable = false;

    assert_eq!(
        ae.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        ae.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        }
    );
}

#[test]
fn alert_enrollment_setter_resets_state_after_direct_disable() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.event_state = EventState::OFFNORMAL.to_raw();
    ae.acked_transitions = 0;
    ae.event_detection_enable = false;

    ae.set_event_detection_enable(true);

    assert_eq!(
        ae.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        ae.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        }
    );
}

#[test]
fn alert_enrollment_status_flags_follow_effective_event_state() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.set_event_state_internal(EventState::OFFNORMAL).unwrap();
    assert_eq!(
        ae.read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b1000_0000],
        }
    );

    ae.event_detection_enable = false;
    assert_eq!(
        ae.read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        }
    );
}

#[test]
fn alert_enrollment_disabled_state_rejects_internal_transition_updates() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.set_event_detection_enable(false);

    assert!(ae.set_event_state_internal(EventState::OFFNORMAL).is_err());
    assert!(ae.set_acked_transitions_internal(0x01, false).is_err());
    assert_eq!(
        ae.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

#[test]
fn alert_enrollment_event_enable() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();
    // Default event_enable = 0b111 -> MSB-first wire byte 0b1110_0000
    assert_eq!(
        val,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        }
    );
}

#[test]
fn alert_enrollment_write_event_enable() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1000_0000],
        },
        None,
    )
    .unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1000_0000],
        }
    );
}

#[test]
fn alert_enrollment_notification_class() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(0));
}

#[test]
fn alert_enrollment_write_notification_class() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(42),
        None,
    )
    .unwrap();
    let val = ae
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(42));
}

#[test]
fn alert_enrollment_property_list() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let props = ae.property_list();
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
    assert!(props.contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE));
    assert!(props.contains(&PropertyIdentifier::EVENT_ENABLE));
    assert!(props.contains(&PropertyIdentifier::ACKED_TRANSITIONS));
    assert!(props.contains(&PropertyIdentifier::NOTIFICATION_CLASS));
    assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
}

#[test]
fn alert_enrollment_writability_matches_write_routes() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    for property in [
        PropertyIdentifier::DESCRIPTION,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyIdentifier::NOTIFICATION_CLASS,
    ] {
        assert!(ae.is_writable_property(property));
    }
    for property in [
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::STATUS_FLAGS,
    ] {
        assert!(!ae.is_writable_property(property));
    }
}
