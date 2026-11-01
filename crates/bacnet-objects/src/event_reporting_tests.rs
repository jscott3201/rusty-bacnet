//! Exactly the built-in types that report events lend `EventReporting`
//! (#2179), through both accessors, and the two lend the same object. A
//! lender dropped from, or added to, the catalogue fails here until the
//! table below changes with it.

use crate::catalogue::representatives;
use alloc::collections::BTreeSet;
use bacnet_types::enums::ObjectType;

/// The built-in types that lend event reporting.
const LENDERS: [ObjectType; 16] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::ACCESS_ZONE,
    ObjectType::ACCESS_DOOR,
    ObjectType::TREND_LOG,
    ObjectType::TREND_LOG_MULTIPLE,
    ObjectType::EVENT_LOG,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::ALERT_ENROLLMENT,
];

#[test]
fn exactly_the_event_reporters_lend_event_reporting_through_both_accessors() {
    let mut seen = BTreeSet::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        let shared = object
            .event_reporting()
            .map(|reporting| (reporting as *const dyn crate::traits::EventReporting).cast::<()>());
        let exclusive = object
            .event_reporting_mut()
            .map(|reporting| (reporting as *mut dyn crate::traits::EventReporting).cast::<()>());
        assert_eq!(
            shared,
            exclusive.map(|pointer| pointer.cast_const()),
            "{object_type:?}: the two accessors must lend the same object"
        );
        let lends = LENDERS.contains(&object_type);
        assert_eq!(
            shared.is_some(),
            lends,
            "{object_type:?} {} lend EventReporting",
            if lends { "must" } else { "must not" }
        );
        seen.insert(object_type.to_raw());
    }
    for object_type in LENDERS {
        assert!(
            seen.contains(&object_type.to_raw()),
            "{object_type:?} is not in the catalogue"
        );
    }
}
