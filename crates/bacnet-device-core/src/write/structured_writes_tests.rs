//! The `structured-writes` feature (#2210): with it, the four datatypes the
//! core decodes itself reach their objects; without it, each write of one is
//! refused with PROPERTY / WRITE_ACCESS_DENIED, and nothing else changes.
use super::decode_write_property_value;
use alloc::vec;
use alloc::vec::Vec;
use bacnet_objects::property_metadata::{Datatype, standard_value_type};
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

/// A write: (object type, property, index, the datatype its row names, the
/// octets).
type Case = (ObjectType, P, Option<u32>, Datatype, Vec<u8>);

/// One write of each datatype the feature covers, with a value of it that
/// decodes.
fn structured_writes() -> Vec<Case> {
    vec![
        // An opaque BACnetEventParameter.
        (
            ObjectType::EVENT_ENROLLMENT,
            P::EVENT_PARAMETERS,
            None,
            Datatype::EventParameter,
            vec![0xfe, 0xff, 0x01, 0x02, 0x03, 0xff, 0xff],
        ),
        // A Recipient naming device 7.
        (
            ObjectType::DEVICE,
            P::AUDIT_NOTIFICATION_RECIPIENT,
            None,
            Datatype::Recipient,
            vec![0x0c, 0x02, 0x00, 0x00, 0x07],
        ),
        // A BACnetPortPermission, as one element and as the whole array.
        (
            ObjectType::NOTIFICATION_FORWARDER,
            P::PORT_FILTER,
            Some(1),
            Datatype::PortPermission,
            vec![0x09, 0x01, 0x19, 0x01],
        ),
        (
            ObjectType::NOTIFICATION_FORWARDER,
            P::PORT_FILTER,
            None,
            Datatype::PortPermission,
            vec![0x09, 0x01, 0x19, 0x01, 0x09, 0x02, 0x19, 0x00],
        ),
        // A BACnetStageLimitValue.
        (
            ObjectType::STAGING,
            P::STAGES,
            Some(1),
            Datatype::StageLimitValue,
            vec![
                0x44, 0x41, 0x20, 0x00, 0x00, 0x82, 0x07, 0x80, 0x44, 0x40, 0xa0, 0x00, 0x00,
            ],
        ),
    ]
}

fn decode(
    object_type: ObjectType,
    property: P,
    index: Option<u32>,
    octets: &[u8],
) -> Result<PropertyValue, Error> {
    let value_type = standard_value_type(object_type, property);
    decode_write_property_value(property, value_type, index, octets)
}

#[cfg(not(feature = "structured-writes"))]
fn is_write_access_denied(result: &Result<PropertyValue, Error>) -> bool {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::RemoteError;
    matches!(
        result,
        Err(Error::Remote(RemoteError::Error { class, code }))
            if *class == ErrorClass::PROPERTY && *code == ErrorCode::WRITE_ACCESS_DENIED
    )
}

/// The rows name the datatypes the cases say, so each case reaches the arm
/// it is meant to.
#[test]
fn each_case_names_its_datatype() {
    for (object_type, property, _, datatype, _) in structured_writes() {
        let value_type = standard_value_type(object_type, property).unwrap();
        assert_eq!(
            value_type.datatype, datatype,
            "{object_type:?} {property:?}"
        );
    }
}

#[cfg(feature = "structured-writes")]
#[test]
fn with_the_feature_each_structured_value_reaches_its_object() {
    for (object_type, property, index, _, octets) in structured_writes() {
        let result = decode(object_type, property, index, &octets);
        assert!(
            result.is_ok(),
            "{object_type:?} {property:?} {index:?}: {result:?}"
        );
    }
}

#[cfg(not(feature = "structured-writes"))]
#[test]
fn without_the_feature_each_structured_write_is_refused() {
    for (object_type, property, index, _, octets) in structured_writes() {
        // A lone NULL too: it is no relinquish of a property the device
        // doesn't let anyone write.
        for octets in [octets, vec![0x00]] {
            let result = decode(object_type, property, index, &octets);
            assert!(
                is_write_access_denied(&result),
                "{object_type:?} {property:?} {index:?} {octets:02x?}: {result:?}"
            );
        }
    }
}

/// What the feature doesn't cover decodes the same either way: an array's
/// size at index 0, an application value, and a raw constructed value.
#[test]
fn other_writes_are_decoded_either_way() {
    assert_eq!(
        decode(
            ObjectType::NOTIFICATION_FORWARDER,
            P::PORT_FILTER,
            Some(0),
            &[0x21, 0x02]
        )
        .unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        decode(
            ObjectType::BINARY_VALUE,
            P::PRESENT_VALUE,
            None,
            &[0x91, 0x01]
        )
        .unwrap(),
        PropertyValue::Enumerated(1)
    );
    let weekday = [0x0e, 0x0f];
    assert_eq!(
        decode(ObjectType::SCHEDULE, P::WEEKLY_SCHEDULE, Some(1), &weekday).unwrap(),
        PropertyValue::ApplicationData(weekday.to_vec())
    );
}

// A hosted build keeps every write it had: `std` turns the feature on.
#[cfg(feature = "std")]
const _: () = assert!(cfg!(feature = "structured-writes"));
