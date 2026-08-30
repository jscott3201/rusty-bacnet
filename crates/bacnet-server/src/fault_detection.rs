//! Fault detection / reliability evaluation.
//!
//! The [`FaultDetector`] periodically invokes each object's opt-in,
//! object-owned reliability evaluation hook.

use std::collections::HashMap;
use std::sync::Mutex;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::ReliabilityEvaluation;
use bacnet_types::primitives::ObjectIdentifier;

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
/// Call [`FaultDetector::evaluate`] periodically (the bundled server uses a
/// 10-second interval) against the object database. Every object is visited;
/// objects opt in by overriding
/// [`BACnetObject::evaluate_reliability_internal`](bacnet_objects::traits::BACnetObject::evaluate_reliability_internal).
/// The default object hook is a no-op.
pub struct FaultDetector {
    /// Timeout after which an object is considered to have a communication
    /// failure. Set to `None` to disable communication-failure detection.
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
    /// Each object owns both the decision and any mutation. Engineering
    /// `Min_Pres_Value` / `Max_Pres_Value` metadata is not a reliability fault
    /// algorithm. Only a successful hook result of
    /// [`ReliabilityEvaluation::Changed`] produces a [`ReliabilityChange`].
    /// Hook errors are rate-limited per object and produce no change record.
    ///
    /// Returns a list of changes that object hooks successfully applied.
    pub fn evaluate(&self, db: &mut ObjectDatabase) -> Vec<ReliabilityChange> {
        let mut changes = Vec::new();
        db.for_each_object_mut(|oid, object| match object.evaluate_reliability_internal() {
            Ok(ReliabilityEvaluation::Unchanged) => {
                self.clear_internal_failure(&oid);
            }
            Ok(ReliabilityEvaluation::Changed {
                old_reliability,
                new_reliability,
            }) => {
                self.clear_internal_failure(&oid);
                changes.push(ReliabilityChange {
                    object_id: oid,
                    old_reliability,
                    new_reliability,
                });
            }
            Err(error) => {
                let error_text = error.to_string();
                if self.should_warn_internal_failure(oid, &error_text) {
                    tracing::warn!(
                        object = %oid,
                        error = %error,
                        "Fault detection object-owned reliability evaluation failed"
                    );
                }
            }
        });
        changes
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier, Reliability};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;

    use super::*;

    const HOOK_ERROR: &str = "test reliability hook failed";

    fn read_reliability(db: &ObjectDatabase, oid: ObjectIdentifier) -> u32 {
        match db
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap()
        {
            PropertyValue::Enumerated(value) => value,
            other => panic!("expected Enumerated Reliability, got {other:?}"),
        }
    }

    struct OptInReliabilityObject {
        oid: ObjectIdentifier,
        name: String,
        reliability: u32,
        target_reliability: u32,
        fail: Arc<AtomicBool>,
    }

    impl OptInReliabilityObject {
        fn new(instance: u32, name: &str, target_reliability: u32, fail: Arc<AtomicBool>) -> Self {
            Self {
                oid: ObjectIdentifier::new(ObjectType::BINARY_INPUT, instance).unwrap(),
                name: name.to_owned(),
                reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
                target_reliability,
                fail,
            }
        }
    }

    impl BACnetObject for OptInReliabilityObject {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.oid
        }

        fn object_name(&self) -> &str {
            &self.name
        }

        fn read_property(
            &self,
            property: PropertyIdentifier,
            _array_index: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            if property == PropertyIdentifier::RELIABILITY {
                Ok(PropertyValue::Enumerated(self.reliability))
            } else {
                Err(Error::Encoding("test property is unsupported".into()))
            }
        }

        fn write_property(
            &mut self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
            _value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), Error> {
            Err(Error::Encoding("test object is read-only".into()))
        }

        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            Cow::Borrowed(&[PropertyIdentifier::RELIABILITY])
        }

        fn evaluate_reliability_internal(&mut self) -> Result<ReliabilityEvaluation, Error> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(Error::Encoding(HOOK_ERROR.into()));
            }
            if self.reliability == self.target_reliability {
                return Ok(ReliabilityEvaluation::Unchanged);
            }

            let old_reliability = self.reliability;
            self.reliability = self.target_reliability;
            Ok(ReliabilityEvaluation::Changed {
                old_reliability,
                new_reliability: self.reliability,
            })
        }
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
    fn stock_analog_engineering_bounds_never_change_reliability() {
        let mut db = ObjectDatabase::new();

        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.set_min_pres_value(0.0);
        ai.set_max_pres_value(100.0);
        ai.set_present_value(200.0);
        let ai_oid = ai.object_identifier();
        db.add(Box::new(ai)).unwrap();

        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        ao.set_min_pres_value(0.0);
        ao.set_max_pres_value(100.0);
        ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(150.0),
            Some(8),
        )
        .unwrap();
        let ao_oid = ao.object_identifier();
        db.add(Box::new(ao)).unwrap();

        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        av.set_min_pres_value(0.0);
        av.set_max_pres_value(100.0);
        av.set_present_value(-10.0);
        let av_oid = av.object_identifier();
        db.add(Box::new(av)).unwrap();

        let changes = FaultDetector::default().evaluate(&mut db);

        assert!(changes.is_empty());
        for oid in [ai_oid, ao_oid, av_oid] {
            assert_eq!(
                read_reliability(&db, oid),
                Reliability::NO_FAULT_DETECTED.to_raw()
            );
        }
        let ao = db.get(&ao_oid).unwrap();
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(150.0)
        );
        assert_eq!(
            ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
                .unwrap(),
            PropertyValue::Real(150.0)
        );
    }

    #[test]
    fn existing_nonzero_in_service_reliability_is_preserved() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.set_min_pres_value(0.0);
        ai.set_max_pres_value(100.0);
        ai.set_present_value(50.0);
        ai.set_reliability_internal(Reliability::NO_SENSOR.to_raw())
            .unwrap();
        let oid = ai.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(ai)).unwrap();

        assert!(FaultDetector::default().evaluate(&mut db).is_empty());
        assert_eq!(read_reliability(&db, oid), Reliability::NO_SENSOR.to_raw());
    }

    #[test]
    fn out_of_service_client_write_and_restore_remain_object_owned() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.set_reliability_internal(Reliability::OVER_RANGE.to_raw())
            .unwrap();
        let oid = ai.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(ai)).unwrap();

        let object = db.get_mut(&oid).unwrap();
        object
            .write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        object
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
                None,
            )
            .unwrap();

        assert!(FaultDetector::default().evaluate(&mut db).is_empty());
        assert_eq!(read_reliability(&db, oid), Reliability::NO_SENSOR.to_raw());

        db.get_mut(&oid)
            .unwrap()
            .write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(false),
                None,
            )
            .unwrap();
        assert_eq!(read_reliability(&db, oid), Reliability::OVER_RANGE.to_raw());
    }

    #[test]
    fn only_hook_returned_changed_creates_a_change_record() {
        let fail = Arc::new(AtomicBool::new(false));
        let object =
            OptInReliabilityObject::new(1, "custom-hook", Reliability::NO_SENSOR.to_raw(), fail);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let detector = FaultDetector::default();
        assert!(detector.should_warn_internal_failure(oid, HOOK_ERROR));

        assert_eq!(
            detector.evaluate(&mut db),
            vec![ReliabilityChange {
                object_id: oid,
                old_reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
                new_reliability: Reliability::NO_SENSOR.to_raw(),
            }]
        );
        assert_eq!(read_reliability(&db, oid), Reliability::NO_SENSOR.to_raw());
        assert!(detector.should_warn_internal_failure(oid, HOOK_ERROR));
        assert!(detector.evaluate(&mut db).is_empty());
        assert!(detector.should_warn_internal_failure(oid, HOOK_ERROR));
    }

    #[test]
    fn failing_hook_is_rate_limited_and_cannot_create_a_change() {
        let fail = Arc::new(AtomicBool::new(true));
        let object = OptInReliabilityObject::new(
            1,
            "failing-hook",
            Reliability::NO_FAULT_DETECTED.to_raw(),
            Arc::clone(&fail),
        );
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let detector = FaultDetector::default();
        let error_text = Error::Encoding(HOOK_ERROR.into()).to_string();

        assert!(detector.evaluate(&mut db).is_empty());
        assert_eq!(
            read_reliability(&db, oid),
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
        assert!(!detector.should_warn_internal_failure(oid, &error_text));
        assert!(detector.evaluate(&mut db).is_empty());
        assert_eq!(
            read_reliability(&db, oid),
            Reliability::NO_FAULT_DETECTED.to_raw()
        );

        fail.store(false, Ordering::SeqCst);
        assert!(detector.evaluate(&mut db).is_empty());
        assert_eq!(
            read_reliability(&db, oid),
            Reliability::NO_FAULT_DETECTED.to_raw()
        );
        assert!(detector.should_warn_internal_failure(oid, &error_text));
    }
}
