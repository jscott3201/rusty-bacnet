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
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};

fn objects() -> Vec<Box<dyn BACnetObject>> {
    let source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut enrollment = EventEnrollmentObject::new(1, "EE", 5).unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference {
        object_identifier: source,
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }));
    vec![
        Box::new(AnalogInputObject::new(1, "AI", 62).unwrap()),
        Box::new(AnalogOutputObject::new(1, "AO", 62).unwrap()),
        Box::new(AnalogValueObject::new(1, "AV", 62).unwrap()),
        Box::new(BinaryInputObject::new(1, "BI").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV", 3).unwrap()),
        Box::new(enrollment),
        Box::new(AlertEnrollmentObject::new(1, "Alert", source).unwrap()),
    ]
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
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32),
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
    if object.object_identifier().object_type() == ObjectType::ALERT_ENROLLMENT {
        // Alert Enrollment retains its existing internal setter, not the
        // atomic commit/AcknowledgeAlarm capability of the other families.
        object
            .set_event_state_internal(EventState::OFFNORMAL)
            .unwrap();
        object.set_acked_transitions_internal(1, false).unwrap();
    } else {
        object
            .commit_event_transition_internal(EventTransitionCommit {
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
                if object
                    .property_list()
                    .contains(&PropertyIdentifier::OUT_OF_SERVICE)
                {
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
                    if !metadata.is_empty() {
                        let row = metadata
                            .iter()
                            .find(|row| {
                                row.property_identifier == PropertyIdentifier::ACKED_TRANSITIONS
                            })
                            .expect("migrated metadata must include Acked_Transitions");
                        assert_eq!(row.write_capability, PropertyWriteCapability::ReadOnly);
                    }
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
            object.set_acked_transitions_internal(1, true).unwrap();
            // TO_NORMAL stays acknowledged even if generic local logic clears it.
            object.set_acked_transitions_internal(4, false).unwrap();
        } else {
            let mut acknowledged = snapshot(&*object);
            acknowledged[1] = bits(0xe0);
            object
                .acknowledge_alarm_correlated_internal(
                    EventState::OFFNORMAL,
                    &BACnetTimeStamp::SequenceNumber(42),
                )
                .unwrap();
            assert_eq!(snapshot(&*object), acknowledged, "{}", object.object_name());
            // The commit kernel can also set a previously cleared coordinate
            // when Notification Class policy does not require acknowledgment.
            for ack_required in [true, false] {
                object
                    .commit_event_transition_internal(EventTransitionCommit {
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
fn acked_transitions_denial_preserves_local_rollback_tokens() {
    for mut object in objects() {
        seed_unacknowledged(&mut *object);
        let before = snapshot(&*object);
        let disabled = PropertyValue::Boolean(false);
        let rollback = object
            .capture_write_property_rollback(PropertyIdentifier::EVENT_DETECTION_ENABLE, &disabled)
            .expect("each family preserves detection-reset state for local callers");
        object
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                disabled,
                None,
            )
            .unwrap();
        assert_eq!(snapshot(&*object)[1], bits(0xe0));
        assert_denied(&mut *object, bits(0x00), None);
        // Compatibility hooks remain local-only; WPM must NOT invoke this
        // restore because its successful detection-disable prefix is committed.
        object.restore_write_property_rollback(rollback).unwrap();
        assert_eq!(snapshot(&*object), before, "{}", object.object_name());
        assert_denied(&mut *object, bits(0xe0), None);
    }
}
