use super::LoadControlObject;
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for Load Control (type 28, ASHRAE 135-2020 §12.28
// Table 12-32; printed p. 344 / PDF p. 346, PDF = printed + 2; property
// descriptions PDF pp. 347-349).
// Order keeps the earlier rows' order; the rows served since #2464 follow
// Event_State in table order, and PROPERTY_LIST comes last. Only implemented
// rows are described: the optional Full_Duty_Baseline and State_Description,
// and the event/audit/tags/value-source/profile rows, stay absent.
// Object_Identifier, Object_Name, Object_Type, Present_Value,
// Expected_Shed_Level, Actual_Shed_Level, Status_Flags, Event_State and
// Shed_Level_Descriptions carry the table R code and have no network write
// route, so RequiredRead/ReadOnly. Description and Reliability carry the
// table O code; Description has a CharacterString write arm (Optional/Always)
// and Reliability none (Optional/ReadOnly). Table 12-32 has no
// Out_Of_Service, so there is no such row (#1064). Requested_Shed_Level (one
// BACnetShedLevel CHOICE, #1133), Start_Time, Shed_Duration, Duty_Window,
// Enable and Shed_Levels carry the table W code and each has its write arm,
// so RequiredWrite/Always; Shed_Levels takes its elements but not a new size
// (the table's footnote 1).
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
    PropertyMetadata::new(P::START_TIME, RequiredWrite, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DUTY_WINDOW, RequiredWrite, None, Always),
    PropertyMetadata::new(P::ENABLE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::SHED_LEVELS, RequiredWrite, None, Always),
    PropertyMetadata::new(P::SHED_LEVEL_DESCRIPTIONS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &LoadControlObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::HashSet;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::{Error, RemoteError};
    use bacnet_types::primitives::{Date, PropertyValue, Time};

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY
                    && code == expected),
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

    /// The rows a client writes: Table 12-32's W rows, and Description.
    const WRITABLE: [P; 7] = [
        P::DESCRIPTION,
        P::REQUESTED_SHED_LEVEL,
        P::SHED_DURATION,
        P::START_TIME,
        P::DUTY_WINDOW,
        P::ENABLE,
        P::SHED_LEVELS,
    ];

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
            P::RELIABILITY,
            P::EVENT_STATE,
            P::DUTY_WINDOW,
            P::ENABLE,
            P::SHED_LEVELS,
            P::SHED_LEVEL_DESCRIPTIONS,
        ];
        // Table 12-32's R and W rows, Status_Flags among them.
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
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::DUTY_WINDOW,
            P::ENABLE,
            P::SHED_LEVELS,
            P::SHED_LEVEL_DESCRIPTIONS,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), 18);
        assert_eq!(
            crate::test_rows::listed(&object.property_metadata()),
            crate::test_rows::listed_from(&all)
        );
        assert_eq!(
            crate::test_rows::required(&object.property_metadata()),
            required
        );
        assert_eq!(
            metadata
                .iter()
                .map(|row| row.property_identifier)
                .collect::<HashSet<_>>()
                .len(),
            metadata.len()
        );
        assert!(crate::creation::builtin_deleteable(
            object.object_identifier().object_type()
        ));
        // Table 13-1 has a Load Control row (#1061).
        assert!(object.cov().is_some_and(|cov| cov.supports_subscribe_cov()));
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            let p = row.property_identifier;
            let expected = if WRITABLE.contains(&p) && p != P::DESCRIPTION {
                RequiredWrite
            } else if required.contains(&p) {
                RequiredRead
            } else {
                Optional
            };
            assert_eq!(row.conformance, expected, "{p:?}");
            object.read_property(p, None).unwrap();
        }
        // Default readbacks pin the pure value stores.
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        // The shed levels start at the LEVEL choice's default, level [1] 0.
        for p in [
            P::REQUESTED_SHED_LEVEL,
            P::EXPECTED_SHED_LEVEL,
            P::ACTUAL_SHED_LEVEL,
        ] {
            assert_eq!(
                object.read_property(p, None).unwrap(),
                PropertyValue::ApplicationData(vec![0x19, 0x00])
            );
        }
        for p in [P::SHED_DURATION, P::DUTY_WINDOW] {
            assert_eq!(
                object.read_property(p, None).unwrap(),
                PropertyValue::Unsigned(0)
            );
        }
        assert_eq!(
            object.read_property(P::START_TIME, None).unwrap(),
            unspec_start_time()
        );
        assert_eq!(
            object.read_property(P::ENABLE, None).unwrap(),
            PropertyValue::Boolean(true)
        );
        for p in [P::SHED_LEVELS, P::SHED_LEVEL_DESCRIPTIONS] {
            assert!(crate::property_metadata::is_array_property(&object, p));
            assert_eq!(
                object.read_property(p, None).unwrap(),
                PropertyValue::List(vec![])
            );
            assert_eq!(
                object.read_property(p, Some(0)).unwrap(),
                PropertyValue::Unsigned(0)
            );
        }
        assert_eq!(
            object.read_property(P::EVENT_STATE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_eq!(wire.len(), 14);
        assert!(crate::property_metadata::is_array_property(
            &object,
            P::PROPERTY_LIST
        ));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(14)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [15, u32::MAX] {
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
        let mut object = LoadControlObject::new(1, "LC-1").unwrap();
        let original = object.property_metadata().into_owned();
        for row in &original {
            let p = row.property_identifier;
            let capability = if WRITABLE.contains(&p) {
                Always
            } else {
                ReadOnly
            };
            assert_eq!(row.write_capability, capability, "{p:?}");
            assert_eq!(
                crate::property_metadata::is_writable_property(&object, p),
                capability.is_writable(),
                "{p:?}"
            );
            // Writing back what reads succeeds on every writable row, the
            // unspecified Start_Time and the empty Shed_Levels included.
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
        // Shed_Level_Descriptions is the embedder's: clients only read it.
        object.set_shed_levels(vec![(1, "lights".into())]).unwrap();
        assert_error(
            object
                .write_property(
                    P::SHED_LEVEL_DESCRIPTIONS,
                    Some(1),
                    PropertyValue::CharacterString("other".into()),
                    None,
                )
                .unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        // Description rejects a mistyped value without changing state.
        assert_error(
            object
                .write_property(P::DESCRIPTION, None, PropertyValue::Unsigned(1), None)
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        // Unserved rows stay unknown on read and on write: Table 12-32 has
        // no Out_Of_Service at all (#1064), and the object leaves out the
        // optional Full_Duty_Baseline and State_Description.
        for p in [
            P::OUT_OF_SERVICE,
            P::FULL_DUTY_BASELINE,
            P::STATE_DESCRIPTION,
        ] {
            assert!(!crate::property_metadata::is_writable_property(&object, p));
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
        assert_eq!(object.property_metadata().as_ref(), original);
    }
}
