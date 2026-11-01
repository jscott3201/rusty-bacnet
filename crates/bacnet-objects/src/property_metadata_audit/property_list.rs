//! The catalogue-wide Property_List check (#2197): every type serves the
//! Property_List its metadata rows derive, through the read path, and no
//! more.

use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

use crate::property_metadata::{has_property, in_property_list, property_list};

/// Each type's Property_List, read whole, by count and element by element,
/// is its rows less the four properties Clause 12.1.1.4.1 leaves out, in row
/// order, and is what the type serves: every listed property reads, and no
/// property the type serves is missing from its rows. The rows hold all
/// four exclusions, so a type that dropped one fails here too.
#[test]
fn every_catalogue_type_serves_the_property_list_its_rows_derive() {
    let mut objects = super::supported_representatives();
    // The catalogue's own test checks it holds one object per supported type.
    assert!(!objects.is_empty());
    // Staging's started state, as the audit reaches it.
    for object in objects
        .iter_mut()
        .filter(|object| object.object_identifier().object_type() == ObjectType::STAGING)
    {
        object
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(5.0), None)
            .unwrap();
    }
    for object in &objects {
        let object_type = object.object_identifier().object_type();
        let rows = object.property_metadata();
        for excluded in [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PROPERTY_LIST,
        ] {
            assert!(
                has_property(&rows, excluded),
                "{object_type:?} rows must hold {excluded:?}"
            );
            assert!(!in_property_list(excluded));
        }
        let derived: Vec<PropertyValue> = property_list(&rows)
            .map(|property| PropertyValue::Enumerated(property.to_raw()))
            .collect();
        assert_eq!(
            derived.len() + 4,
            rows.len(),
            "{object_type:?} lists every row but the four exclusions"
        );

        let served = object.read_property(P::PROPERTY_LIST, None).unwrap();
        assert_eq!(
            served,
            PropertyValue::List(derived.clone()),
            "{object_type:?} serves the Property_List its rows derive"
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(derived.len() as u64),
            "{object_type:?} Property_List length"
        );
        for (index, element) in derived.iter().enumerate() {
            assert_eq!(
                &object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                element,
                "{object_type:?} Property_List[{}]",
                index + 1
            );
        }
        for property in property_list(&rows) {
            assert!(
                crate::property_metadata_tests::metadata_row_reads(object.as_ref(), property),
                "{object_type:?} lists {property:?} but doesn't serve it"
            );
        }
        let unlisted = crate::property_metadata_tests::served_without_a_row(object.as_ref());
        assert!(unlisted.is_empty(), "{}", unlisted.join("\n"));
        let past_end = object.read_property(P::PROPERTY_LIST, Some(derived.len() as u32 + 1));
        assert!(
            matches!(
                past_end,
                Err(Error::Remote(RemoteError::Error {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::INVALID_ARRAY_INDEX,
                }))
            ),
            "{object_type:?} Property_List past its end: {past_end:?}"
        );
    }
}
