//! Fault detection / reliability evaluation.
//!
//! The [`FaultDetector`] periodically evaluates each object's reliability,
//! checking for OVER_RANGE, UNDER_RANGE (analog objects), and optionally
//! COMMUNICATION_FAILURE (staleness timeout).

use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::collections::HashMap;
use std::sync::Mutex;

/// A reliability change detected by the fault detector.
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityChange {
    /// The object whose reliability changed.
    pub object_id: ObjectIdentifier,
    /// Previous reliability value (raw u32).
    pub old_reliability: u32,
    /// New reliability value (raw u32).
    pub new_reliability: u32,
}

/// Fault detection engine.
///
/// Call [`FaultDetector::evaluate`] periodically (e.g. every 10 s) against
/// the object database.  It returns a list of objects whose reliability
/// changed so the caller can update them.
pub struct FaultDetector {
    /// Timeout after which an object is considered to have a communication
    /// failure.  Set to `None` to disable communication-failure detection.
    pub comm_timeout: Option<std::time::Duration>,
    warned_internal_failures: Mutex<HashMap<ObjectIdentifier, String>>,
}

impl Default for FaultDetector {
    fn default() -> Self {
        Self {
            comm_timeout: Some(std::time::Duration::from_secs(60)),
            warned_internal_failures: Mutex::new(HashMap::new()),
        }
    }
}

impl FaultDetector {
    /// Create a new fault detector with the given communication timeout.
    pub fn new(comm_timeout: Option<std::time::Duration>) -> Self {
        Self {
            comm_timeout,
            warned_internal_failures: Mutex::new(HashMap::new()),
        }
    }

    fn should_warn_internal_failure(&self, oid: ObjectIdentifier, error: &str) -> bool {
        let mut warned = self
            .warned_internal_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if warned.get(&oid).is_some_and(|previous| previous == error) {
            false
        } else {
            warned.insert(oid, error.to_owned());
            true
        }
    }

    fn clear_internal_failure(&self, oid: &ObjectIdentifier) {
        self.warned_internal_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(oid);
    }

    /// Evaluate reliability for all objects in the database.
    ///
    /// For each analog object (AI, AO, AV) that has `MIN_PRES_VALUE` and
    /// `MAX_PRES_VALUE` properties, the present value is compared against
    /// those limits.  If out of range the reliability is set to
    /// `OVER_RANGE` or `UNDER_RANGE`; otherwise `NO_FAULT_DETECTED`.
    ///
    /// **Objects with `Out_Of_Service` TRUE are skipped entirely** — not compared,
    /// not written, and not represented in the returned list. While an object is
    /// out of service the client owns `Reliability`: ASHRAE 135-2020 Clause 12.2(c)
    /// and 12.3(c) make it writable "to allow simulating specific conditions or for
    /// testing purposes" (Clause 12.4(b) for Analog Value), and 12.2(b) / 12.3(b)
    /// decouple it from the physical point. Re-deriving it here would overwrite a
    /// simulated value within one evaluation interval.
    ///
    /// The check is fail-open: an object that does not report `Out_Of_Service`, or
    /// reports it at an unexpected type, is still evaluated.
    ///
    /// Returns a list of changes that were applied to the database.
    pub fn evaluate(&self, db: &mut ObjectDatabase) -> Vec<ReliabilityChange> {
        let analog_types = [
            ObjectType::ANALOG_INPUT,
            ObjectType::ANALOG_OUTPUT,
            ObjectType::ANALOG_VALUE,
        ];

        let mut updates: Vec<(ObjectIdentifier, u32, u32)> = Vec::new();

        for &obj_type in &analog_types {
            let oids = db.find_by_type(obj_type);
            for oid in oids {
                if let Some(obj) = db.get(&oid) {
                    // While the object is out of service the client owns Reliability, so
                    // re-deriving it here would defeat a behavior the standard mandates.
                    // The three types ground that differently, so cite them separately:
                    //
                    // - Analog Input, Clause 12.2(b) and (c): Reliability "shall be
                    //   decoupled from the physical input", and it "shall be writable to
                    //   allow simulating specific conditions or for testing purposes".
                    // - Analog Output, Clause 12.3(b) and (c): the same two, worded for a
                    //   physical output.
                    // - Analog Value, Clause 12.4(b): writability for simulation only.
                    //   There is no decoupling item, because an Analog Value has no
                    //   physical point to decouple from.
                    //
                    // Note the letters differ between 12.2/12.3 and 12.4 — writability is
                    // (c) on the first two and (b) on the third.
                    //
                    // Deliberately fail-open: an object that does not report Out_Of_Service,
                    // or reports it at an unexpected type, is still evaluated. That
                    // preserves prior behavior for anything unusual rather than silently
                    // disabling fault detection for it.
                    if matches!(
                        obj.read_property(PropertyIdentifier::OUT_OF_SERVICE, None),
                        Ok(PropertyValue::Boolean(true))
                    ) {
                        continue;
                    }

                    let current_reliability =
                        match obj.read_property(PropertyIdentifier::RELIABILITY, None) {
                            Ok(PropertyValue::Enumerated(v)) => v,
                            _ => 0,
                        };

                    let present_value =
                        match obj.read_property(PropertyIdentifier::PRESENT_VALUE, None) {
                            Ok(PropertyValue::Real(v)) => v,
                            _ => continue,
                        };

                    let min_pres = obj
                        .read_property(PropertyIdentifier::MIN_PRES_VALUE, None)
                        .ok()
                        .and_then(|v| match v {
                            PropertyValue::Real(f) => Some(f),
                            _ => None,
                        });

                    let max_pres = obj
                        .read_property(PropertyIdentifier::MAX_PRES_VALUE, None)
                        .ok()
                        .and_then(|v| match v {
                            PropertyValue::Real(f) => Some(f),
                            _ => None,
                        });

                    let new_reliability = if let Some(max) = max_pres {
                        if present_value > max {
                            Reliability::OVER_RANGE.to_raw()
                        } else if let Some(min) = min_pres {
                            if present_value < min {
                                Reliability::UNDER_RANGE.to_raw()
                            } else {
                                Reliability::NO_FAULT_DETECTED.to_raw()
                            }
                        } else {
                            Reliability::NO_FAULT_DETECTED.to_raw()
                        }
                    } else if let Some(min) = min_pres {
                        if present_value < min {
                            Reliability::UNDER_RANGE.to_raw()
                        } else {
                            Reliability::NO_FAULT_DETECTED.to_raw()
                        }
                    } else {
                        continue;
                    };

                    if new_reliability != current_reliability {
                        updates.push((oid, current_reliability, new_reliability));
                    }
                }
            }
        }

        let mut changes = Vec::new();
        for (oid, old_rel, new_rel) in updates {
            if let Some(obj) = db.get_mut(&oid) {
                match obj.set_reliability_internal(new_rel) {
                    Ok(()) => {
                        self.clear_internal_failure(&oid);
                        changes.push(ReliabilityChange {
                            object_id: oid,
                            old_reliability: old_rel,
                            new_reliability: new_rel,
                        });
                    }
                    Err(error) => {
                        let error_text = error.to_string();
                        if self.should_warn_internal_failure(oid, &error_text) {
                            tracing::warn!(
                                object = %oid,
                                error = %error,
                                "Fault detection could not apply Reliability through BACnetObject::set_reliability_internal"
                            );
                        }
                    }
                }
            }
        }

        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
    use bacnet_types::enums::{ErrorClass, ErrorCode, EventState};
    use bacnet_types::error::Error;

    fn commit_test_proposal(
        object: &mut dyn bacnet_objects::traits::BACnetObject,
        outcome: bacnet_objects::event::TransitionOutcome,
    ) -> bacnet_objects::event::TransitionOutcome {
        object
            .commit_event_transition_internal(bacnet_objects::event::EventTransitionCommit {
                coordinate: outcome.change.transition(),
                change: outcome.change.clone(),
                ack_required: false,
                timestamp: bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(0),
                message_text: None,
            })
            .expect("built-in test proposal must commit");
        outcome
    }

    /// Helper: build an ObjectDatabase with a single AI that has min/max limits.
    fn db_with_analog_input(
        present_value: f32,
        min_pres: Option<f32>,
        max_pres: Option<f32>,
    ) -> ObjectDatabase {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.set_present_value(present_value);
        if let Some(min) = min_pres {
            ai.set_min_pres_value(min);
        }
        if let Some(max) = max_pres {
            ai.set_max_pres_value(max);
        }
        let mut db = ObjectDatabase::new();
        db.add(Box::new(ai)).unwrap();
        db
    }

    #[test]
    fn identical_internal_failure_warning_is_suppressed_until_state_changes() {
        let detector = FaultDetector::default();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        assert!(detector.should_warn_internal_failure(oid, "first error"));
        assert!(!detector.should_warn_internal_failure(oid, "first error"));
        assert!(detector.should_warn_internal_failure(oid, "different error"));
        assert!(!detector.should_warn_internal_failure(oid, "different error"));

        detector.clear_internal_failure(&oid);
        assert!(detector.should_warn_internal_failure(oid, "first error"));
    }

    #[test]
    fn no_fault_when_in_range() {
        let mut db = db_with_analog_input(50.0, Some(0.0), Some(100.0));
        let detector = FaultDetector::default();
        let changes = detector.evaluate(&mut db);
        assert!(changes.is_empty(), "no change expected for in-range value");
    }

    #[test]
    fn over_range_detected() {
        let mut db = db_with_analog_input(150.0, Some(0.0), Some(100.0));
        let detector = FaultDetector::default();
        let changes = detector.evaluate(&mut db);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].new_reliability, Reliability::OVER_RANGE.to_raw());
        assert_eq!(
            changes[0].old_reliability,
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
    }

    #[test]
    fn under_range_detected() {
        let mut db = db_with_analog_input(-10.0, Some(0.0), Some(100.0));
        let detector = FaultDetector::default();
        let changes = detector.evaluate(&mut db);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].new_reliability,
            Reliability::UNDER_RANGE.to_raw()
        );
    }

    #[test]
    fn analog_simulation_restores_derived_fault_without_normal_churn() {
        let mut db = db_with_analog_input(150.0, Some(0.0), Some(100.0));
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let changes = FaultDetector::default().evaluate(&mut db);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].new_reliability, Reliability::OVER_RANGE.to_raw());

        let obj = db.get_mut(&oid).unwrap();
        let proposal = obj
            .evaluate_intrinsic_reporting()
            .expect("derived Reliability must enter FAULT");
        let real_fault = commit_test_proposal(obj.as_mut(), proposal);
        assert_eq!(real_fault.change.to, EventState::FAULT);

        obj.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        obj.write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .unwrap();
        let proposal = obj
            .evaluate_intrinsic_reporting()
            .expect("different simulated fault must re-enter FAULT");
        let simulated_fault = commit_test_proposal(obj.as_mut(), proposal);
        assert_eq!(simulated_fault.change.to, EventState::FAULT);

        obj.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
        assert_eq!(
            obj.read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw())
        );
        assert_eq!(
            obj.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::FAULT.to_raw())
        );

        let proposal = obj
            .evaluate_intrinsic_reporting()
            .expect("restored real fault must re-enter FAULT");
        let restored_fault = commit_test_proposal(obj.as_mut(), proposal);
        assert_eq!(restored_fault.change.to, EventState::FAULT);
        assert_eq!(
            obj.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::FAULT.to_raw())
        );
    }

    #[test]
    fn returns_to_no_fault_after_correction() {
        let mut db = db_with_analog_input(150.0, Some(0.0), Some(100.0));
        let detector = FaultDetector::default();

        // First evaluation: over-range
        let changes = detector.evaluate(&mut db);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].new_reliability, Reliability::OVER_RANGE.to_raw());

        // Correct the value back in range
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            None,
        )
        // AI write needs out_of_service=true
        .unwrap_or_else(|_| {
            obj.write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
            obj.write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(50.0),
                None,
            )
            .unwrap();
        });
        obj.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            obj.read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw())
        );

        // Returning to service restores the pre-simulation fault. The detector
        // then converges it to the corrected physical condition.
        let changes = detector.evaluate(&mut db);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].new_reliability,
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
    }

    #[test]
    fn no_limits_means_no_evaluation() {
        // AI without min/max limits — detector should skip it entirely
        let mut db = db_with_analog_input(999.0, None, None);
        let detector = FaultDetector::default();
        let changes = detector.evaluate(&mut db);
        assert!(changes.is_empty());
    }

    #[test]
    fn max_only_over_range() {
        let mut db = db_with_analog_input(200.0, None, Some(100.0));
        let detector = FaultDetector::default();
        let changes = detector.evaluate(&mut db);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].new_reliability, Reliability::OVER_RANGE.to_raw());
    }

    #[test]
    fn min_only_under_range() {
        let mut db = db_with_analog_input(-5.0, Some(0.0), None);
        let detector = FaultDetector::default();
        let changes = detector.evaluate(&mut db);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].new_reliability,
            Reliability::UNDER_RANGE.to_raw()
        );
    }

    #[test]
    fn no_change_emitted_when_already_faulted() {
        let mut db = db_with_analog_input(150.0, Some(0.0), Some(100.0));
        let detector = FaultDetector::default();

        // First run: change detected
        let changes = detector.evaluate(&mut db);
        assert_eq!(changes.len(), 1);

        // Second run: same fault, no new change
        let changes = detector.evaluate(&mut db);
        assert!(changes.is_empty());
    }

    #[test]
    fn evaluates_multiple_analog_types() {
        let mut db = ObjectDatabase::new();

        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.set_present_value(200.0);
        ai.set_max_pres_value(100.0);
        db.add(Box::new(ai)).unwrap();

        let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        // AO starts at 0.0 — in range with no limits, so skipped
        db.add(Box::new(ao)).unwrap();

        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        av.set_present_value(-10.0);
        av.set_min_pres_value(0.0);
        db.add(Box::new(av)).unwrap();

        let detector = FaultDetector::default();
        let changes = detector.evaluate(&mut db);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn out_of_service_preserves_client_written_reliability_without_change_record() {
        let mut db = db_with_analog_input(50.0, Some(0.0), Some(100.0));
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        obj.write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .unwrap();

        let changes = FaultDetector::default().evaluate(&mut db);

        assert!(changes.is_empty());
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw())
        );
    }

    #[test]
    fn in_service_refuses_client_write_and_recomputes_reliability() {
        let mut db = db_with_analog_input(50.0, Some(0.0), Some(100.0));
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let obj = db.get_mut(&oid).unwrap();
        obj.set_reliability_internal(Reliability::NO_SENSOR.to_raw())
            .unwrap();
        let error = obj
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
                None,
            )
            .expect_err("in-service Reliability write must be refused");

        let changes = FaultDetector::default().evaluate(&mut db);

        match error {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32);
            }
            other => panic!("expected PROPERTY / WRITE_ACCESS_DENIED, got {other:?}"),
        }
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].object_id, oid);
        assert_eq!(
            changes[0].new_reliability,
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
        );
    }

    #[test]
    fn out_of_service_out_of_range_does_not_overwrite_client_reliability() {
        let mut db = db_with_analog_input(150.0, Some(0.0), Some(100.0));
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let obj = db.get_mut(&oid).unwrap();
        obj.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        obj.write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .unwrap();

        let changes = FaultDetector::default().evaluate(&mut db);

        assert!(changes.is_empty());
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw())
        );
    }

    #[test]
    fn mixed_service_states_evaluate_only_in_service_object() {
        let mut db = ObjectDatabase::new();
        let mut in_service = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        in_service.set_present_value(150.0);
        in_service.set_max_pres_value(100.0);
        db.add(Box::new(in_service)).unwrap();

        let mut out_of_service = AnalogInputObject::new(2, "AI-2", 62).unwrap();
        out_of_service.set_present_value(150.0);
        out_of_service.set_max_pres_value(100.0);
        let out_of_service_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
        db.add(Box::new(out_of_service)).unwrap();
        let obj = db.get_mut(&out_of_service_oid).unwrap();
        obj.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        obj.write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .unwrap();

        let changes = FaultDetector::default().evaluate(&mut db);

        let in_service_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].object_id, in_service_oid);
        assert_eq!(changes[0].new_reliability, Reliability::OVER_RANGE.to_raw());
        assert_eq!(
            db.get(&out_of_service_oid)
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw())
        );
    }
}
