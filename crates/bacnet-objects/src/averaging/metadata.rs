use super::AveragingObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for Averaging (type 18, Clause 12.5 Table 12-5).
// Order preserves the legacy projection, with the window rows after
// Object_Property_Reference as in the table; PROPERTY_LIST is appended so the
// projection helper omits it while required_properties keeps it. Only
// implemented rows are described: table rows the object does not serve
// (timestamps, Variance_Value, audit, tags, profile rows) stay absent until
// dispatch exists. Attempted_Samples, Window_Interval and Window_Samples carry
// the table W code and Object_Property_Reference the R code; all four take
// network writes, each of which resets the sample window, so they are
// RequiredWrite/Always. Table 12-5 has no Present_Value, Status_Flags,
// Out_Of_Service, Reliability or Event_State, so there are no such rows
// (#1064 removed the rows the 0.1.0 import carried).
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MINIMUM_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MAXIMUM_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::AVERAGE_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ATTEMPTED_SAMPLES, RequiredWrite, None, Always),
    PropertyMetadata::new(P::VALID_SAMPLES, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_PROPERTY_REFERENCE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::WINDOW_INTERVAL, RequiredWrite, None, Always),
    PropertyMetadata::new(P::WINDOW_SAMPLES, RequiredWrite, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &AveragingObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;
    use std::collections::HashSet;

    /// The rows network writes reach. Every one but Description resets the
    /// sample window.
    const WRITABLE: [P; 5] = [
        P::DESCRIPTION,
        P::ATTEMPTED_SAMPLES,
        P::OBJECT_PROPERTY_REFERENCE,
        P::WINDOW_INTERVAL,
        P::WINDOW_SAMPLES,
    ];

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY
                    && code == expected),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_averaging_exact_sets_readable_rows_and_indexed_list() {
        let object = AveragingObject::new(1, "AVG-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::MINIMUM_VALUE,
            P::MAXIMUM_VALUE,
            P::AVERAGE_VALUE,
            P::ATTEMPTED_SAMPLES,
            P::VALID_SAMPLES,
            P::OBJECT_PROPERTY_REFERENCE,
            P::WINDOW_INTERVAL,
            P::WINDOW_SAMPLES,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::MINIMUM_VALUE,
            P::MAXIMUM_VALUE,
            P::AVERAGE_VALUE,
            P::ATTEMPTED_SAMPLES,
            P::VALID_SAMPLES,
            P::OBJECT_PROPERTY_REFERENCE,
            P::WINDOW_INTERVAL,
            P::WINDOW_SAMPLES,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), 13);
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
            let p = row.property_identifier;
            let expected = if p == P::DESCRIPTION {
                Optional
            } else if WRITABLE.contains(&p) {
                RequiredWrite
            } else {
                assert!(required.contains(&p), "{p:?}");
                RequiredRead
            };
            assert_eq!(row.conformance, expected, "{p:?}");
            object.read_property(p, None).unwrap();
        }
        // Table 12-5 has no Present_Value, Status_Flags, Out_Of_Service,
        // Reliability or Event_State row (#1064).
        for p in [
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::EVENT_STATE,
        ] {
            assert_error(
                object.read_property(p, None).unwrap_err(),
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_eq!(wire.len(), 9);
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(9)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [10, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_averaging_write_capabilities_match_dispatch() {
        let mut object = AveragingObject::new(1, "AVG-1").unwrap();
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
        // OBJECT_NAME has no network write route: a rename falls through
        // to WRITE_ACCESS_DENIED even with a well-formed value.
        assert!(!object.is_writable_property(P::OBJECT_NAME));
        assert_error(
            object
                .write_property(
                    P::OBJECT_NAME,
                    None,
                    PropertyValue::CharacterString("AVG-2".into()),
                    None,
                )
                .unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        // Zero is the only Attempted_Samples a client may write.
        assert_error(
            object
                .write_property(P::ATTEMPTED_SAMPLES, None, PropertyValue::Unsigned(1), None)
                .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        // The statistics scalars have no network write route.
        for p in [
            P::MINIMUM_VALUE,
            P::MAXIMUM_VALUE,
            P::AVERAGE_VALUE,
            P::VALID_SAMPLES,
        ] {
            let value = object.read_property(p, None).unwrap();
            assert_error(
                object.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert!(!object.is_writable_property(p));
        }
        // Description rejects a mistyped value without changing state.
        assert_error(
            object
                .write_property(P::DESCRIPTION, None, PropertyValue::Unsigned(1), None)
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        // Unserved Table 12-5 rows, and the rows the table doesn't define
        // (#1064), stay unknown on both paths.
        for p in [
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::EVENT_STATE,
            P::MINIMUM_VALUE_TIMESTAMP,
            P::MAXIMUM_VALUE_TIMESTAMP,
            P::VARIANCE_VALUE,
            P::ALL,
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
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
        assert_eq!(object.property_metadata().as_ref(), original);
    }
}
