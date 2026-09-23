use super::CommandObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for Command (type 7, ASHRAE 135-2020 §12.10
// Table 12-12; printed pp. 215-216 / PDF pp. 217-218).
// Order preserves the legacy 11-property projection; PROPERTY_LIST is
// appended so the projection helper omits it while required_properties keeps
// it. Only implemented rows are described: table rows the object does not
// serve (Action_Text, Event_* detectors, Value_Source, audit, tags, profile
// rows) stay absent until dispatch exists.
// Present_Value carries the table W code and dispatch accepts Unsigned
// writes (stored verbatim with no action execution), so it is
// RequiredWrite/Always. In_Process, All_Writes_Successful, and Action carry
// the table R code and have no network write route (Action writes are
// WRITE_ACCESS_DENIED), so they stay RequiredRead/ReadOnly rather than
// advertising a route write_property rejects. Status_Flags and Reliability
// carry the table O code; Description is Optional with a routed CharacterString
// write arm; Out_Of_Service is unlisted but routed, so Optional/Always.
// Object_Name has no network write route (dispatch denies renames), so it is
// RequiredRead/ReadOnly.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::IN_PROCESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ALL_WRITES_SUCCESSFUL, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACTION, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &CommandObject) -> Cow<'_, [PropertyMetadata]> {
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

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_command_exact_sets_readable_rows_and_indexed_list() {
        let object = CommandObject::new(1, "CMD-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::IN_PROCESS,
            P::ALL_WRITES_SUCCESSFUL,
            P::ACTION,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::IN_PROCESS,
            P::ALL_WRITES_SUCCESSFUL,
            P::ACTION,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), 12);
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
            let expected = if row.property_identifier == P::PRESENT_VALUE {
                RequiredWrite
            } else if required.contains(&row.property_identifier) {
                RequiredRead
            } else {
                Optional
            };
            assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
            object.read_property(row.property_identifier, None).unwrap();
        }
        // Present_Value stores the written Unsigned verbatim; no action
        // execution, priority, or range handling exists on this route.
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        // Action reads back the whole list; the object arm ignores the array
        // index, so indexed reads return the same whole value.
        let whole = PropertyValue::List(vec![]);
        assert_eq!(object.read_property(P::ACTION, None).unwrap(), whole);
        assert!(object.is_array_property(P::ACTION));
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_eq!(wire.len(), 8);
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(8)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [9, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_command_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            let mut object = CommandObject::new(1, "CMD-1").unwrap();
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
                    P::DESCRIPTION | P::PRESENT_VALUE | P::OUT_OF_SERVICE => Always,
                    _ => ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    object.is_writable_property(p),
                    capability.is_writable(),
                    "{p:?}"
                );
                // Present_Value only accepts Unsigned; the other writable
                // rows round-trip their read-back value.
                let value = if p == P::PRESENT_VALUE {
                    PropertyValue::Unsigned(3)
                } else {
                    object.read_property(p, None).unwrap()
                };
                let result = object.write_property(p, None, value, None);
                if capability.is_writable() {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            // Present_Value stores Unsigned verbatim and rejects other types
            // without changing state.
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(3), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Unsigned(3)
            );
            assert_error(
                object
                    .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(1.0), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Unsigned(3)
            );
            // Action is network read-only even with a well-formed value.
            assert!(!object.is_writable_property(P::ACTION));
            assert_error(
                object
                    .write_property(
                        P::ACTION,
                        None,
                        PropertyValue::OctetString(vec![1, 2, 3]),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Object_Name has no network write route: a rename falls through
            // to WRITE_ACCESS_DENIED even with a well-formed value.
            assert!(!object.is_writable_property(P::OBJECT_NAME));
            assert_error(
                object
                    .write_property(
                        P::OBJECT_NAME,
                        None,
                        PropertyValue::CharacterString("CMD-2".into()),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Table-required scalars without a write arm stay denied.
            for p in [P::IN_PROCESS, P::ALL_WRITES_SUCCESSFUL] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
            }
            // Description and Out_Of_Service reject mistyped values without
            // changing state.
            for (p, value) in [
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Unserved rows stay unknown on read and denied on write.
            for p in [P::ACTION_TEXT, P::EVENT_STATE] {
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
