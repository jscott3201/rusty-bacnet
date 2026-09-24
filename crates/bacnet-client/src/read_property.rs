//! Shared standalone/endpoint ReadProperty ACK correlation (Clause 15.5).
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_types::{enums::ObjectType, error::Error, primitives::ObjectIdentifier};

pub(crate) fn decode_ack(
    request: &ReadPropertyRequest,
    bytes: &[u8],
) -> Result<ReadPropertyACK, Error> {
    let ack = ReadPropertyACK::decode(bytes)?;
    let requested = request.object_identifier;
    let reported = ack.object_identifier;
    // 15.5.2 permits these request aliases; 15.5.1.2 identifies the object
    // actually read in the ACK. A wildcard ACK never establishes that identity.
    let permitted_alias = requested.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
        && matches!(
            requested.object_type(),
            ObjectType::DEVICE | ObjectType::NETWORK_PORT
        );
    if reported.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
        || reported.object_type() != requested.object_type()
        || (!permitted_alias && reported != requested)
    {
        return Err(Error::decoding(
            0,
            "ReadProperty ACK object identifier does not match the request",
        ));
    }
    if ack.property_identifier != request.property_identifier {
        return Err(Error::decoding(
            0,
            "ReadProperty ACK property identifier does not match the request",
        ));
    }
    if ack.property_array_index != request.property_array_index {
        return Err(Error::decoding(
            0,
            "ReadProperty ACK array index does not match the request",
        ));
    }
    Ok(ack)
}
