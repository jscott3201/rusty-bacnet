//! Exactly the built-in types that keep Event Enrollment evaluation state
//! lend `EnrollmentEval`, through both accessors that lend the same object,
//! and exactly those that evaluate or take a derived Reliability lend
//! `ReliabilityEval` (#2179). A lender dropped from, or added to, the
//! catalogue fails here until its table changes with it.

use crate::catalogue::representatives;
use alloc::collections::BTreeSet;
use bacnet_types::enums::ObjectType;

#[cfg(feature = "event-reporting")]
/// The built-in type that lends Event Enrollment evaluation.
const ENROLLMENT_EVAL_LENDERS: [ObjectType; 1] = [ObjectType::EVENT_ENROLLMENT];

/// The built-in types that lend reliability evaluation.
const RELIABILITY_EVAL_LENDERS: [ObjectType; 17] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::LOOP,
    ObjectType::SCHEDULE,
    ObjectType::LIFE_SAFETY_POINT,
    ObjectType::LIFE_SAFETY_ZONE,
    ObjectType::ACCESS_ZONE,
    ObjectType::CREDENTIAL_DATA_INPUT,
    // Takes the application's Reliability in service (#2280).
    ObjectType::ACCUMULATOR,
    // Likewise (#2446).
    ObjectType::INTEGER_VALUE,
];

fn assert_in_catalogue(seen: &BTreeSet<u32>, lenders: &[ObjectType]) {
    for object_type in lenders {
        assert!(
            seen.contains(&object_type.to_raw()),
            "{object_type:?} is not in the catalogue"
        );
    }
}

#[cfg(feature = "event-reporting")]
#[test]
fn exactly_the_event_enrollment_lends_enrollment_eval_through_both_accessors() {
    let mut seen = BTreeSet::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        let shared = object
            .enrollment_eval()
            .map(|eval| (eval as *const dyn crate::traits::EnrollmentEval).cast::<()>());
        let exclusive = object
            .enrollment_eval_mut()
            .map(|eval| (eval as *mut dyn crate::traits::EnrollmentEval).cast::<()>());
        assert_eq!(
            shared,
            exclusive.map(|pointer| pointer.cast_const()),
            "{object_type:?}: the two accessors must lend the same object"
        );
        let lends = ENROLLMENT_EVAL_LENDERS.contains(&object_type);
        assert_eq!(
            shared.is_some(),
            lends,
            "{object_type:?} {} lend EnrollmentEval",
            if lends { "must" } else { "must not" }
        );
        seen.insert(object_type.to_raw());
    }
    assert_in_catalogue(&seen, &ENROLLMENT_EVAL_LENDERS);
}

#[test]
fn exactly_the_reliability_evaluators_lend_reliability_eval() {
    let mut seen = BTreeSet::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        let lends = RELIABILITY_EVAL_LENDERS.contains(&object_type);
        assert_eq!(
            object.reliability_eval().is_some(),
            lends,
            "{object_type:?} {} lend ReliabilityEval",
            if lends { "must" } else { "must not" }
        );
        seen.insert(object_type.to_raw());
    }
    assert_in_catalogue(&seen, &RELIABILITY_EVAL_LENDERS);
}
