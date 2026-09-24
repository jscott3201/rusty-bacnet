//! Shared standalone and endpoint ReadRange response correlation.
use bacnet_types::error::Error;

pub(crate) fn validate_ack(
    request: &bacnet_services::read_range::ReadRangeRequest,
    ack: &bacnet_services::read_range::ReadRangeAck,
) -> Result<(), Error> {
    if ack.object_identifier != request.object_identifier {
        return Err(Error::decoding(
            0,
            "ReadRange ACK object identifier does not match the request",
        ));
    }
    if ack.property_identifier != request.property_identifier {
        return Err(Error::decoding(
            0,
            "ReadRange ACK property identifier does not match the request",
        ));
    }
    if ack.property_array_index != request.property_array_index {
        return Err(Error::decoding(
            0,
            "ReadRange ACK array index does not match the request",
        ));
    }

    let sequence_range = matches!(
        request.range.as_ref(),
        Some(
            bacnet_services::read_range::RangeSpec::BySequenceNumber { .. }
                | bacnet_services::read_range::RangeSpec::ByTime { .. }
        )
    );
    match (sequence_range, ack.item_count, ack.first_sequence_number) {
        (true, 1.., Some(1..)) | (true, 0, None) | (false, _, None) => {}
        (true, 1.., _) => {
            return Err(Error::decoding(
                0,
                "nonempty ReadRange By Sequence/Time ACK requires a nonzero first sequence number",
            ));
        }
        _ => {
            return Err(Error::decoding(
                0,
                "ReadRange ACK first sequence number is invalid for the request range",
            ));
        }
    }

    Ok(())
}
