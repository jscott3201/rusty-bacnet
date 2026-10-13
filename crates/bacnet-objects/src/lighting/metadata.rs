use super::binary::BinaryLightingOutputObject;
use super::LightingOutputObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata, PropertyPresenceCondition,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for the Lighting duo (ASHRAE 135-2020; PDF = printed + 2):
// - Lighting Output (type 54, §12.54 Table 12-64; printed pp. 518-519 / PDF pp. 520-521)
// - Binary Lighting Output (type 55, §12.55 Table 12-69; printed pp. 532-533 / PDF pp. 534-535)
// Order preserves each legacy projection; DEFAULT_FADE_TIME (once readable
// but unlisted) is appended after the Lighting Output legacy rows (Lift
// FLOOR_NUMBER precedent), followed by the rows #1092 added
// (Default_Ramp_Rate, Default_Step_Increment, then Current_Command_Priority
// on both objects), and PROPERTY_LIST is appended so the projection helper
// omits it while required_properties keeps it. Only implemented rows are
// described: table rows the objects do not serve (Lighting Output
// Transition, Feedback_Value, Power, Instantaneous_Power,
// Min/Max_Actual_Value, Value_Source family, event/intrinsic/audit rows;
// Binary Lighting Output Feedback_Value, Power, Polarity,
// Elapsed_Active_Time family, Value_Source family, event/intrinsic/audit
// rows) are all optional and stay absent until dispatch exists. Tags,
// Profile_Location and Profile_Name join once `set_profile` provisions
// them (#1553).
// The colour links of both objects (Color_Reference, Color_Override and
// Override_Color_Reference, Addendum 135-2020ca part 4, #1527) and Lighting
// Output's trims (High_End_Trim, Low_End_Trim and Trim_Fade_Time, part 5,
// #1528) join the rows once set; see `for_lighting_output_object`.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED (unlike commandable analog/binary outputs, lighting has
// no write_object_name arm). Description carries the table O code with a
// routed CharacterString write arm, so Optional/Always. Present_Value and
// Lighting_Command carry the table W code and dispatch accepts the
// commandable Real and the BACnetLightingCommand routes, so
// RequiredWrite/Always.
// Table-R served rows with no network write route stay RequiredRead/ReadOnly;
// table-R rows with a write arm are RequiredRead/Always. Table-O served rows
// are Optional, with Always exactly where dispatch accepts the write
// (Description) and ReadOnly where it does not (Reliability).
// Default_Fade_Time, Default_Ramp_Rate and Default_Step_Increment carry the
// table R code and take range-checked writes (Clauses 12.54.16 to 12.54.18
// give each a range and the error for a write outside it; #1092, #1111), so
// RequiredRead/Always.
// Current_Command_Priority carries the table R code on both objects and is
// derived from Priority_Array (Clauses 12.54.39 and 12.55.32), so
// RequiredRead/ReadOnly.
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
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::DEFAULT_FADE_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DEFAULT_RAMP_RATE, RequiredRead, None, Always),
    PropertyMetadata::new(P::DEFAULT_STEP_INCREMENT, RequiredRead, None, Always),
    PropertyMetadata::new(P::CURRENT_COMMAND_PRIORITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::COV_INCREMENT, Optional, None, Always),
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
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::CURRENT_COMMAND_PRIORITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_lighting_output_object(
    object: &LightingOutputObject,
) -> Cow<'_, [PropertyMetadata]> {
    // The colour links (#1527) come first, in the table's order. The trims
    // (#1528) are Optional rows that take writes, each present once set;
    // Trim_Fade_Time comes with either, and the footnote to Table 12-64 makes
    // it required then.
    let trims = &object.trims;
    let trim_rows = [
        (P::HIGH_END_TRIM, trims.high_end().is_some(), None),
        (P::LOW_END_TRIM, trims.low_end().is_some(), None),
        (
            P::TRIM_FADE_TIME,
            trims.has_fade_time(),
            Some(PropertyPresenceCondition::LightingTrims),
        ),
    ]
    .into_iter()
    .filter(|&(_, present, _)| present)
    .map(|(property, _, condition)| PropertyMetadata::new(property, Optional, condition, Always));
    // Tags, Profile_Location and Profile_Name (#1553) come between them, as
    // in the addendum's table.
    let rows = super::color_link::metadata(object.color_link.as_ref())
        .chain(object.profile.metadata())
        .chain(trim_rows);
    with_rows(LIGHTING_OUTPUT_BASE, rows)
}

pub(super) fn for_binary_lighting_output_object(
    object: &BinaryLightingOutputObject,
) -> Cow<'_, [PropertyMetadata]> {
    let rows = super::color_link::metadata(object.color_link()).chain(object.profile().metadata());
    with_rows(BINARY_LIGHTING_OUTPUT_BASE, rows)
}

/// `base` with `rows` put in before its Property_List row, or `base` itself
/// when there are none.
fn with_rows(
    base: &'static [PropertyMetadata],
    rows: impl Iterator<Item = PropertyMetadata>,
) -> Cow<'static, [PropertyMetadata]> {
    let mut rows = rows.peekable();
    if rows.peek().is_none() {
        return Cow::Borrowed(base);
    }
    let (property_list, base) = base.split_last().expect("a Property_List row");
    let mut metadata = base.to_vec();
    metadata.extend(rows);
    metadata.push(*property_list);
    Cow::Owned(metadata)
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
            P::DEFAULT_RAMP_RATE,
            P::DEFAULT_STEP_INCREMENT,
            P::CURRENT_COMMAND_PRIORITY,
            P::COV_INCREMENT,
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
            P::DEFAULT_RAMP_RATE,
            P::DEFAULT_STEP_INCREMENT,
            P::CURRENT_COMMAND_PRIORITY,
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
            PropertyValue::ApplicationData(vec![0x09, 0x00])
        );
        assert_eq!(
            object
                .read_property(P::LIGHTING_COMMAND_DEFAULT_PRIORITY, None)
                .unwrap(),
            PropertyValue::Unsigned(16)
        );
        assert_eq!(
            object.read_property(P::DEFAULT_FADE_TIME, None).unwrap(),
            PropertyValue::Unsigned(100)
        );
        assert_eq!(
            object.read_property(P::DEFAULT_RAMP_RATE, None).unwrap(),
            PropertyValue::Real(100.0)
        );
        assert_eq!(
            object
                .read_property(P::DEFAULT_STEP_INCREMENT, None)
                .unwrap(),
            PropertyValue::Real(1.0)
        );
        // Nothing commands the new object, so Relinquish_Default is in effect.
        assert_eq!(
            object
                .read_property(P::CURRENT_COMMAND_PRIORITY, None)
                .unwrap(),
            PropertyValue::Null
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
        assert!(!object.is_array_property(P::CURRENT_COMMAND_PRIORITY));
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
            P::CURRENT_COMMAND_PRIORITY,
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
            P::CURRENT_COMMAND_PRIORITY,
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
        assert_eq!(
            object
                .read_property(P::CURRENT_COMMAND_PRIORITY, None)
                .unwrap(),
            PropertyValue::Null
        );
        assert!(object.is_array_property(P::PRIORITY_ARRAY));
        assert!(!object.is_array_property(P::BLINK_WARN_ENABLE));
        assert!(!object.is_array_property(P::CURRENT_COMMAND_PRIORITY));
    }

    #[test]
    fn property_metadata_lighting_duo_write_capabilities_match_dispatch() {
        // Constructor paired with the properties it must accept writes for.
        type WriteCase = (fn() -> Box<dyn BACnetObject>, &'static [P]);
        let cases: [WriteCase; 2] = [
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
                    P::RELINQUISH_DEFAULT,
                    P::DEFAULT_FADE_TIME,
                    P::DEFAULT_RAMP_RATE,
                    P::DEFAULT_STEP_INCREMENT,
                    P::COV_INCREMENT,
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
                    // Indexed reads remain valid even though Priority_Array is read-only.
                    let (value, index) = if p == P::PRIORITY_ARRAY {
                        let value = object.read_property(p, Some(8)).unwrap();
                        (value, Some(8))
                    } else if p == P::LIGHTING_COMMAND {
                        // It reads NONE until written, and NONE can't be
                        // written (Table 12-67), so write STOP instead.
                        (PropertyValue::ApplicationData(vec![0x09, 0x0A]), None)
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
                    PropertyValue::ApplicationData(vec![0x09, 0x0A]),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::LIGHTING_COMMAND, None).unwrap(),
                PropertyValue::ApplicationData(vec![0x09, 0x0A])
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
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
                (P::BLINK_WARN_ENABLE, PropertyValue::Enumerated(1)),
                (P::EGRESS_TIME, PropertyValue::Boolean(true)),
                (P::DEFAULT_FADE_TIME, PropertyValue::Real(100.0)),
                (P::DEFAULT_RAMP_RATE, PropertyValue::Unsigned(10)),
                (P::DEFAULT_STEP_INCREMENT, PropertyValue::Double(1.0)),
                (P::COV_INCREMENT, PropertyValue::Double(1.0)),
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
                P::CURRENT_COMMAND_PRIORITY,
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
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }

        // Optional table rows neither object implements.
        let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
        assert_unserved(&mut lo, P::TRANSITION);
        assert_unserved(&mut lo, P::FEEDBACK_VALUE);
        assert_unserved(&mut lo, P::POWER);
        let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        assert_unserved(&mut blo, P::FEEDBACK_VALUE);
        assert_unserved(&mut blo, P::POLARITY);
    }
}
