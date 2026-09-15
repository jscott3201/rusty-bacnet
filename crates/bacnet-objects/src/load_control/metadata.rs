use super::LoadControlObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for Load Control (type 28, ASHRAE 135-2020 §12.28
// Table 12-32; printed p. 344 / PDF p. 346, PDF = printed + 2; property
// descriptions PDF pp. 347-349).
// Order preserves the legacy 13-property projection; EVENT_STATE (readable
// but unlisted) and PROPERTY_LIST are appended so the projection helper omits
// PROPERTY_LIST while required_properties keeps it. Only implemented rows are
// described: table rows the object does not serve (Duty_Window, Enable,
// Full_Duty_Baseline, Shed_Levels, Shed_Level_Descriptions, State_Description,
// event/audit/tags/value-source/profile rows) stay absent until dispatch
// exists. Enable has no PropertyIdentifier constant so no row can be emitted.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Description is
// Optional with a routed CharacterString write arm, so Optional/Always.
// Present_Value, Expected_Shed_Level, and Actual_Shed_Level carry the table R
// code and have no network write route, so RequiredRead/ReadOnly.
// Requested_Shed_Level carries the table W code and dispatch accepts List
// with one Unsigned (percent) or finite Real (amount), so
// RequiredWrite/Always. Shed_Duration carries the table W code and dispatch
// accepts Unsigned, so RequiredWrite/Always. Start_Time carries the table W
// code but dispatch has no write arm, so the row mirrors dispatch as
// RequiredRead/ReadOnly rather than advertising a route write_property
// rejects (Averaging Attempted_Samples precedent). Status_Flags and
// Reliability carry the table O code, so Optional/ReadOnly. Out_Of_Service is
// Optional with a routed Boolean write arm, so Optional/Always. Event_State
// carries the table R code and is readable but was absent from the legacy
// list, so RequiredRead/ReadOnly and appended to the projection.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::REQUESTED_SHED_LEVEL, RequiredWrite, None, Always),
    PropertyMetadata::new(P::EXPECTED_SHED_LEVEL, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACTUAL_SHED_LEVEL, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SHED_DURATION, RequiredWrite, None, Always),
    PropertyMetadata::new(P::START_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &LoadControlObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{Date, PropertyValue, Time};
    use std::collections::HashSet;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    fn unspec_start_time() -> PropertyValue {
        PropertyValue::List(vec![
            PropertyValue::Date(Date {
                year: 0xFF,
                month: 0xFF,
                day: 0xFF,
                day_of_week: 0xFF,
            }),
            PropertyValue::Time(Time {
                hour: 0xFF,
                minute: 0xFF,
                second: 0xFF,
                hundredths: 0xFF,
            }),
        ])
    }

    #[test]
    fn property_metadata_load_control_exact_sets_readable_rows_and_indexed_list() {
        let object = LoadControlObject::new(1, "LC-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::REQUESTED_SHED_LEVEL,
            P::EXPECTED_SHED_LEVEL,
            P::ACTUAL_SHED_LEVEL,
            P::SHED_DURATION,
            P::START_TIME,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::EVENT_STATE,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::REQUESTED_SHED_LEVEL,
            P::EXPECTED_SHED_LEVEL,
            P::ACTUAL_SHED_LEVEL,
            P::SHED_DURATION,
            P::START_TIME,
            P::EVENT_STATE,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), 15);
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
        assert!(!object.supports_cov());
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            let expected = if row.property_identifier == P::REQUESTED_SHED_LEVEL
                || row.property_identifier == P::SHED_DURATION
            {
                RequiredWrite
            } else if required.contains(&row.property_identifier) {
                RequiredRead
            } else {
                Optional
            };
            assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
            object.read_property(row.property_identifier, None).unwrap();
        }
        // Default readbacks pin the pure value stores.
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::REQUESTED_SHED_LEVEL, None).unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(0)])
        );
        assert_eq!(
            object.read_property(P::EXPECTED_SHED_LEVEL, None).unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(0)])
        );
        assert_eq!(
            object.read_property(P::ACTUAL_SHED_LEVEL, None).unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(0)])
        );
        assert_eq!(
            object.read_property(P::SHED_DURATION, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object.read_property(P::START_TIME, None).unwrap(),
            unspec_start_time()
        );
        assert_eq!(
            object.read_property(P::EVENT_STATE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_eq!(wire.len(), 11);
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(11)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [12, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_load_control_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            let mut object = LoadControlObject::new(1, "LC-1").unwrap();
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
                let capability = match p {
                    P::DESCRIPTION
                    | P::REQUESTED_SHED_LEVEL
                    | P::SHED_DURATION
                    | P::OUT_OF_SERVICE => Always,
                    _ => ReadOnly,
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
            // Object_Name has no network write route: a rename falls through
            // to WRITE_ACCESS_DENIED even with a well-formed value.
            assert!(!object.is_writable_property(P::OBJECT_NAME));
            assert_error(
                object
                    .write_property(
                        P::OBJECT_NAME,
                        None,
                        PropertyValue::CharacterString("LC-2".into()),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Present_Value, Expected_Shed_Level, and Actual_Shed_Level have
            // no network write route.
            for p in [
                P::PRESENT_VALUE,
                P::EXPECTED_SHED_LEVEL,
                P::ACTUAL_SHED_LEVEL,
            ] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
            }
            // Start_Time carries the table W code but dispatch has no write
            // arm (not even a schedule update), so even the read-back
            // Date/Time list is denied and the row stays ReadOnly.
            assert!(!object.is_writable_property(P::START_TIME));
            assert_error(
                object
                    .write_property(P::START_TIME, None, unspec_start_time(), None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Requested_Shed_Level accepts a single-element Unsigned
            // (percent) or finite Real (amount) list and stores Percent or
            // Amount; a bare Unsigned is the wrong datatype.
            object
                .write_property(
                    P::REQUESTED_SHED_LEVEL,
                    None,
                    PropertyValue::List(vec![PropertyValue::Unsigned(50)]),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::REQUESTED_SHED_LEVEL, None).unwrap(),
                PropertyValue::List(vec![PropertyValue::Unsigned(50)])
            );
            object
                .write_property(
                    P::REQUESTED_SHED_LEVEL,
                    None,
                    PropertyValue::List(vec![PropertyValue::Real(25.5)]),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::REQUESTED_SHED_LEVEL, None).unwrap(),
                PropertyValue::List(vec![PropertyValue::Real(25.5)])
            );
            assert_error(
                object
                    .write_property(
                        P::REQUESTED_SHED_LEVEL,
                        None,
                        PropertyValue::Unsigned(50),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            // Shed_Duration stores Unsigned verbatim and rejects other types
            // without changing state.
            object
                .write_property(P::SHED_DURATION, None, PropertyValue::Unsigned(3600), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::SHED_DURATION, None).unwrap(),
                PropertyValue::Unsigned(3600)
            );
            assert_error(
                object
                    .write_property(P::SHED_DURATION, None, PropertyValue::Real(1.0), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            assert_eq!(
                object.read_property(P::SHED_DURATION, None).unwrap(),
                PropertyValue::Unsigned(3600)
            );
            // Description and Out_Of_Service reject mistyped values without
            // changing state.
            for (p, value) in [
                (P::DESCRIPTION, PropertyValue::Null),
                (P::OUT_OF_SERVICE, PropertyValue::Null),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Unserved table rows stay unknown on read and denied on write.
            // Enable has no PropertyIdentifier constant so no row can be
            // emitted; the remaining unserved Load Control rows pin the
            // exclusion.
            for p in [
                P::DUTY_WINDOW,
                P::SHED_LEVELS,
                P::FULL_DUTY_BASELINE,
                P::SHED_LEVEL_DESCRIPTIONS,
                P::STATE_DESCRIPTION,
            ] {
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
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }
}
