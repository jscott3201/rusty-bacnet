//! Exactly the built-in types that report COV lend `CovReporting` (#2179),
//! and exactly the Averaging and Command objects among them refuse
//! SubscribeCOV. A lender dropped from, or added to, the catalogue fails
//! here until the tables below change with it.

use crate::catalogue::representatives;
use crate::traits::CovReportedProperty;
use alloc::collections::BTreeSet;
use bacnet_types::enums::ObjectType;

/// The built-in types that lend COV reporting.
const LENDERS: [ObjectType; 37] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::BITSTRING_VALUE,
    ObjectType::CHARACTERSTRING_VALUE,
    ObjectType::DATE_VALUE,
    ObjectType::DATEPATTERN_VALUE,
    ObjectType::DATETIME_VALUE,
    ObjectType::DATETIMEPATTERN_VALUE,
    ObjectType::INTEGER_VALUE,
    ObjectType::LARGE_ANALOG_VALUE,
    ObjectType::OCTETSTRING_VALUE,
    ObjectType::POSITIVE_INTEGER_VALUE,
    ObjectType::TIME_VALUE,
    ObjectType::TIMEPATTERN_VALUE,
    ObjectType::LOOP,
    ObjectType::STAGING,
    ObjectType::LOAD_CONTROL,
    ObjectType::ACCUMULATOR,
    ObjectType::PULSE_CONVERTER,
    ObjectType::LIGHTING_OUTPUT,
    ObjectType::BINARY_LIGHTING_OUTPUT,
    ObjectType::COLOR,
    ObjectType::COLOR_TEMPERATURE,
    ObjectType::LIFE_SAFETY_POINT,
    ObjectType::LIFE_SAFETY_ZONE,
    ObjectType::ACCESS_DOOR,
    ObjectType::ACCESS_POINT,
    ObjectType::CREDENTIAL_DATA_INPUT,
    ObjectType::AVERAGING,
    ObjectType::COMMAND,
];

/// The lenders with no Table 13-1 row: they take property subscriptions
/// only.
const PROPERTY_ONLY: [ObjectType; 2] = [ObjectType::AVERAGING, ObjectType::COMMAND];

#[test]
fn exactly_the_cov_reporters_lend_cov_reporting() {
    let mut seen = BTreeSet::new();
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        let lends = LENDERS.contains(&object_type);
        let cov = object.cov();
        assert_eq!(
            cov.is_some(),
            lends,
            "{object_type:?} {} lend CovReporting",
            if lends { "must" } else { "must not" }
        );
        if let Some(cov) = cov {
            assert_eq!(
                cov.supports_subscribe_cov(),
                !PROPERTY_ONLY.contains(&object_type),
                "{object_type:?}: SubscribeCOV"
            );
            // Each lender reports its type's Table 13-1 extras.
            assert_eq!(
                cov.cov_reported_properties(),
                CovReportedProperty::standard_for(object_type),
                "{object_type:?}"
            );
        }
        seen.insert(object_type.to_raw());
    }
    for object_type in LENDERS {
        assert!(
            seen.contains(&object_type.to_raw()),
            "{object_type:?} is not in the catalogue"
        );
    }
}

#[test]
fn every_type_with_table_13_1_extras_lends_cov_reporting() {
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        if !CovReportedProperty::standard_for(object_type).is_empty() {
            assert!(LENDERS.contains(&object_type), "{object_type:?}");
            assert!(object.cov().is_some(), "{object_type:?}");
        }
    }
}
