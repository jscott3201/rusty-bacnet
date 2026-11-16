//! Decoding an acknowledgement and checking it answers its request: the
//! answer checks of the typed calls' service descriptions (#2588), which
//! every way of sending a request shares.

use bacnet_services::file::{AtomicReadFileAck, FileAccessMethod, FileReadAckMethod};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_types::{enums::ObjectType, error::Error, primitives::ObjectIdentifier};

/// A ReadProperty acknowledgement for `request`: it must name the object,
/// property and array index read (Clause 15.5). A request by the Device or
/// Network Port wildcard instance takes the object of that type the device
/// reports as its own; a wildcard instance in the answer is refused.
///
/// # Errors
///
/// A decoding error for an answer that doesn't decode or doesn't match.
pub(crate) fn read_property(
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

/// An AtomicReadFile acknowledgement for a request with `access`: the same
/// access method, and no more octets or records than it asked for.
///
/// # Errors
///
/// A decoding error for an answer that doesn't decode or doesn't match.
pub(crate) fn atomic_read_file(
    access: &FileAccessMethod,
    bytes: &[u8],
) -> Result<AtomicReadFileAck, Error> {
    let ack = AtomicReadFileAck::decode(bytes)?;
    validate_atomic_read_file_ack(access, &ack)?;
    Ok(ack)
}

/// The window check of [`atomic_read_file`], on a decoded answer.
pub(crate) fn validate_atomic_read_file_ack(
    request: &FileAccessMethod,
    ack: &AtomicReadFileAck,
) -> Result<(), Error> {
    match (request, &ack.access) {
        (
            FileAccessMethod::Stream {
                requested_octet_count,
                ..
            },
            FileReadAckMethod::Stream { file_data, .. },
        ) if file_data.len() <= *requested_octet_count as usize => Ok(()),
        (FileAccessMethod::Stream { .. }, FileReadAckMethod::Stream { .. }) => Err(
            Error::decoding(0, "AtomicReadFile ACK exceeds the requested octet window"),
        ),
        (
            FileAccessMethod::Record {
                requested_record_count,
                ..
            },
            FileReadAckMethod::Record {
                returned_record_count,
                file_record_data,
                ..
            },
        ) if returned_record_count <= requested_record_count
            && *returned_record_count as usize == file_record_data.len() =>
        {
            Ok(())
        }
        (FileAccessMethod::Record { .. }, FileReadAckMethod::Record { .. }) => Err(
            Error::decoding(0, "AtomicReadFile ACK exceeds the requested record window"),
        ),
        _ => Err(Error::decoding(
            0,
            "AtomicReadFile ACK access method does not match the request",
        )),
    }
}
