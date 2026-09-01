//! AlertEnrollmentObject tests.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode, NotifyType};

fn alert_source(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn assert_property_error<T: std::fmt::Debug>(
    result: Result<T, Error>,
    expected: ErrorCode,
    context: &str,
) {
    match result.expect_err(context) {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32, "{context}");
            assert_eq!(code, expected.to_raw() as u32, "{context}");
        }
        other => panic!("{context}: expected property error, got {other:?}"),
    }
}

// AlertEnrollmentObject tests
// -----------------------------------------------------------------------

#[test]
fn alert_enrollment_create() {
    let source = alert_source(7);
    let ae = AlertEnrollmentObject::new(1, "AE-1", source).unwrap();
    assert_eq!(
        ae.object_identifier().object_type(),
        ObjectType::ALERT_ENROLLMENT
    );
    assert_eq!(ae.object_identifier().instance_number(), 1);
    assert_eq!(ae.object_name(), "AE-1");
    assert_eq!(ae.present_value, source);
}

#[test]
fn alert_enrollment_object_type() {
    let ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
    let val = ae
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::ALERT_ENROLLMENT.to_raw())
    );
}

#[test]
fn alert_enrollment_present_value_is_the_explicit_initial_source_and_is_read_only() {
    let source = alert_source(9);
    let mut ae = AlertEnrollmentObject::new(1, "AE", source).unwrap();
    let val = ae
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::ObjectIdentifier(source));

    assert_property_error(
        ae.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::ObjectIdentifier(alert_source(10)),
            None,
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
        "Present_Value network write",
    );
    assert_eq!(ae.present_value, source);
}

#[test]
fn record_alert_source_changes_only_present_value() {
    let mut ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
    ae.set_event_state_internal(EventState::OFFNORMAL).unwrap();
    ae.set_acked_transitions_internal(0x01, false).unwrap();
    let observed_properties = [
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyIdentifier::EVENT_TIME_STAMPS,
    ];
    let before: Vec<_> = observed_properties
        .iter()
        .map(|property| ae.read_property(*property, None).unwrap())
        .collect();

    let new_source = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 22).unwrap();
    ae.record_alert_source(new_source);

    assert_eq!(ae.present_value, new_source);
    assert_eq!(
        ae.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::ObjectIdentifier(new_source)
    );
    let after: Vec<_> = observed_properties
        .iter()
        .map(|property| ae.read_property(*property, None).unwrap())
        .collect();
    assert_eq!(
        after, before,
        "source recording must have no event side effects"
    );
}

#[test]
fn alert_enrollment_event_detection_enable() {
    let ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn alert_enrollment_write_event_detection_enable() {
    let mut ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
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
    let mut ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
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
    let mut ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
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
fn alert_enrollment_removed_common_properties_are_unknown_and_nonmutating() {
    let source = alert_source(1);
    let mut ae = AlertEnrollmentObject::new(1, "AE", source).unwrap();
    for property in [
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::RELIABILITY,
    ] {
        assert_property_error(
            ae.read_property(property, None),
            ErrorCode::UNKNOWN_PROPERTY,
            &format!("{property:?} read"),
        );
        assert_property_error(
            ae.write_property(property, None, PropertyValue::Boolean(true), None),
            ErrorCode::WRITE_ACCESS_DENIED,
            &format!("{property:?} write"),
        );
    }
    assert_eq!(ae.present_value, source);
}

#[test]
fn alert_enrollment_disabled_state_rejects_internal_transition_updates() {
    let mut ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
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
    let ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
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
    let mut ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
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
    let ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
    let val = ae
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(0));
}

#[test]
fn alert_enrollment_write_notification_class() {
    let mut ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
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
    let ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
    assert_eq!(
        ae.property_list().as_ref(),
        [
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_TIME_STAMPS,
        ]
    );
    assert_eq!(
        ae.read_property(PropertyIdentifier::PROPERTY_LIST, None)
            .unwrap(),
        PropertyValue::List(
            [
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::EVENT_STATE,
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                PropertyIdentifier::NOTIFICATION_CLASS,
                PropertyIdentifier::EVENT_ENABLE,
                PropertyIdentifier::ACKED_TRANSITIONS,
                PropertyIdentifier::NOTIFY_TYPE,
                PropertyIdentifier::EVENT_TIME_STAMPS,
            ]
            .into_iter()
            .map(|property| PropertyValue::Enumerated(property.to_raw()))
            .collect(),
        )
    );
}

#[test]
fn alert_enrollment_notify_type_defaults_and_accepts_only_alarm_or_event() {
    let mut ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
    assert_eq!(
        ae.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw())
    );

    for accepted in [NotifyType::EVENT, NotifyType::ALARM] {
        ae.write_property(
            PropertyIdentifier::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(accepted.to_raw()),
            None,
        )
        .unwrap();
        assert_eq!(
            ae.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(accepted.to_raw())
        );
    }

    for refused in [NotifyType::ACK_NOTIFICATION.to_raw(), 3, 99, u32::MAX] {
        assert_property_error(
            ae.write_property(
                PropertyIdentifier::NOTIFY_TYPE,
                None,
                PropertyValue::Enumerated(refused),
                None,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
            &format!("Notify_Type={refused}"),
        );
    }
    assert_property_error(
        ae.write_property(
            PropertyIdentifier::NOTIFY_TYPE,
            None,
            PropertyValue::Unsigned(1),
            None,
        ),
        ErrorCode::INVALID_DATA_TYPE,
        "Notify_Type wrong datatype",
    );
    assert_eq!(
        ae.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        "every refused write must preserve the prior value"
    );
}

#[test]
fn alert_enrollment_writability_matches_write_routes() {
    let ae = AlertEnrollmentObject::new(1, "AE", alert_source(1)).unwrap();
    for property in [
        PropertyIdentifier::DESCRIPTION,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyIdentifier::NOTIFY_TYPE,
    ] {
        assert!(ae.is_writable_property(property));
    }
    for property in [
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::RELIABILITY,
    ] {
        assert!(!ae.is_writable_property(property));
    }
}
