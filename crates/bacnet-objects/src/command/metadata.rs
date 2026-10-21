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
// serve (Event_* detectors, Value_Source, audit, tags, profile rows) stay
// absent until dispatch exists. Action_Text is a per-instance row, present
// once the application sets it, read-only like Action.
// Present_Value carries the table W code and dispatch accepts an Unsigned up
// to the Action size, which starts that list (#1150), so it is
// RequiredWrite/Always. In_Process, All_Writes_Successful, and Action carry
// the table R code and have no network write route (Action writes are
// WRITE_ACCESS_DENIED), so they stay RequiredRead/ReadOnly rather than
// advertising a route write_property rejects. Status_Flags and Reliability
// carry the table O code; Description is Optional with a routed CharacterString
// write arm. Table 12-12 has no Out_Of_Service, so there is no such row (#1064).
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
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(object: &CommandObject) -> Cow<'_, [PropertyMetadata]> {
    if object.action_text.is_none() {
        return Cow::Borrowed(BASE);
    }
    let mut rows = BASE.to_vec();
    let after_action = rows
        .iter()
        .position(|row| row.property_identifier == P::ACTION)
        .expect("Action row")
        + 1;
    rows.insert(
        after_action,
        PropertyMetadata::new(P::ACTION_TEXT, Optional, None, ReadOnly),
    );
    Cow::Owned(rows)
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
                if class == ErrorClass::PROPERTY
                    && code == expected),
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
        assert_eq!(metadata.len(), 11);
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
        // Present_Value starts at zero, selecting no list.
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
        assert_eq!(wire.len(), 7);
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(7)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [8, u32::MAX] {
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
        let mut object = CommandObject::new(1, "CMD-1").unwrap();
        // Three empty lists, so Present_Value 3 is in range and completes at
        // once without queuing a run.
        object
            .set_action(vec![
                bacnet_types::constructed::BACnetActionList::default();
                3
            ])
            .unwrap();
        let original = object.property_metadata().into_owned();
        for row in &original {
            let p = row.property_identifier;
            let capability = match p {
                P::DESCRIPTION | P::PRESENT_VALUE => Always,
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
        // Present_Value takes an Unsigned up to the Action size and rejects
        // other types without changing state.
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
        // Description rejects a mistyped value without changing state.
        assert_error(
            object
                .write_property(P::DESCRIPTION, None, PropertyValue::Unsigned(1), None)
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        // Unserved rows stay unknown on read and write; Table 12-12 has no
        // Out_Of_Service at all (#1064).
        for p in [P::ACTION_TEXT, P::EVENT_STATE, P::OUT_OF_SERVICE] {
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

    #[test]
    fn property_metadata_command_action_text_row_follows_action_once_set() {
        let mut object = CommandObject::new(1, "CMD-1").unwrap();
        object
            .set_action(vec![
                bacnet_types::constructed::BACnetActionList::default();
                2
            ])
            .unwrap();
        object
            .set_action_text(vec!["Unoccupied".into(), "Occupied".into()])
            .unwrap();
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Owned(_)));
        let at = metadata
            .iter()
            .position(|row| row.property_identifier == P::ACTION_TEXT)
            .unwrap();
        assert_eq!(metadata[at - 1].property_identifier, P::ACTION);
        assert_eq!(metadata[at].conformance, Optional);
        assert_eq!(metadata[at].write_capability, ReadOnly);
        assert!(object.property_list().contains(&P::ACTION_TEXT));
        assert!(object.is_array_property(P::ACTION_TEXT));
        assert!(!object.is_writable_property(P::ACTION_TEXT));
        for index in [None, Some(1)] {
            assert_error(
                object
                    .write_property(
                        P::ACTION_TEXT,
                        index,
                        PropertyValue::CharacterString("x".into()),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
    }
}
