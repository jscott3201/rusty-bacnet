//! Acked_Transitions is read-only in the property descriptions accompanying
//! Tables 12-2/3/4, 12-6/8/10, 12-21/22/23, 12-14, and 12-61 (135-2020).

use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::event::{EventStateChange, EventTransition, EventTransitionCommit};
use crate::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::property_metadata::PropertyWriteCapability;
use crate::traits::BACnetObject;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, EventState, EventType, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};

fn objects() -> Vec<Box<dyn BACnetObject>> {
    let source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut enrollment = EventEnrollmentObject::new(1, "EE", EventType::OUT_OF_RANGE).unwrap();
    enrollment
        .set_object_property_reference(Some(BACnetDeviceObjectPropertyReference {
            object_identifier: source,
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        }))
        .unwrap();
    // An unset enrollment serves `none` and generates no events (#1868).
    enrollment.set_event_parameters(
        bacnet_types::constructed::BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 0.0,
            high_limit: 100.0,
            deadband: 1.0,
        },
    );
    vec![
        Box::new(AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(AnalogOutputObject::new(1, "AO", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(AnalogValueObject::new(1, "AV", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(BinaryInputObject::new(1, "BI").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV", 3).unwrap()),
        Box::new(enrollment),
        Box::new(unacknowledged_alert(source)),
    ]
}

/// Alert Enrollment has no transition commit, so its unacknowledged
/// TO_OFFNORMAL is seeded in its fields.
fn unacknowledged_alert(source: ObjectIdentifier) -> AlertEnrollmentObject {
    let mut alert = AlertEnrollmentObject::new(1, "Alert", source).unwrap();
    alert.event_state = EventState::OFFNORMAL;
    alert
        .acked_transitions
        .remove(EventTransition::ToOffnormal.bit_mask());
    alert
}

fn bits(wire_octet: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![wire_octet],
    }
}

fn snapshot(object: &dyn BACnetObject) -> [PropertyValue; 4] {
    [
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
    ]
    .map(|property| object.read_property(property, None).unwrap())
}

fn assert_denied(object: &mut dyn BACnetObject, value: PropertyValue, priority: Option<u8>) {
    let before = snapshot(object);
    let error = object
        .write_property(PropertyIdentifier::ACKED_TRANSITIONS, None, value, priority)
        .unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED),
        "{}: {error:?}",
        object.object_name()
    );
    assert_eq!(snapshot(object), before, "{}", object.object_name());
}

fn seed_unacknowledged(object: &mut dyn BACnetObject) {
    object
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    // Alert Enrollment comes seeded: it has no atomic commit or
    // AcknowledgeAlarm capability like the other families.
    if object.object_identifier().object_type() != ObjectType::ALERT_ENROLLMENT {
        object
            .event_reporting_mut()
            .unwrap()
            .commit_event_transition(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::OFFNORMAL,
                },
                coordinate: EventTransition::ToOffnormal,
                ack_required: true,
                timestamp: BACnetTimeStamp::SequenceNumber(42),
                message_text: None,
            })
            .unwrap();
    }
    assert_eq!(snapshot(object)[1], bits(0x60), "{}", object.object_name());
}

#[test]
fn acked_transitions_network_write_and_metadata_deny_every_family() {
    for mut object in objects() {
        seed_unacknowledged(&mut *object);
        for enabled in [true, false] {
            object
                .write_property(
                    PropertyIdentifier::EVENT_DETECTION_ENABLE,
                    None,
                    PropertyValue::Boolean(enabled),
                    None,
                )
                .unwrap();
            for out_of_service in [false, true] {
                if crate::property_metadata::has_property(
                    &object.property_metadata(),
                    PropertyIdentifier::OUT_OF_SERVICE,
                ) {
                    object
                        .write_property(
                            PropertyIdentifier::OUT_OF_SERVICE,
                            None,
                            PropertyValue::Boolean(out_of_service),
                            None,
                        )
                        .unwrap();
                }
                assert!(!object.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS));
                {
                    let metadata = object.property_metadata();
                    let row = metadata
                        .iter()
                        .find(|row| {
                            row.property_identifier == PropertyIdentifier::ACKED_TRANSITIONS
                        })
                        .expect("the metadata must include Acked_Transitions");
                    assert_eq!(row.write_capability, PropertyWriteCapability::ReadOnly);
                }
                for priority in [None, Some(1), Some(16)] {
                    // Fabricating, erasing, and partially replacing acknowledgments
                    // must all fail, including while detection is disabled or OOS.
                    for value in [bits(0xe0), bits(0x00), bits(0xa0)] {
                        assert_denied(&mut *object, value, priority);
                    }
                }
            }
        }
    }
}

#[test]
fn acked_transitions_internal_commit_and_acknowledgment_bypass_property_writes() {
    for mut object in objects() {
        seed_unacknowledged(&mut *object);
        assert_denied(&mut *object, bits(0xe0), None);
        if object.object_identifier().object_type() == ObjectType::ALERT_ENROLLMENT {
            // No local route acknowledges an Alert Enrollment's transition.
            assert_eq!(snapshot(&*object)[1], bits(0x60));
            continue;
        }
        {
            let mut acknowledged = snapshot(&*object);
            acknowledged[1] = bits(0xe0);
            object
                .event_reporting_mut()
                .unwrap()
                .acknowledge_alarm(EventState::OFFNORMAL, &BACnetTimeStamp::SequenceNumber(42))
                .unwrap();
            assert_eq!(snapshot(&*object), acknowledged, "{}", object.object_name());
            // The commit kernel can also set a previously cleared coordinate
            // when Notification Class policy does not require acknowledgment.
            for ack_required in [true, false] {
                object
                    .event_reporting_mut()
                    .unwrap()
                    .commit_event_transition(EventTransitionCommit {
                        change: EventStateChange {
                            from: EventState::OFFNORMAL,
                            to: EventState::OFFNORMAL,
                        },
                        coordinate: EventTransition::ToOffnormal,
                        ack_required,
                        timestamp: BACnetTimeStamp::SequenceNumber(43),
                        message_text: None,
                    })
                    .unwrap();
                assert_eq!(
                    snapshot(&*object)[1],
                    bits(if ack_required { 0x60 } else { 0xe0 })
                );
            }
        }
        assert_eq!(
            snapshot(&*object)[1],
            bits(0xe0),
            "{}",
            object.object_name()
        );
        assert!(!object.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS));
    }
}

#[test]
fn acked_transitions_denial_preserves_state_before_and_after_detection_reset() {
    for mut object in objects() {
        seed_unacknowledged(&mut *object);
        let before = snapshot(&*object);
        assert_denied(&mut *object, bits(0x00), None);
        assert_eq!(snapshot(&*object), before, "{}", object.object_name());
        object
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Boolean(false),
                None,
            )
            .unwrap();
        let reset = snapshot(&*object);
        assert_eq!(reset[1], bits(0xe0));
        assert_denied(&mut *object, bits(0x00), None);
        assert_eq!(snapshot(&*object), reset, "{}", object.object_name());
    }
}
