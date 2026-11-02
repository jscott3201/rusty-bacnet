//! The Table 13-1 extras a whole-object notification carries: an object
//! that names no list reports its type's row, and a named list wins.

use super::PreparedReported;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::{BACnetObject, CovReportedProperty, CovReporting};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::borrow::Cow;

/// A custom Access Door, whose Table 13-1 row adds Door_Alarm_State, a
/// trigger. `list` is what its `cov_reported_properties` answers.
struct CustomDoor {
    list: Option<&'static [CovReportedProperty]>,
}

impl BACnetObject for CustomDoor {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ACCESS_DOOR, 1).unwrap()
    }
    fn object_name(&self) -> &str {
        "custom door"
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[P::PRESENT_VALUE, P::STATUS_FLAGS, P::DOOR_ALARM_STATE])
    }
    fn read_property(&self, p: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        match p {
            P::DOOR_ALARM_STATE => Ok(PropertyValue::Enumerated(0)),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn cov(&self) -> Option<&dyn CovReporting> {
        Some(self)
    }
}

impl CovReporting for CustomDoor {
    fn cov_reported_properties(&self) -> Option<&'static [CovReportedProperty]> {
        self.list
    }
}

#[test]
fn a_custom_object_that_names_no_list_reports_its_types_row() {
    let reported = PreparedReported::read(&CustomDoor { list: None }).unwrap();
    let properties: Vec<_> = reported
        .values
        .iter()
        .map(|value| value.property_identifier)
        .collect();
    assert_eq!(properties, [P::DOOR_ALARM_STATE]);
    assert_eq!(reported.triggers.len(), 1, "Door_Alarm_State triggers");
}

#[test]
fn a_named_list_is_reported_exactly() {
    let reported = PreparedReported::read(&CustomDoor { list: Some(&[]) }).unwrap();
    assert!(reported.values.is_empty());
    assert!(reported.triggers.is_empty());
}
