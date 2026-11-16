//! ReadRange: one page, checked strictly or leniently against its request.
use super::service::Range;
use super::*;

use bacnet_services::read_range::{
    RangeSpec, ReadRangeAck, ReadRangeReply, ReadRangeRequest, ReadRangeValidation,
};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Read a range of items from a list or log-buffer property on the
    /// station `to`.
    ///
    /// The acknowledgement must keep every rule
    /// [`ReadRangeAck::violations`] checks against the request; one that
    /// breaks a rule fails with [`Error::ReadRangeViolation`]. Use
    /// [`read_range_with`](Self::read_range_with) and
    /// [`ReadRangeValidation::Lenient`] to keep such an answer.
    pub async fn read_range(
        &self,
        to: &(impl ToStation + ?Sized),
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
        self.read_range_with(to, &request, ReadRangeValidation::Strict)
            .await
            .map(|reply| reply.ack)
    }

    /// Read a range of items from the station `to`, choosing how to treat
    /// an acknowledgement that breaks a rule:
    /// [`Strict`](ReadRangeValidation::Strict) refuses it,
    /// [`Lenient`](ReadRangeValidation::Lenient) returns it with the list of
    /// rules it broke, so a device that numbers the record after its
    /// sequence wrap 0 doesn't cost the page.
    ///
    /// An acknowledgement that doesn't decode fails under either mode.
    pub async fn read_range_with(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &ReadRangeRequest,
        validation: ReadRangeValidation,
    ) -> Result<ReadRangeReply, Error> {
        self.call(
            to,
            Range {
                request,
                validation,
            },
        )
        .await
    }
}

#[cfg(test)]
#[path = "routed_read_services_tests.rs"]
mod tests;
