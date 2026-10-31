//! COV_Increment is required of every object that reports COV (#2193).
//!
//! Each Clause 12 table with a COV_Increment row makes it required of an
//! object that supports COV reporting (the footnotes are cited on
//! `COV_INCREMENT_ROW`). Presence conditions are tags on rows an object
//! builds for itself, so this audit is where the tag meets the object's
//! own `supports_cov()`: on every representative, a COV_Increment row is
//! tagged exactly when the object reports COV, and every tagged row is
//! required.

use super::*;

/// The types whose tables have a COV_Increment row, independent of the
/// production metadata.
const COV_INCREMENT_TYPES: [ObjectType; 10] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::LOOP,
    ObjectType::PULSE_CONVERTER,
    ObjectType::LARGE_ANALOG_VALUE,
    ObjectType::INTEGER_VALUE,
    ObjectType::POSITIVE_INTEGER_VALUE,
    ObjectType::LIGHTING_OUTPUT,
    ObjectType::STAGING,
];

#[test]
fn every_type_that_reports_cov_lists_its_cov_increment_as_required() {
    let objects = supported_representatives();
    audit_object_type_coverage(&objects);
    let mut with_row = Vec::new();
    for object in &objects {
        let kind = object.object_identifier().object_type();
        let metadata = object.property_metadata();
        let required = object.required_properties();
        for row in metadata.iter() {
            let p = row.property_identifier;
            let tagged = row.presence_condition == Some(PropertyPresenceCondition::CovReporting);
            if tagged {
                assert!(object.supports_cov(), "{kind:?} {p:?} tagged without COV");
                assert!(row.is_required(), "{kind:?} {p:?}");
                assert!(required.contains(&p), "{kind:?} {p:?}");
            }
            if p == PropertyIdentifier::COV_INCREMENT {
                with_row.push(kind);
                assert_eq!(row.conformance, PropertyConformance::Optional, "{kind:?}");
                assert_eq!(tagged, object.supports_cov(), "{kind:?}");
            } else if tagged {
                // Only Pulse Converter's COV_Period shares the footnote.
                assert_eq!(
                    (kind, p),
                    (ObjectType::PULSE_CONVERTER, PropertyIdentifier::COV_PERIOD)
                );
            }
        }
    }
    with_row.sort_by_key(|kind| kind.to_raw());
    let mut expected = COV_INCREMENT_TYPES.to_vec();
    expected.sort_by_key(|kind| kind.to_raw());
    assert_eq!(with_row, expected);
    // Every representative of these types reports COV, so each lists it.
    for kind in COV_INCREMENT_TYPES {
        let object = objects
            .iter()
            .find(|object| object.object_identifier().object_type() == kind)
            .unwrap();
        assert!(object.supports_cov(), "{kind:?}");
        assert!(
            object
                .required_properties()
                .contains(&PropertyIdentifier::COV_INCREMENT),
            "{kind:?}"
        );
    }
}
