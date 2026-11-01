//! The Device properties the executor owns: a host serves them from its own
//! state, so no write reaches the Device object for them.
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// The Device properties the executor serves from its own state: its
/// services, Property_List, the two COV lists and Device_Address_Binding.
pub fn owned(property: P) -> bool {
    matches!(
        property,
        P::PROTOCOL_SERVICES_SUPPORTED
            | P::PROPERTY_LIST
            | P::ACTIVE_COV_SUBSCRIPTIONS
            | P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
            | P::DEVICE_ADDRESS_BINDING
    )
}
/// Executor-owned fields cannot be assigned through a custom Device writer.
/// Call only after the existing object/index/value and authorization preflight.
pub fn check_executor_owned_write(oid: ObjectIdentifier, property: P) -> Result<(), Error> {
    if oid.object_type() == ObjectType::DEVICE && (owned(property) || identity_property(property)) {
        Err(property_error(ErrorCode::WRITE_ACCESS_DENIED))
    } else {
        Ok(())
    }
}

fn property_error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::PROPERTY, code)
}

/// The Device properties projected from its frozen identity: the
/// identifier, vendor, APDU capacity and the segmentation declarations.
pub fn identity_property(property: P) -> bool {
    matches!(
        property,
        P::OBJECT_IDENTIFIER
            | P::VENDOR_IDENTIFIER
            | P::MAX_APDU_LENGTH_ACCEPTED
            | P::SEGMENTATION_SUPPORTED
            | P::APDU_SEGMENT_TIMEOUT
            | P::MAX_SEGMENTS_ACCEPTED
    )
}
