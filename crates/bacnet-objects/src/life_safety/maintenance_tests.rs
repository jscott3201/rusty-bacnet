//! A Point's Maintenance_Required: a BACnetMaintenance (Table 12-18), read,
//! set by the application and written by clients (#2252). The Zone's is a
//! BOOLEAN of its own (Table 12-19), which this object doesn't serve.

use super::LifeSafetyPointObject;
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, Maintenance, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

#[test]
fn maintenance_required_is_a_bacnet_maintenance() {
    // A BACnetMaintenance (Table 12-18), starting at NONE (#2252).
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let read = |pt: &LifeSafetyPointObject| {
        pt.read_property(PropertyIdentifier::MAINTENANCE_REQUIRED, None)
            .unwrap()
    };
    assert_eq!(read(&pt), PropertyValue::Enumerated(0));
    pt.set_maintenance_required(Maintenance::NEED_SERVICE_INOPERATIVE);
    assert_eq!(read(&pt), PropertyValue::Enumerated(3));
}

#[test]
fn writes_take_a_bacnet_maintenance_and_refuse_a_boolean() {
    let mut pt = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let read = |pt: &LifeSafetyPointObject| {
        pt.read_property(PropertyIdentifier::MAINTENANCE_REQUIRED, None)
            .unwrap()
    };
    // Every standard value, and the proprietary range's ends.
    for raw in [0, 1, 2, 3, 256, 65_535] {
        pt.write_property(
            PropertyIdentifier::MAINTENANCE_REQUIRED,
            None,
            PropertyValue::Enumerated(raw),
            None,
        )
        .unwrap();
        assert_eq!(read(&pt), PropertyValue::Enumerated(raw));
    }
    // Reserved and out-of-range numbers, then other datatypes: the old
    // BOOLEAN, and the Unsigned a client might send. None changes the value.
    for (value, code) in [
        (PropertyValue::Enumerated(4), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Enumerated(255),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            PropertyValue::Enumerated(65_536),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Boolean(true), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Boolean(false), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Unsigned(2), ErrorCode::INVALID_DATA_TYPE),
    ] {
        let error = pt
            .write_property(PropertyIdentifier::MAINTENANCE_REQUIRED, None, value, None)
            .unwrap_err();
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code: c })
                if class == ErrorClass::PROPERTY && c == code),
            "{error:?}"
        );
        assert_eq!(read(&pt), PropertyValue::Enumerated(65_535));
    }
}
