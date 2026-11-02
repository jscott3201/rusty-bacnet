//! Catalogue-wide check that every built-in object refuses an indexed read
//! of a property that isn't an array, the way the server does (#2274).
//!
//! The server runs [`check_read_array_index`] after the object lookup and
//! before any object reads a value. Each built-in object runs it too, first
//! in `read_property`, so a direct caller gets the same answer instead of
//! the index being ignored and the whole value returned.

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

use crate::catalogue::representatives;
use crate::property_metadata::check_read_array_index;

fn not_an_array<T>(result: &Result<T, Error>) -> bool {
    matches!(
        result,
        Err(Error::Remote(RemoteError::Error { class, code }))
            if *class == ErrorClass::PROPERTY && *code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY
    )
}

#[test]
fn every_scalar_row_refuses_an_indexed_direct_read() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        for row in object.property_metadata().iter() {
            let property = row.property_identifier;
            if object.is_array_property(property) {
                continue;
            }
            assert!(not_an_array(&check_read_array_index(
                object.as_ref(),
                property,
                Some(1)
            )));
            for index in [0, 1, u32::MAX] {
                let result: Result<PropertyValue, Error> =
                    object.read_property(property, Some(index));
                if !not_an_array(&result) {
                    failures.push(format!("{object_type} {property} [{index}]: {result:?}"));
                }
            }
            checked += 1;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked > 1000, "only {checked} rows checked");
}

#[test]
fn array_rows_pass_the_read_check() {
    let mut checked = 0;
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        for row in object.property_metadata().iter() {
            let property = row.property_identifier;
            if !object.is_array_property(property) {
                continue;
            }
            // Element 0 is the array's size, which every array serves.
            let result = object.read_property(property, Some(0));
            assert!(
                !not_an_array(&result),
                "{object_type} {property}: {result:?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 150, "only {checked} array rows checked");
}
