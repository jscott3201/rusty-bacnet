use super::CalendarObject;
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Only rows Clause 12.9 Table 12-11 defines. The table has no Status_Flags,
// Event_State, Out_Of_Service or Reliability, so Calendar serves none of them
// (#984). Present_Value stays application-managed. Description and Date_List
// are network-writable; Date_List takes every BACnetCalendarEntry choice, as
// the clause requires of a writable Date_List (#996).
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DATE_LIST, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &CalendarObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::HashSet;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::{Error, RemoteError};
    use bacnet_types::primitives::PropertyValue;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY
                    && code == expected),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_calendar_exact_sets_readable_rows_and_indexed_list() {
        let object = CalendarObject::new(1, "CAL-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::DATE_LIST,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::DATE_LIST,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), 7);
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
            assert_eq!(
                row.conformance,
                if required.contains(&row.property_identifier) {
                    RequiredRead
                } else {
                    Optional
                }
            );
            object.read_property(row.property_identifier, None).unwrap();
        }
        let wire: Vec<_> = [P::DESCRIPTION, P::PRESENT_VALUE, P::DATE_LIST]
            .iter()
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(3)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [4, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_calendar_write_capabilities_match_dispatch() {
        let mut object = CalendarObject::new(1, "CAL-1").unwrap();
        object.set_description("original");
        let metadata = object.property_metadata().into_owned();
        for row in &metadata {
            let p = row.property_identifier;
            let writable = matches!(p, P::DESCRIPTION | P::DATE_LIST);
            assert_eq!(
                row.write_capability,
                if writable { Always } else { ReadOnly }
            );
            assert_eq!(object.is_writable_property(p), writable, "{p:?}");
            let before = object.read_property(p, None).unwrap();
            let result = object.write_property(p, None, before.clone(), None);
            if writable {
                result.unwrap();
            } else {
                assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            assert_eq!(object.read_property(p, None).unwrap(), before);
        }
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("updated".into()),
                None,
            )
            .unwrap();
        assert_error(
            object
                .write_property(P::DESCRIPTION, None, PropertyValue::Unsigned(1), None)
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        object
            .write_property(P::DESCRIPTION, None, PropertyValue::Null, None)
            .unwrap();
        assert_eq!(
            object.read_property(P::DESCRIPTION, None).unwrap(),
            PropertyValue::CharacterString("updated".into())
        );
        for index in [None, Some(0), Some(1), Some(u32::MAX)] {
            assert_error(
                object
                    .write_property(P::PRESENT_VALUE, index, PropertyValue::Null, Some(8))
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        // Date_List is a BACnetLIST of calendar entries: no index, no NULL.
        for index in [Some(0), Some(1), Some(u32::MAX)] {
            assert_error(
                object
                    .write_property(P::DATE_LIST, index, PropertyValue::List(vec![]), None)
                    .unwrap_err(),
                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            );
        }
        assert_error(
            object
                .write_property(P::DATE_LIST, None, PropertyValue::Null, Some(8))
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        // Table 12-11 defines none of these, so reads and writes alike report
        // the property unknown rather than read-only.
        for p in [
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
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
        assert_eq!(object.property_metadata().as_ref(), metadata);
    }
}
