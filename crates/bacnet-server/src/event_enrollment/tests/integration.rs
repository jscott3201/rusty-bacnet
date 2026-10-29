//! Multiple-enrollment integration tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::EngineeringUnits;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ---- Integration: multiple enrollments ----

#[test]
fn evaluates_multiple_enrollments() {
    let mut db = ObjectDatabase::new();

    // Two analog inputs
    let mut ai1 = AnalogInputObject::new(80, "AI-80", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai1.set_present_value(90.0); // will trigger HIGH_LIMIT
    let ai1_oid = ai1.object_identifier();
    db.add(Box::new(ai1)).unwrap();

    let mut ai2 = AnalogInputObject::new(81, "AI-81", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai2.set_present_value(50.0); // normal
    let ai2_oid = ai2.object_identifier();
    db.add(Box::new(ai2)).unwrap();

    // Two enrollments
    let mut ee1 = EventEnrollmentObject::new(80, "EE-80", EventType::OUT_OF_RANGE).unwrap();
    ee1.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai1_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    ee1.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee1.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee1)).unwrap();

    let mut ee2 = EventEnrollmentObject::new(81, "EE-81", EventType::OUT_OF_RANGE).unwrap();
    ee2.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai2_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    ee2.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee2.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee2)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    // Only AI-80 triggers (90 > 80)
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].monitored_oid, ai1_oid);
}

#[test]
fn missing_monitored_object_is_skipped() {
    let mut db = ObjectDatabase::new();

    let fake_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();
    let mut ee = EventEnrollmentObject::new(90, "EE-miss", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        fake_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee)).unwrap();

    // Should not panic or return transitions
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}

pub(super) fn setup_qualified_reference(
    local_device_instances: &[u32],
    reference_device_instance: u32,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    for instance in local_device_instances {
        let device = DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(*instance, 0)
                .expect("valid Device identity")
                .with_name(format!("Device-{instance}")),
            ..DeviceConfig::default()
        })
        .unwrap();
        db.add(Box::new(device)).unwrap();
    }

    let mut ai = AnalogInputObject::new(3, "AI-3", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(90.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let reference_device_oid =
        ObjectIdentifier::new(ObjectType::DEVICE, reference_device_instance).unwrap();
    let mut ee = EventEnrollmentObject::new(3, "EE-3", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_remote(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
        reference_device_oid,
    )))
    .unwrap();
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(EventTransitionBits::all());
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, ai_oid)
}

#[test]
fn self_qualified_reference_evaluates_local_object() {
    let (mut db, _ee_oid, ai_oid) = setup_qualified_reference(&[100], 100);

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].monitored_oid, ai_oid);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}

#[test]
fn foreign_reference_does_not_evaluate_same_numbered_local_object() {
    let (mut db, ee_oid, _ai_oid) = setup_qualified_reference(&[100], 200);

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    let reference = super::super::decode_reference_value(
        &db.get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
            .unwrap(),
    )
    .expect("one framed reference");
    assert_eq!(
        reference.device_identifier,
        Some(ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap())
    );
}

#[test]
fn qualified_reference_names_the_selected_device() {
    let (mut missing, _, _) = setup_qualified_reference(&[], 100);
    assert!(evaluate_event_enrollments(&mut missing, 1).is_empty());

    // With two Devices the lower one is this device, as discovery and
    // notifications already present it (#1184).
    let (mut selected, _, ai_oid) = setup_qualified_reference(&[200, 100], 100);
    let transitions = evaluate_event_enrollments(&mut selected, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].monitored_oid, ai_oid);
    let (mut other, _, _) = setup_qualified_reference(&[100, 200], 200);
    assert!(evaluate_event_enrollments(&mut other, 1).is_empty());

    let (mut wildcard, _, _) = setup_qualified_reference(
        &[ObjectIdentifier::WILDCARD_INSTANCE],
        ObjectIdentifier::WILDCARD_INSTANCE,
    );
    assert!(evaluate_event_enrollments(&mut wildcard, 1).is_empty());
}

pub(super) struct ReferenceValueObject {
    pub(super) inner: EventEnrollmentObject,
    reference: Option<PropertyValue>,
    pub(super) event_parameters_readable: Arc<AtomicBool>,
    pub(super) fault_parameters_supported: bool,
    pub(super) state_writable: Arc<AtomicBool>,
    pub(super) state_write_count: Arc<AtomicUsize>,
    pub(super) source_supported: bool,
    pub(super) source_writable: Arc<AtomicBool>,
    pub(super) normal_event_state_writable: bool,
    pub(super) event_state_error_after_write: bool,
    pub(super) atomic_commit_supported: bool,
    pub(super) reliability_commit_supported: bool,
    pub(super) reliability_error_after_write: bool,
}

impl ReferenceValueObject {
    pub(super) fn new(reference: Option<PropertyValue>) -> Self {
        Self::new_for_event_type(reference, EventType::OUT_OF_RANGE)
    }

    pub(super) fn new_for_event_type(
        reference: Option<PropertyValue>,
        event_type: EventType,
    ) -> Self {
        let mut inner = EventEnrollmentObject::new(999, "reference-value", event_type).unwrap();
        inner.set_event_parameters(BACnetEventParameter::OutOfRange {
            time_delay: 2,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        });
        inner.set_event_enable(EventTransitionBits::all());
        Self {
            inner,
            reference,
            event_parameters_readable: Arc::new(AtomicBool::new(true)),
            fault_parameters_supported: true,
            state_writable: Arc::new(AtomicBool::new(true)),
            state_write_count: Arc::new(AtomicUsize::new(0)),
            source_supported: true,
            source_writable: Arc::new(AtomicBool::new(true)),
            normal_event_state_writable: true,
            event_state_error_after_write: false,
            atomic_commit_supported: true,
            reliability_commit_supported: true,
            reliability_error_after_write: false,
        }
    }
}

impl BACnetObject for ReferenceValueObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.inner.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, bacnet_types::error::Error> {
        if property == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE {
            self.reference
                .clone()
                .ok_or_else(|| bacnet_types::error::Error::Encoding("reference read failed".into()))
        } else if property == PropertyIdentifier::FAULT_PARAMETERS
            && !self.fault_parameters_supported
        {
            Err(bacnet_types::error::Error::protocol(
                bacnet_types::enums::ErrorClass::PROPERTY,
                bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY,
            ))
        } else if property == PropertyIdentifier::EVENT_PARAMETERS
            && !self.event_parameters_readable.load(Ordering::SeqCst)
        {
            Err(bacnet_types::error::Error::Encoding(
                "event parameters read failed".into(),
            ))
        } else {
            self.inner.read_property(property, array_index)
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), bacnet_types::error::Error> {
        if property == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE {
            self.reference = Some(value);
            Ok(())
        } else {
            self.inner.write_property_from(
                property,
                array_index,
                value,
                priority,
                &crate::command_source::test_origin(),
            )
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }

    fn enrollment_eval_state_internal(
        &self,
    ) -> Option<bacnet_objects::event_enrollment::EventEnrollmentEvalState> {
        self.inner.enrollment_eval_state_internal()
    }

    fn set_enrollment_eval_state_internal(
        &mut self,
        state: bacnet_objects::event_enrollment::EventEnrollmentEvalState,
    ) -> Result<(), bacnet_types::error::Error> {
        self.state_write_count.fetch_add(1, Ordering::SeqCst);
        if !self.state_writable.load(Ordering::SeqCst) {
            return Err(bacnet_types::error::Error::Encoding(
                "evaluation state write failed".into(),
            ));
        }
        self.inner.set_enrollment_eval_state_internal(state)
    }

    fn enrollment_eval_source_internal(
        &self,
    ) -> Option<Option<bacnet_objects::event_enrollment::EventEnrollmentMonitoredSource>> {
        self.source_supported
            .then(|| self.inner.enrollment_eval_source_internal().flatten())
    }

    fn set_enrollment_eval_source_internal(
        &mut self,
        source: Option<bacnet_objects::event_enrollment::EventEnrollmentMonitoredSource>,
    ) -> Result<(), bacnet_types::error::Error> {
        if !self.source_writable.load(Ordering::SeqCst) {
            return Err(bacnet_types::error::Error::Encoding(
                "source write failed".into(),
            ));
        }
        self.inner.set_enrollment_eval_source_internal(source)
    }

    fn set_event_state_internal(
        &mut self,
        state: EventState,
    ) -> Result<(), bacnet_types::error::Error> {
        if state == EventState::NORMAL && !self.normal_event_state_writable {
            return Err(bacnet_types::error::Error::Encoding(
                "event state write failed".into(),
            ));
        }
        self.inner.set_event_state_internal(state)?;
        if self.event_state_error_after_write {
            Err(bacnet_types::error::Error::Encoding(
                "event state write reported failure".into(),
            ))
        } else {
            Ok(())
        }
    }

    fn set_acked_transitions_internal(
        &mut self,
        transition_bit: EventTransitionBits,
        acknowledged: bool,
    ) -> Result<(), bacnet_types::error::Error> {
        self.inner
            .set_acked_transitions_internal(transition_bit, acknowledged)
    }

    fn commit_event_transition_internal(
        &mut self,
        commit: bacnet_objects::event::EventTransitionCommit,
    ) -> Result<(), bacnet_objects::event::EventTransitionCommitError> {
        if !self.atomic_commit_supported {
            return Err(bacnet_objects::event::EventTransitionCommitError::Unsupported);
        }
        self.inner.commit_event_transition_internal(commit)?;
        if self.event_state_error_after_write {
            Err(bacnet_objects::event::EventTransitionCommitError::Unsupported)
        } else {
            Ok(())
        }
    }

    fn commit_event_enrollment_reliability_internal(
        &mut self,
        commit: bacnet_objects::event_enrollment::EventEnrollmentReliabilityCommit,
    ) -> Result<(), bacnet_objects::event::EventTransitionCommitError> {
        if !self.reliability_commit_supported {
            return Err(bacnet_objects::event::EventTransitionCommitError::Unsupported);
        }
        self.inner
            .commit_event_enrollment_reliability_internal(commit)?;
        if self.reliability_error_after_write {
            Err(bacnet_objects::event::EventTransitionCommitError::Unsupported)
        } else {
            Ok(())
        }
    }
}

pub(super) fn indexed_reference_value(target: ObjectIdentifier, index: u32) -> PropertyValue {
    reference_value(
        target,
        PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
        Some(index),
        None,
    )
}

/// A reference as an Event Enrollment read serves it: the Clause 21
/// encoding of a BACnetDeviceObjectPropertyReference (#1182).
pub(super) fn reference_value(
    target: ObjectIdentifier,
    property: u32,
    index: Option<u32>,
    device: Option<ObjectIdentifier>,
) -> PropertyValue {
    let mut encoded = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_device_object_property_reference(
        &mut encoded,
        &BACnetDeviceObjectPropertyReference {
            object_identifier: target,
            property_identifier: property,
            property_array_index: index,
            device_identifier: device,
        },
    );
    PropertyValue::ApplicationData(encoded.to_vec())
}

#[test]
fn failed_source_write_does_not_persist_dependent_state() {
    let mut db = ObjectDatabase::new();
    let mut target =
        AnalogValueObject::new(99, "AV-failed-source", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    target
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    enrollment.source_writable.store(false, Ordering::SeqCst);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    for _ in 0..4 {
        assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
        assert_eq!(
            db.get(&enrollment_oid)
                .unwrap()
                .enrollment_eval_state_internal(),
            Some(bacnet_objects::event_enrollment::EventEnrollmentEvalState::default())
        );
    }
}

#[test]
fn source_write_failure_still_allows_an_immediate_stateless_transition() {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(
        100,
        "AV-immediate-source",
        EngineeringUnits::DEGREES_CELSIUS,
    )
    .unwrap();
    target
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut enrollment = ReferenceValueObject::new(Some(indexed_reference_value(target_oid, 1)));
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::OutOfRange {
            time_delay: 0,
            low_limit: 20.0,
            high_limit: 80.0,
            deadband: 2.0,
        });
    enrollment.source_writable.store(false, Ordering::SeqCst);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
}

#[test]
fn failed_state_reset_clears_source_ownership() {
    let mut db = ObjectDatabase::new();
    let mut target =
        AnalogValueObject::new(101, "AV-state-reset", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    target
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(10.0),
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    target
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(2),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let old_source = (target_oid, PropertyIdentifier::PRIORITY_ARRAY, Some(1));
    let mut enrollment = ReferenceValueObject::new_for_event_type(
        Some(indexed_reference_value(target_oid, 2)),
        EventType::CHANGE_OF_VALUE,
    );
    enrollment
        .inner
        .set_event_parameters(BACnetEventParameter::ChangeOfValue {
            time_delay: 0,
            criteria: bacnet_types::constructed::ChangeOfValueCriteria::ReferencedPropertyIncrement(
                5.0,
            ),
        });
    enrollment
        .inner
        .set_enrollment_eval_state_internal(
            bacnet_objects::event_enrollment::EventEnrollmentEvalState {
                pending: None,
                cov_baseline: Some(PropertyValue::Real(10.0)),
                last_offnormal_value: None,
            },
        )
        .unwrap();
    enrollment
        .inner
        .set_enrollment_eval_source_internal(Some(old_source))
        .unwrap();
    enrollment.state_writable.store(false, Ordering::SeqCst);
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_source_internal(),
        Some(None)
    );
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
}

#[test]
fn malformed_reference_shapes_do_not_become_local() {
    let target = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let foreign_device = ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap();
    let property = PropertyIdentifier::PRESENT_VALUE;
    let PropertyValue::ApplicationData(good) =
        reference_value(target, property.to_raw(), None, Some(foreign_device))
    else {
        unreachable!()
    };
    let malformed = [
        // The flat application-tagged form reads used to serve.
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Unsigned(property.to_raw() as u64),
            PropertyValue::ObjectIdentifier(foreign_device),
        ]),
        PropertyValue::Null,
        // The unset form an enrollment without a reference reads as (#1417):
        // analog-input 4194303, and a reference naming Device 4194303.
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55]),
        PropertyValue::ApplicationData([&good[..7], &[0x3C, 0x02, 0x3F, 0xFF, 0xFF][..]].concat()),
        // Cut inside the Device member.
        PropertyValue::ApplicationData(good[..good.len() - 1].to_vec()),
        // A context tag [4] after the reference.
        PropertyValue::ApplicationData([good.clone(), vec![0x49, 0x01]].concat()),
        // Two references.
        PropertyValue::ApplicationData([good.clone(), good.clone()].concat()),
        // A property identifier past u32.
        PropertyValue::ApplicationData(vec![
            0x0C, 0x00, 0x00, 0x00, 0x03, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x55,
        ]),
        // An array index past u32.
        PropertyValue::ApplicationData(vec![
            0x0C, 0x00, 0x00, 0x00, 0x03, 0x19, 0x55, 0x2D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00,
        ]),
    ];

    for value in malformed {
        let enrollment = ReferenceValueObject::new(Some(value));
        assert!(matches!(
            super::super::read_object_property_ref(&enrollment),
            Err(super::super::LocalConfigurationReadError::Malformed)
        ));
    }

    // ASHRAE assigns property identifiers above 4194303 too, such as
    // Default_Color_Temperature (#887).
    for property in [property, PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE] {
        let framed =
            ReferenceValueObject::new(Some(reference_value(target, property.to_raw(), None, None)));
        assert_eq!(
            super::super::read_object_property_ref(&framed),
            Ok(super::super::MonitoredReference::local(
                target, property, None
            ))
        );
    }
}

fn stale_eval_state() -> bacnet_objects::event_enrollment::EventEnrollmentEvalState {
    bacnet_objects::event_enrollment::EventEnrollmentEvalState {
        pending: Some(bacnet_objects::event_enrollment::EventEnrollmentPending {
            state: EventState::HIGH_LIMIT,
            remaining: 1,
            condition: 0,
            params_fingerprint: 1,
        }),
        cov_baseline: Some(PropertyValue::Real(90.0)),
        last_offnormal_value: Some(1),
    }
}

#[test]
fn malformed_retarget_does_not_resume_stale_countdown() {
    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(3, "AI-3", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(90.0);
    let target = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let property = PropertyIdentifier::PRESENT_VALUE;
    let valid = reference_value(target, property.to_raw(), None, None);
    let enrollment = ReferenceValueObject::new(Some(valid.clone()));
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    let malformed = PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(target),
        PropertyValue::Unsigned(property.to_raw() as u64),
        PropertyValue::ObjectIdentifier(ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap()),
    ]);
    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            malformed,
            None,
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap(),
        bacnet_objects::event_enrollment::EventEnrollmentEvalState::default()
    );

    db.get_mut(&enrollment_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            None,
            valid,
            None,
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        evaluate_event_enrollments(&mut db, 1)[0].change.to,
        EventState::HIGH_LIMIT
    );
}

#[test]
fn invalid_reference_clears_before_other_property_failure() {
    let target = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let PropertyValue::ApplicationData(good) = reference_value(
        target,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
        None,
        None,
    ) else {
        unreachable!()
    };
    // The reference cut inside its property identifier.
    let mut enrollment = ReferenceValueObject::new(Some(PropertyValue::ApplicationData(
        good[..good.len() - 1].to_vec(),
    )));
    enrollment
        .event_parameters_readable
        .store(false, Ordering::SeqCst);
    enrollment
        .set_enrollment_eval_state_internal(stale_eval_state())
        .unwrap();
    let enrollment_oid = enrollment.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap(),
        bacnet_objects::event_enrollment::EventEnrollmentEvalState::default()
    );
}
