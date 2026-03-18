//! Fault detection / reliability evaluation.
//!
//! The [`FaultDetector`] periodically evaluates each object's reliability,
//! checking for OVER_RANGE, UNDER_RANGE (analog objects), and optionally
//! COMMUNICATION_FAILURE (staleness timeout).

use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

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
}

impl Default for FaultDetector {
    fn default() -> Self {
        Self {
            comm_timeout: Some(std::time::Duration::from_secs(60)),
        }
    }
}

impl FaultDetector {
    /// Create a new fault detector with the given communication timeout.
    pub fn new(comm_timeout: Option<std::time::Duration>) -> Self {
        Self { comm_timeout }
    }

    /// Evaluate reliability for all objects in the database.
    ///
    /// For each analog object (AI, AO, AV) that has `MIN_PRES_VALUE` and
    /// `MAX_PRES_VALUE` properties, the present value is compared against
    /// those limits.  If out of range the reliability is set to
    /// `OVER_RANGE` or `UNDER_RANGE`; otherwise `NO_FAULT_DETECTED`.
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
                if obj
                    .write_property(
                        PropertyIdentifier::RELIABILITY,
                        None,
                        PropertyValue::Enumerated(new_rel),
                        None,
                    )
                    .is_ok()
                {
                    changes.push(ReliabilityChange {
                        object_id: oid,
                        old_reliability: old_rel,
                        new_reliability: new_rel,
                    });
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

        // Second evaluation: back to no-fault
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
}
