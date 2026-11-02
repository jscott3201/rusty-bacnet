//! Catalogue-wide check that every built-in object refuses an indexed read
//! of a property that isn't an array, the way the server does (#2274).
//!
//! The server runs [`check_read_array_index`] after the object lookup and
//! before any object reads a value. Each built-in object runs it too, first
//! in `read_property`, so a direct caller gets the same answer instead of
//! the index being ignored and the whole value returned.

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

use crate::catalogue::representatives;
use crate::property_metadata::{check_read_array_index, has_property, standard_value_type};

fn refused<T>(result: &Result<T, Error>, expected: ErrorCode) -> bool {
    matches!(
        result,
        Err(Error::Remote(RemoteError::Error { class, code }))
            if *class == ErrorClass::PROPERTY && *code == expected
    )
}

fn not_an_array<T>(result: &Result<T, Error>) -> bool {
    refused(result, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
}

#[test]
fn every_scalar_row_refuses_an_indexed_direct_read() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        for row in object.property_metadata().iter() {
            let property = row.property_identifier;
            if crate::property_metadata::is_array_property(&*object, property) {
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
            if !crate::property_metadata::is_array_property(&*object, property) {
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

/// A standard array the object doesn't have keeps its array shape, so an
/// indexed read of it passes the check and the object answers
/// UNKNOWN_PROPERTY, as the server does.
#[test]
fn an_absent_standard_array_read_with_an_index_is_unknown() {
    let arrays = [
        PropertyIdentifier::PRIORITY_ARRAY,
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        PropertyIdentifier::STATE_TEXT,
        PropertyIdentifier::ALARM_VALUES,
        PropertyIdentifier::TAGS,
    ];
    let mut checked = 0;
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        let rows = object.property_metadata();
        for property in arrays {
            let standard_array = standard_value_type(object_type, property)
                .is_some_and(|value_type| value_type.is_array());
            if has_property(&rows, property) || !standard_array {
                continue;
            }
            let result = object.read_property(property, Some(1));
            assert!(
                refused(&result, ErrorCode::UNKNOWN_PROPERTY),
                "{object_type} {property}: {result:?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no absent standard array checked");
}

/// A proprietary property has no standard shape, so an index on one the
/// object lacks is PROPERTY_IS_NOT_AN_ARRAY, as the server answers; without
/// an index it is UNKNOWN_PROPERTY.
#[test]
fn an_indexed_proprietary_property_is_not_an_array() {
    let proprietary = PropertyIdentifier::from_raw(600);
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        let indexed = object.read_property(proprietary, Some(1));
        assert!(not_an_array(&indexed), "{object_type}: {indexed:?}");
        let whole = object.read_property(proprietary, None);
        assert!(
            refused(&whole, ErrorCode::UNKNOWN_PROPERTY),
            "{object_type}: {whole:?}"
        );
    }
}
