//! ReadRange: one page, checked strictly or leniently against its request.
use super::*;

use bacnet_services::read_range::{
    RangeSpec, ReadRangeAck, ReadRangeReply, ReadRangeRequest, ReadRangeValidation,
};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Read a range of items from a list or log-buffer property.
    ///
    /// The acknowledgement must keep every rule
    /// [`ReadRangeAck::violations`] checks against the request; one that
    /// breaks a rule fails with [`Error::ReadRangeViolation`]. Use
    /// [`read_range_with`](Self::read_range_with) and
    /// [`ReadRangeValidation::Lenient`] to keep such an answer.
    pub async fn read_range(
        &self,
        destination_mac: &[u8],
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
        range: Option<RangeSpec>,
    ) -> Result<ReadRangeAck, Error> {
        let request = ReadRangeRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            range,
        };
        self.read_range_with(destination_mac, &request, ReadRangeValidation::Strict)
            .await
            .map(|reply| reply.ack)
    }

    /// Read a range of items, choosing how to treat an acknowledgement that
    /// breaks a rule: [`Strict`](ReadRangeValidation::Strict) refuses it,
    /// [`Lenient`](ReadRangeValidation::Lenient) returns it with the list of
    /// rules it broke, so a device that numbers the record after its
    /// sequence wrap 0 doesn't cost the page.
    ///
    /// An acknowledgement that doesn't decode fails under either mode.
    pub async fn read_range_with(
        &self,
        destination_mac: &[u8],
        request: &ReadRangeRequest,
        validation: ReadRangeValidation,
    ) -> Result<ReadRangeReply, Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        let response = self
            .confirmed_request(destination_mac, ConfirmedServiceChoice::READ_RANGE, &buf)
            .await?;
        ReadRangeReply::check(request, ReadRangeAck::decode(&response)?, validation)
    }
}
