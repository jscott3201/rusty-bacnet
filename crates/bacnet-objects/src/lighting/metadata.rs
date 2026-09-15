use super::binary::BinaryLightingOutputObject;
use super::LightingOutputObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for the Lighting duo (ASHRAE 135-2020; PDF = printed + 2):
// - Lighting Output (type 54, §12.54 Table 12-64; printed pp. 518-519 / PDF pp. 520-521)
// - Binary Lighting Output (type 55, §12.55 Table 12-69; printed pp. 532-533 / PDF pp. 534-535)
// Order preserves each legacy projection; DEFAULT_FADE_TIME (readable but
// unlisted, served constant Unsigned 0) is appended after the Lighting Output
// legacy rows (Lift FLOOR_NUMBER precedent) and PROPERTY_LIST is appended so
// the projection helper omits it while required_properties keeps it. Only
// implemented rows are described: table rows the objects do not serve
// (Lighting Output Default_Ramp_Rate, Default_Step_Increment, Transition,
// Feedback_Value, Power, Instantaneous_Power, Min/Max_Actual_Value,
// Current_Command_Priority, Value_Source family, event/intrinsic/audit/tag/
// profile rows; Binary Lighting Output Feedback_Value, Power, Polarity,
// Elapsed_Active_Time family, Current_Command_Priority, Value_Source family,
// event/intrinsic/audit/tag/profile rows) stay absent until dispatch exists.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED (unlike commandable analog/binary outputs, lighting has
// no write_object_name arm). Description carries the table O code with a
// routed CharacterString write arm, so Optional/Always. Present_Value and
// Lighting_Command carry the table W code and dispatch accepts the
// commandable Real / OctetString routes, so RequiredWrite/Always.
// Table-R served rows with no network write route stay RequiredRead/ReadOnly;
// table-R rows with a write arm are RequiredRead/Always. Table-O served rows
// are Optional, with Always exactly where dispatch accepts the write
// (Description) and ReadOnly where it does not (Reliability).
// Default_Fade_Time carries the table R code and is readable as constant
// Unsigned 0 with no write arm, so RequiredRead/ReadOnly.
// Writability is Always, never WhenOutOfService: dispatch routes every write
// arm unconditionally and the suites pin in-service writes, so the metadata
// mirrors dispatch. Presence is None throughout: the implementation models no
// commandable, intrinsic-reporting, or value-source gating on this family.
// The duo is not createable at runtime (the network factory builds only the
// eight analog/binary/multi-state input/output/value types, so the
// is_createable=false default holds) and remains deleteable (delete denies
// only Device and NetworkPort, so the is_deleteable=true default holds);
// neither needs an override. Array gating keeps the default: Priority_Array
// and Property_List admit an index (BACnetARRAY per Tables 12-64/12-69)
// while every other served row rejects one. COV keeps its override:
// supports_cov=true on both objects, and the COV gating path (read_property
// plus supports_cov_property to supports_cov) never consults metadata.
const LIGHTING_OUTPUT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::TRACKING_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LIGHTING_COMMAND, RequiredWrite, None, Always),
    PropertyMetadata::new(
        P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(P::IN_PROGRESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::BLINK_WARN_ENABLE, RequiredRead, None, Always),
    PropertyMetadata::new(P::EGRESS_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::EGRESS_ACTIVE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::DEFAULT_FADE_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const BINARY_LIGHTING_OUTPUT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::BLINK_WARN_ENABLE, RequiredRead, None, Always),
    PropertyMetadata::new(P::EGRESS_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::EGRESS_ACTIVE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_lighting_output_object(
    _object: &LightingOutputObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(LIGHTING_OUTPUT_BASE)
}

pub(super) fn for_binary_lighting_output_object(
    _object: &BinaryLightingOutputObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BINARY_LIGHTING_OUTPUT_BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property_metadata::PropertyWriteCapability;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;
    use std::collections::HashSet;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    fn assert_exact_sets(object: &dyn BACnetObject, all: &[P], required: &[P]) {
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), all.len() + 1);
        assert_eq!(object.property_list().as_ref(), all);
        assert_eq!(object.required_properties().as_ref(), required);
        assert_eq!(
            metadata
                .iter()
                .map(|row| row.property_identifier)
                .collect::<HashSet<_>>()
                .len(),
            metadata.len()
        );
        assert!(!object.is_createable());
        assert!(object.is_deleteable());
        assert!(object.supports_cov());
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            let expected = if required.contains(&row.property_identifier) {
                if row.property_identifier == P::PRESENT_VALUE
                    || row.property_identifier == P::LIGHTING_COMMAND
                {
                    RequiredWrite
                } else {
                    RequiredRead
                }
            } else {
                Optional
            };
            assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
            object.read_property(row.property_identifier, None).unwrap();
        }
    }

    fn assert_indexed_property_list(object: &dyn BACnetObject, all: &[P]) {
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert!(object.is_array_property(P::PRIORITY_ARRAY));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(wire.len() as u64)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [wire.len() as u32 + 1, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_lighting_output_exact_sets_readable_rows_and_indexed_list() {
        let object = LightingOutputObject::new(1, "LO-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::LIGHTING_COMMAND,
            P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
            P::IN_PROGRESS,
            P::BLINK_WARN_ENABLE,
            P::EGRESS_TIME,
            P::EGRESS_ACTIVE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
            P::DEFAULT_FADE_TIME,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::LIGHTING_COMMAND,
            P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
            P::IN_PROGRESS,
            P::BLINK_WARN_ENABLE,
            P::EGRESS_TIME,
            P::EGRESS_ACTIVE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
            P::DEFAULT_FADE_TIME,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            object.read_property(P::TRACKING_VALUE, None).unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            object.read_property(P::LIGHTING_COMMAND, None).unwrap(),
            PropertyValue::OctetString(vec![])
        );
        assert_eq!(
            object
                .read_property(P::LIGHTING_COMMAND_DEFAULT_PRIORITY, None)
                .unwrap(),
            PropertyValue::Unsigned(16)
        );
        assert_eq!(
            object.read_property(P::DEFAULT_FADE_TIME, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object.read_property(P::EGRESS_ACTIVE, None).unwrap(),
            PropertyValue::Boolean(false)
        );
        // Priority_Array is BACnetARRAY (Table 12-64), so the service gate
        // admits an index; Tracking_Value is scalar and rejects one.
        assert!(object.is_array_property(P::PRIORITY_ARRAY));
        assert!(!object.is_array_property(P::TRACKING_VALUE));
        assert!(!object.is_array_property(P::DEFAULT_FADE_TIME));
    }

    #[test]
    fn property_metadata_binary_lighting_output_exact_sets_readable_rows_and_indexed_list() {
        let object = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::BLINK_WARN_ENABLE,
            P::EGRESS_TIME,
            P::EGRESS_ACTIVE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::BLINK_WARN_ENABLE,
            P::EGRESS_TIME,
            P::EGRESS_ACTIVE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::EGRESS_ACTIVE, None).unwrap(),
            PropertyValue::Boolean(false)
        );
        assert!(object.is_array_property(P::PRIORITY_ARRAY));
        assert!(!object.is_array_property(P::BLINK_WARN_ENABLE));
    }

    #[test]
    fn property_metadata_lighting_duo_write_capabilities_match_dispatch() {
        let cases: [(fn() -> Box<dyn BACnetObject>, &[P]); 2] = [
            (
                || Box::new(LightingOutputObject::new(1, "LO-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::PRESENT_VALUE,
                    P::LIGHTING_COMMAND,
                    P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
                    P::BLINK_WARN_ENABLE,
                    P::EGRESS_TIME,
                    P::PRIORITY_ARRAY,
                    P::RELINQUISH_DEFAULT,
                ],
            ),
            (
                || Box::new(BinaryLightingOutputObject::new(1, "BLO-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::PRESENT_VALUE,
                    P::BLINK_WARN_ENABLE,
                    P::EGRESS_TIME,
                    P::PRIORITY_ARRAY,
                    P::RELINQUISH_DEFAULT,
                ],
            ),
        ];
        for (make, writable) in cases {
            for out_of_service in [false, true] {
                let mut object = make();
                object
                    .write_property(
                        P::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Boolean(out_of_service),
                        None,
                    )
                    .unwrap();
                let original = object.property_metadata().into_owned();
                for row in &original {
                    let p = row.property_identifier;
                    let capability = if writable.contains(&p) {
                        PropertyWriteCapability::Always
                    } else {
                        PropertyWriteCapability::ReadOnly
                    };
                    assert_eq!(row.write_capability, capability, "{p:?}");
                    assert_eq!(
                        object.is_writable_property(p),
                        capability.is_writable(),
                        "{p:?}"
                    );
                    // Priority_Array whole-array writes are denied by design;
                    // exercise an indexed slot instead so the Always route is
                    // proven without the whole-array denial.
                    let (value, index) = if p == P::PRIORITY_ARRAY {
                        let value = object.read_property(p, Some(8)).unwrap();
                        (value, Some(8))
                    } else {
                        let value = object.read_property(p, None).unwrap();
                        (value, None)
                    };
                    let result = object.write_property(p, index, value, None);
                    if capability.is_writable() {
                        result.unwrap();
                    } else {
                        assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                    }
                }
                // Object_Name has no network write route: a rename falls
                // through to WRITE_ACCESS_DENIED even with a well-formed value.
                assert!(!object.is_writable_property(P::OBJECT_NAME));
                assert_error(
                    object
                        .write_property(
                            P::OBJECT_NAME,
                            None,
                            PropertyValue::CharacterString("renamed".into()),
                            None,
                        )
                        .unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                // Default_Fade_Time is readable as constant 0 with no write
                // arm: even its own readback is denied on write.
                if object.object_identifier().object_type()
                    == bacnet_types::enums::ObjectType::LIGHTING_OUTPUT
                {
                    assert!(!object.is_writable_property(P::DEFAULT_FADE_TIME));
                    assert_error(
                        object
                            .write_property(
                                P::DEFAULT_FADE_TIME,
                                None,
                                PropertyValue::Unsigned(0),
                                None,
                            )
                            .unwrap_err(),
                        ErrorCode::WRITE_ACCESS_DENIED,
                    );
                }
                assert_eq!(object.property_metadata().as_ref(), original);
            }
        }
    }

    #[test]
    fn property_metadata_lighting_output_writes_store_verbatim_with_range_gates() {
        for out_of_service in [false, true] {
            let mut object = LightingOutputObject::new(1, "LO-1").unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(50.0), Some(8))
                .unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Real(50.0)
            );
            object
                .write_property(
                    P::LIGHTING_COMMAND,
                    None,
                    PropertyValue::OctetString(vec![0x01, 0x02]),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::LIGHTING_COMMAND, None).unwrap(),
                PropertyValue::OctetString(vec![0x01, 0x02])
            );
            object
                .write_property(
                    P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
                    None,
                    PropertyValue::Unsigned(8),
                    None,
                )
                .unwrap();
            assert_eq!(
                object
                    .read_property(P::LIGHTING_COMMAND_DEFAULT_PRIORITY, None)
                    .unwrap(),
                PropertyValue::Unsigned(8)
            );
            for raw in [0u64, 17, u64::from(u32::MAX)] {
                assert_error(
                    object
                        .write_property(
                            P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
                            None,
                            PropertyValue::Unsigned(raw),
                            None,
                        )
                        .unwrap_err(),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
            }
            object
                .write_property(P::RELINQUISH_DEFAULT, None, PropertyValue::Real(75.0), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
                PropertyValue::Real(75.0)
            );
            assert_error(
                object
                    .write_property(
                        P::RELINQUISH_DEFAULT,
                        None,
                        PropertyValue::Real(101.0),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            for (p, value) in [
                (P::PRESENT_VALUE, PropertyValue::Enumerated(1)),
                (P::LIGHTING_COMMAND, PropertyValue::Unsigned(1)),
                (
                    P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
                    PropertyValue::Enumerated(8),
                ),
                (P::DESCRIPTION, PropertyValue::Null),
                (P::OUT_OF_SERVICE, PropertyValue::Null),
                (P::BLINK_WARN_ENABLE, PropertyValue::Enumerated(1)),
                (P::EGRESS_TIME, PropertyValue::Boolean(true)),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            for p in [
                P::TRACKING_VALUE,
                P::IN_PROGRESS,
                P::EGRESS_ACTIVE,
                P::STATUS_FLAGS,
                P::RELIABILITY,
            ] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
            }
        }
    }

    #[test]
    fn property_metadata_lighting_duo_unserved_rows_stay_unknown() {
        fn assert_unserved(object: &mut dyn BACnetObject, p: P) {
            assert!(!object.is_writable_property(p));
            assert_error(
                object.read_property(p, None).unwrap_err(),
                ErrorCode::UNKNOWN_PROPERTY,
            );
            assert_error(
                object
                    .write_property(p, None, PropertyValue::Null, None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }

        let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
        assert_unserved(&mut lo, P::DEFAULT_RAMP_RATE);
        assert_unserved(&mut lo, P::DEFAULT_STEP_INCREMENT);
        assert_unserved(&mut lo, P::FEEDBACK_VALUE);
        assert_unserved(&mut lo, P::CURRENT_COMMAND_PRIORITY);
        let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        assert_unserved(&mut blo, P::FEEDBACK_VALUE);
        assert_unserved(&mut blo, P::POLARITY);
        assert_unserved(&mut blo, P::CURRENT_COMMAND_PRIORITY);
    }
}
