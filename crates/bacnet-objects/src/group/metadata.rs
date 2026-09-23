use super::{GlobalGroupObject, GroupObject, StructuredViewObject};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for the Group trio (ASHRAE 135-2020; PDF = printed + 2):
// - Group (type 11, §12.14 Table 12-17; printed pp. 243-244 / PDF pp. 245-246)
// - GlobalGroup (type 26, §12.50 Table 12-57; printed pp. 490-492 / PDF pp. 492-494)
// - StructuredView (type 29, §12.29 Table 12-34; printed p. 351 / PDF pp. 353-355)
// Order preserves each legacy projection; PROPERTY_LIST is appended so the
// projection helper omits it while required_properties keeps it. Only
// implemented rows are described: table rows the objects do not serve (Group
// audit/tag/profile rows; GlobalGroup event/COV/audit/tag/profile rows,
// including Event_State and Member_Status_Flags; StructuredView subordinate
// tags/relationships rows) stay absent until dispatch exists.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED. Description is Optional with a routed CharacterString
// write arm, so Optional/Always. List_Of_Group_Members, Group_Members,
// Present_Value, Group_Member_Names (Table 12-57 O), Node_Type, Node_Subtype
// (Table 12-34 O), Subordinate_List, and Subordinate_Annotations
// (Table 12-34 O) have no network write route — group values are populated
// locally, never commanded — so table-code conformance with ReadOnly; the R
// rows are RequiredRead and the three O rows are Optional.
// Status_Flags, Out_Of_Service, and Reliability are served on all three
// symbols through the shared common read arms. Table codes map faithfully
// (DESCRIPTION O → Optional precedent): GlobalGroup Table 12-57 lists
// Status_Flags R, Out_Of_Service R, and Reliability O, so served-but-optional
// rows stay Optional and readable — GlobalGroup Reliability is
// Optional/ReadOnly. Group Table 12-17 and StructuredView Table 12-34 carry
// no such rows, so the NetworkPort precedent for served non-table rows keeps
// Group and StructuredView Reliability at RequiredRead/ReadOnly; likewise
// Status_Flags stays RequiredRead/ReadOnly and Out_Of_Service stays
// RequiredRead/Always with its routed Boolean write arm.
// Presence is None throughout: the implementation models no commandable,
// intrinsic-reporting, or paired-text gating on this family.
// The trio is not createable at runtime (the network factory builds only the
// eight analog/binary/multi-state input/output/value types, so the
// is_createable=false default holds) and remains deleteable (delete denies
// only Device and NetworkPort, so the is_deleteable=true default holds);
// neither needs an override. Array gating, writability, and COV also keep
// their defaults: Present_Value admits an index only on GlobalGroup
// (BACnetARRAY per Table 12-57) and rejects it on Group (BACnetLIST per
// Table 12-17); List_Of_Group_Members stays index-rejecting while
// Group_Members, Group_Member_Names, Subordinate_List, and
// Subordinate_Annotations admit one.
const GROUP_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LIST_OF_GROUP_MEMBERS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const GLOBAL_GROUP_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GROUP_MEMBERS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GROUP_MEMBER_NAMES, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const STRUCTURED_VIEW_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NODE_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NODE_SUBTYPE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::SUBORDINATE_LIST, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SUBORDINATE_ANNOTATIONS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_group_object(_object: &GroupObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(GROUP_BASE)
}

pub(super) fn for_global_group_object(_object: &GlobalGroupObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(GLOBAL_GROUP_BASE)
}

pub(super) fn for_structured_view_object(
    _object: &StructuredViewObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(STRUCTURED_VIEW_BASE)
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
        assert!(!object.supports_cov());
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
    fn property_metadata_group_exact_sets_readable_rows_and_indexed_list() {
        let object = GroupObject::new(1, "G-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::LIST_OF_GROUP_MEMBERS,
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::LIST_OF_GROUP_MEMBERS,
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        // Default member stores read back empty.
        assert_eq!(
            object
                .read_property(P::LIST_OF_GROUP_MEMBERS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::List(vec![])
        );
        // Group Present_Value is a BACnetLIST (Table 12-17), so an index is
        // rejected; GlobalGroup admits one (Table 12-57 BACnetARRAY).
        assert!(!object.is_array_property(P::PRESENT_VALUE));
        assert!(!object.is_array_property(P::LIST_OF_GROUP_MEMBERS));
    }

    #[test]
    fn property_metadata_global_group_exact_sets_readable_rows_and_indexed_list() {
        let object = GlobalGroupObject::new(1, "GG-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::GROUP_MEMBERS,
            P::PRESENT_VALUE,
            P::GROUP_MEMBER_NAMES,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::GROUP_MEMBERS,
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        // Default member stores read back empty.
        assert_eq!(
            object.read_property(P::GROUP_MEMBERS, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            object.read_property(P::GROUP_MEMBER_NAMES, None).unwrap(),
            PropertyValue::List(vec![])
        );
        // GlobalGroup collections are BACnetARRAY (Table 12-57), including
        // Present_Value, which is scalar elsewhere.
        assert!(object.is_array_property(P::GROUP_MEMBERS));
        assert!(object.is_array_property(P::GROUP_MEMBER_NAMES));
        assert!(object.is_array_property(P::PRESENT_VALUE));
    }

    #[test]
    fn property_metadata_structured_view_exact_sets_readable_rows_and_indexed_list() {
        let object = StructuredViewObject::new(1, "SV-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::NODE_TYPE,
            P::NODE_SUBTYPE,
            P::SUBORDINATE_LIST,
            P::SUBORDINATE_ANNOTATIONS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::NODE_TYPE,
            P::SUBORDINATE_LIST,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        // Default node/subordinate stores.
        assert_eq!(
            object.read_property(P::NODE_TYPE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::NODE_SUBTYPE, None).unwrap(),
            PropertyValue::CharacterString(String::new())
        );
        assert_eq!(
            object.read_property(P::SUBORDINATE_LIST, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            object
                .read_property(P::SUBORDINATE_ANNOTATIONS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
        // StructuredView subordinate collections are BACnetARRAY (Table 12-34).
        assert!(object.is_array_property(P::SUBORDINATE_LIST));
        assert!(object.is_array_property(P::SUBORDINATE_ANNOTATIONS));
    }

    #[test]
    fn property_metadata_group_trio_write_capabilities_match_dispatch() {
        let fresh: [fn() -> Box<dyn BACnetObject>; 3] = [
            || Box::new(GroupObject::new(1, "G-1").unwrap()),
            || Box::new(GlobalGroupObject::new(1, "GG-1").unwrap()),
            || Box::new(StructuredViewObject::new(1, "SV-1").unwrap()),
        ];
        for make in fresh {
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
                    let capability = match p {
                        P::DESCRIPTION | P::OUT_OF_SERVICE => Always,
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
                // Present_Value, member, and subordinate stores are populated
                // locally, never commanded: even their read-back values are
                // denied on write.
                for p in [
                    P::PRESENT_VALUE,
                    P::LIST_OF_GROUP_MEMBERS,
                    P::GROUP_MEMBERS,
                    P::GROUP_MEMBER_NAMES,
                    P::NODE_TYPE,
                    P::NODE_SUBTYPE,
                    P::SUBORDINATE_LIST,
                    P::SUBORDINATE_ANNOTATIONS,
                ] {
                    if object.read_property(p, None).is_ok() {
                        let value = object.read_property(p, None).unwrap();
                        assert_error(
                            object.write_property(p, None, value, None).unwrap_err(),
                            ErrorCode::WRITE_ACCESS_DENIED,
                        );
                        assert!(!object.is_writable_property(p));
                    }
                }
                // Description and Out_Of_Service reject mistyped values
                // without changing state.
                for (p, value) in [
                    (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                    (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
                ] {
                    assert_error(
                        object.write_property(p, None, value, None).unwrap_err(),
                        ErrorCode::INVALID_DATA_TYPE,
                    );
                }
                assert_eq!(object.property_metadata().as_ref(), original);
            }
        }
    }

    #[test]
    fn property_metadata_group_trio_unserved_rows_stay_unknown() {
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

        let mut group = GroupObject::new(1, "G-1").unwrap();
        assert_unserved(&mut group, P::PROFILE_NAME);
        let mut global = GlobalGroupObject::new(1, "GG-1").unwrap();
        // Event_State and Member_Status_Flags are table rows with no read
        // arm, so they stay absent from the served set.
        assert_unserved(&mut global, P::EVENT_STATE);
        assert_unserved(&mut global, P::MEMBER_STATUS_FLAGS);
        let mut view = StructuredViewObject::new(1, "SV-1").unwrap();
        assert_unserved(&mut view, P::SUBORDINATE_TAGS);
    }
}
