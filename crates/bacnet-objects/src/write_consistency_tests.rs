//! Catalogue-wide checks that every built-in object answers a direct write
//! the same way (#2258).
//!
//! - An array index on a property that isn't an array is refused. The
//!   server runs [`check_write_array_index`] before any object sees a write;
//!   each built-in object runs it too, first in `write_property` and
//!   `write_property_from`, so a direct caller gets the same answer instead
//!   of the index being dropped and the write taken whole.
//! - A NULL written to Description succeeds and leaves it as it was, the
//!   shared helper's rule, on every object whose Description is writable.

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

use crate::catalogue::representatives;
use crate::command_source::test_origin;
use crate::property_metadata::check_write_array_index;

fn not_an_array(result: &Result<(), Error>) -> bool {
    matches!(
        result,
        Err(Error::Remote(RemoteError::Error { class, code }))
            if *class == ErrorClass::PROPERTY && *code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY
    )
}

#[test]
fn every_scalar_row_refuses_an_indexed_direct_write() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        let rows = object.property_metadata().into_owned();
        for row in rows {
            let property = row.property_identifier;
            if object.is_array_property(property) {
                continue;
            }
            assert!(not_an_array(&check_write_array_index(
                object.as_ref(),
                property,
                Some(1)
            )));
            // The current value, which a whole write would take.
            let value = object
                .read_property(property, None)
                .unwrap_or(PropertyValue::Null);
            let direct = object.write_property(property, Some(1), value.clone(), None);
            let from =
                object.write_property_from(property, Some(1), value, Some(8), &test_origin());
            for (route, result) in [("write_property", direct), ("write_property_from", from)] {
                if !not_an_array(&result) {
                    failures.push(format!("{object_type} {property} {route}: {result:?}"));
                }
            }
            checked += 1;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked > 900, "only {checked} rows checked");
}

#[test]
fn every_writable_description_takes_null_as_unchanged() {
    let mut checked = 0;
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        if !object.is_writable_property(PropertyIdentifier::DESCRIPTION) {
            continue;
        }
        let text = PropertyValue::CharacterString("before".into());
        object
            .write_property(PropertyIdentifier::DESCRIPTION, None, text.clone(), None)
            .unwrap();
        let result = object.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::Null,
            None,
        );
        assert!(result.is_ok(), "{object_type}: {result:?}");
        assert_eq!(
            object
                .read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            text,
            "{object_type}"
        );
        checked += 1;
    }
    assert!(checked > 50, "only {checked} objects checked");
}
