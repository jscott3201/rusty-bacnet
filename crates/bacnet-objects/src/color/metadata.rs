use super::{ColorObject, ColorTemperatureObject};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for the Color pair (dispatch-first; table codes
// unverifiable from the local sources):
// - Color (type 63) and Color Temperature (type 64) carry the color module
//   header anchor "ASHRAE 135-2020 Addendum bj, Clauses 12.55-12.56", which is
//   unverified in the local PDF (whose Table of Contents already assigns
//   12.55 to Binary Lighting Output and 12.56 to Network Port) and the
//   addendum text is not in scope. The color module header is left stale by
//   directive; no clause or table conformance code below is claimed from that
//   anchor. Conformance codes instead mirror the Accumulator/Elevator heritage
//   pattern: served rows the dispatch treats as core state carry the R-like
//   RequiredRead code, secondary tuning rows carry the O-like Optional code,
//   and writability mirrors the write arms exactly (Elevator precedent: a
//   served row with a routed arm is RequiredRead/Always, never an invented
//   RequiredWrite without a table basis).
// Order preserves each legacy projection; PROPERTY_LIST is appended so the
// projection helper omits it while required_properties keeps it. Only
// implemented rows are described: table rows the objects do not serve stay
// absent until dispatch exists.
// Object_Identifier, Object_Name, and Object_Type carry RequiredRead/ReadOnly
// (no network write route). Object_Name explicitly documents the denial: a
// rename falls through to WRITE_ACCESS_DENIED (neither object has a
// write_object_name arm). Description carries Optional with the routed
// CharacterString arm, so Optional/Always. Out_Of_Service carries
// RequiredRead with the routed Boolean arm, so RequiredRead/Always.
// Color Present_Value has no write arm (non-commandable, no priority array),
// so RequiredRead/ReadOnly. Color Color_Command (opaque OctetString arm) and
// Default_Fade_Time (Unsigned arm rejecting values above 86_400_000) are
// served rows with routed arms, so RequiredRead/Always.
// Color Temperature Present_Value carries the routed Unsigned arm
// (u64_to_u32 plus the min/max clamp, with no Out_Of_Service gate), so
// RequiredRead/Always; Color_Command likewise RequiredRead/Always.
// Default_Fade_Time on Color Temperature has no write arm — the asymmetry
// versus Color is preserved, not reconciled — so Optional/ReadOnly alongside
// the other secondary rows (Default_Color, Default_Color_Temperature,
// Default_Ramp_Rate, Default_Step_Increment, Reliability).
// Tracking_Value, Transition, In_Progress, Event_State, Status_Flags, and
// Min/Max_Pres_Value are heritage-R-like served rows with no write route, so
// RequiredRead/ReadOnly. Min/Max_Pres_Value read unknown while None, but the
// constructor sets both and no clearing API exists, so they are always
// readable in practice and stay static borrowed rows.
// Writability is Always, never WhenOutOfService: dispatch routes every write
// arm unconditionally (Color Temperature Present_Value included) and the
// suites pin in-service writes, so the metadata mirrors dispatch. Presence is
// None throughout: the implementation models no commandable,
// intrinsic-reporting, or paired-text gating on this family.
// The pair is not createable at runtime (the network factory builds only the
// eight analog/binary/multi-state input/output/value types, so the
// is_createable=false default holds) and remains deleteable (delete denies
// only Device and NetworkPort, so the is_deleteable=true default holds);
// neither needs an override. Array gating keeps the default: Property_List
// admits an index (BACnetARRAY) while every other served row rejects one. COV
// keeps its overrides: supports_cov=true on both objects, and the COV gating
// path (read_property plus supports_cov_property to supports_cov) never
// consults metadata.
const COLOR_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::TRACKING_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::COLOR_COMMAND, RequiredRead, None, Always),
    PropertyMetadata::new(P::IN_PROGRESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DEFAULT_COLOR, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DEFAULT_FADE_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::TRANSITION, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const COLOR_TEMPERATURE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::TRACKING_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::COLOR_COMMAND, RequiredRead, None, Always),
    PropertyMetadata::new(P::IN_PROGRESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DEFAULT_COLOR_TEMPERATURE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DEFAULT_FADE_TIME, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DEFAULT_RAMP_RATE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DEFAULT_STEP_INCREMENT, Optional, None, ReadOnly),
    PropertyMetadata::new(P::TRANSITION, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MIN_PRES_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MAX_PRES_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_color_object(_object: &ColorObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(COLOR_BASE)
}

pub(super) fn for_color_temperature_object(
    _object: &ColorTemperatureObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(COLOR_TEMPERATURE_BASE)
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
                RequiredRead
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
    fn property_metadata_color_exact_sets_readable_rows_and_indexed_list() {
        let object = ColorObject::new(1, "CLR-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::COLOR_COMMAND,
            P::IN_PROGRESS,
            P::DEFAULT_COLOR,
            P::DEFAULT_FADE_TIME,
            P::TRANSITION,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::COLOR_COMMAND,
            P::IN_PROGRESS,
            P::DEFAULT_FADE_TIME,
            P::TRANSITION,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        // BACnetxyColor reads back as a two-REAL list at the D65 default.
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::List(vec![
                PropertyValue::Real(0.3127),
                PropertyValue::Real(0.3290),
            ])
        );
        assert_eq!(
            object.read_property(P::TRACKING_VALUE, None).unwrap(),
            PropertyValue::List(vec![
                PropertyValue::Real(0.3127),
                PropertyValue::Real(0.3290),
            ])
        );
        assert_eq!(
            object.read_property(P::DEFAULT_COLOR, None).unwrap(),
            PropertyValue::List(vec![
                PropertyValue::Real(0.3127),
                PropertyValue::Real(0.3290),
            ])
        );
        assert_eq!(
            object.read_property(P::COLOR_COMMAND, None).unwrap(),
            PropertyValue::OctetString(vec![])
        );
        // The xy lists are BACnetLIST-style productions, so an index is
        // rejected; Present_Value is scalar and likewise rejects one.
        assert!(!object.is_array_property(P::PRESENT_VALUE));
        assert!(!object.is_array_property(P::TRACKING_VALUE));
        assert!(!object.is_array_property(P::COLOR_COMMAND));
    }

    #[test]
    fn property_metadata_color_temperature_exact_sets_readable_rows_and_indexed_list() {
        let object = ColorTemperatureObject::new(1, "CT-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::COLOR_COMMAND,
            P::IN_PROGRESS,
            P::DEFAULT_COLOR_TEMPERATURE,
            P::DEFAULT_FADE_TIME,
            P::DEFAULT_RAMP_RATE,
            P::DEFAULT_STEP_INCREMENT,
            P::TRANSITION,
            P::MIN_PRES_VALUE,
            P::MAX_PRES_VALUE,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TRACKING_VALUE,
            P::COLOR_COMMAND,
            P::IN_PROGRESS,
            P::TRANSITION,
            P::MIN_PRES_VALUE,
            P::MAX_PRES_VALUE,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Unsigned(4000)
        );
        assert_eq!(
            object.read_property(P::TRACKING_VALUE, None).unwrap(),
            PropertyValue::Unsigned(4000)
        );
        // The constructor bounds are always readable in practice (no
        // clearing API exists to reach the None arm).
        assert_eq!(
            object.read_property(P::MIN_PRES_VALUE, None).unwrap(),
            PropertyValue::Unsigned(1000)
        );
        assert_eq!(
            object.read_property(P::MAX_PRES_VALUE, None).unwrap(),
            PropertyValue::Unsigned(30000)
        );
        assert_eq!(
            object
                .read_property(P::DEFAULT_COLOR_TEMPERATURE, None)
                .unwrap(),
            PropertyValue::Unsigned(4000)
        );
        assert!(!object.is_array_property(P::PRESENT_VALUE));
        assert!(!object.is_array_property(P::COLOR_COMMAND));
    }

    #[test]
    fn property_metadata_color_pair_write_capabilities_match_dispatch() {
        let cases: [(fn() -> Box<dyn BACnetObject>, &[P]); 2] = [
            (
                || Box::new(ColorObject::new(1, "CLR-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::COLOR_COMMAND,
                    P::DEFAULT_FADE_TIME,
                ],
            ),
            (
                || Box::new(ColorTemperatureObject::new(1, "CT-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::PRESENT_VALUE,
                    P::COLOR_COMMAND,
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
                    let value = object.read_property(p, None).unwrap();
                    let result = object.write_property(p, None, value, None);
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
    fn property_metadata_color_writes_store_verbatim_with_fade_time_gate() {
        for out_of_service in [false, true] {
            let mut object = ColorObject::new(1, "CLR-1").unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            // Color_Command stores the opaque OctetString verbatim.
            let command = PropertyValue::OctetString(vec![0x01, 0x02, 0x03]);
            object
                .write_property(P::COLOR_COMMAND, None, command.clone(), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::COLOR_COMMAND, None).unwrap(),
                command
            );
            // Default_Fade_Time stores Unsigned verbatim up to 86_400_000.
            object
                .write_property(
                    P::DEFAULT_FADE_TIME,
                    None,
                    PropertyValue::Unsigned(86_400_000),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::DEFAULT_FADE_TIME, None).unwrap(),
                PropertyValue::Unsigned(86_400_000)
            );
            assert_error(
                object
                    .write_property(
                        P::DEFAULT_FADE_TIME,
                        None,
                        PropertyValue::Unsigned(86_400_001),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(
                object.read_property(P::DEFAULT_FADE_TIME, None).unwrap(),
                PropertyValue::Unsigned(86_400_000)
            );
            // Mistyped values are rejected without changing state.
            for (p, value) in [
                (P::COLOR_COMMAND, PropertyValue::Unsigned(1)),
                (P::DEFAULT_FADE_TIME, PropertyValue::Real(1000.0)),
                (P::DESCRIPTION, PropertyValue::Null),
                (P::OUT_OF_SERVICE, PropertyValue::Null),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Present_Value is non-commandable: even its own readback is
            // denied on write, in and out of service.
            assert!(!object.is_writable_property(P::PRESENT_VALUE));
            let present = object.read_property(P::PRESENT_VALUE, None).unwrap();
            assert_error(
                object
                    .write_property(P::PRESENT_VALUE, None, present, None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Rows with no network write route deny even their readback.
            for p in [
                P::TRACKING_VALUE,
                P::DEFAULT_COLOR,
                P::TRANSITION,
                P::IN_PROGRESS,
                P::STATUS_FLAGS,
                P::EVENT_STATE,
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
    fn property_metadata_color_temperature_writes_clamp_to_min_max() {
        for out_of_service in [false, true] {
            let mut object = ColorTemperatureObject::new(1, "CT-1").unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            // Boundary values round-trip in both states: no OOS gate.
            for value in [1000, 30000] {
                object
                    .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(value), None)
                    .unwrap();
                assert_eq!(
                    object.read_property(P::PRESENT_VALUE, None).unwrap(),
                    PropertyValue::Unsigned(value)
                );
                assert_eq!(
                    object.read_property(P::TRACKING_VALUE, None).unwrap(),
                    PropertyValue::Unsigned(value)
                );
            }
            // Outside the configured bounds the write is refused and both
            // readbacks keep the last stored value.
            for value in [999, 30001, u32::MAX as u64] {
                assert_error(
                    object
                        .write_property(
                            P::PRESENT_VALUE,
                            None,
                            PropertyValue::Unsigned(value),
                            None,
                        )
                        .unwrap_err(),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
            }
            // Over-wide values that do not fit u32 are refused the same way.
            assert_error(
                object
                    .write_property(
                        P::PRESENT_VALUE,
                        None,
                        PropertyValue::Unsigned(0x1_0000_03E8),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Unsigned(30000)
            );
            // Color_Command stores the opaque OctetString verbatim.
            let command = PropertyValue::OctetString(vec![0x04, 0x05]);
            object
                .write_property(P::COLOR_COMMAND, None, command.clone(), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::COLOR_COMMAND, None).unwrap(),
                command
            );
            // Mistyped values are rejected without changing state.
            for (p, value) in [
                (P::PRESENT_VALUE, PropertyValue::Real(4000.0)),
                (P::COLOR_COMMAND, PropertyValue::Unsigned(1)),
                (P::DESCRIPTION, PropertyValue::Null),
                (P::OUT_OF_SERVICE, PropertyValue::Null),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Default rows have no network write route: even their readback
            // is denied on write (the Default_Fade_Time asymmetry versus
            // Color is preserved, not reconciled).
            for p in [
                P::DEFAULT_COLOR_TEMPERATURE,
                P::DEFAULT_FADE_TIME,
                P::DEFAULT_RAMP_RATE,
                P::DEFAULT_STEP_INCREMENT,
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
    fn property_metadata_color_pair_unserved_rows_stay_unknown() {
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

        // Neither object serves priority, command-value, or device rows.
        let mut color = ColorObject::new(1, "CLR-1").unwrap();
        assert_unserved(&mut color, P::PRIORITY_ARRAY);
        assert_unserved(&mut color, P::COV_INCREMENT);
        assert_unserved(&mut color, P::DEVICE_TYPE);
        let mut temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
        assert_unserved(&mut temperature, P::PRIORITY_ARRAY);
        assert_unserved(&mut temperature, P::COV_INCREMENT);
        assert_unserved(&mut temperature, P::DEVICE_TYPE);
    }
}
