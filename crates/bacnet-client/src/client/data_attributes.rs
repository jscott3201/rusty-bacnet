//! Data attributes on one request's frames (#2723): what the endpoint's
//! removed requester took as each read's `data_attributes` argument.
//!
//! A link's data attributes go with an NPDU: on BACnet/SC its header data
//! options (Annex AB), such as Secure Path, which the transport checks when
//! it sends; other links carry none. [`BACnetClient::with_data_attributes`]
//! gives a view whose calls send every frame of their confirmed requests
//! with them: the first, each retry and each segment, to any station form.
//!
//! The request hands them to its transaction when it registers, and each
//! frame it sends reads them from the transaction's owner, which every send
//! already holds; a request made without the view hands over nothing and
//! its sends find none. No ambient state is involved, so a request made
//! through the view carries them whichever task runs it. A source-audited
//! request, which its node runs in a task of its own, takes them along.
//! The client's own answers and acknowledgments (a SegmentACK for a
//! segmented answer, an Abort) and unconfirmed requests carry none.

use bacnet_transport::port::DataAttribute;

use super::*;

/// The data attributes a confirmed request's frames carry, `None` for
/// none, as the request path passes them to the transaction.
pub(super) type Attributes<'a> = Option<&'a Arc<[DataAttribute]>>;

/// A client whose confirmed requests carry data attributes on every frame,
/// from [`BACnetClient::with_data_attributes`]. Keep it to send several
/// requests with the same attributes; it borrows the client and costs one
/// reference count to make.
pub struct WithDataAttributes<'a, T: TransportPort = AnyTransport> {
    client: &'a BACnetClient<T>,
    attributes: Arc<[DataAttribute]>,
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// This client, sending every frame of the confirmed requests made
    /// through the view (the first, each retry and each segment, to any
    /// station form) with `attributes` (#2723). On BACnet/SC they are the
    /// header data options of Annex AB, which the transport checks when it
    /// sends; a link that carries none ignores them. The view has the calls
    /// the endpoint's removed requester took them on, ReadProperty,
    /// ReadPropertyMultiple and ReadRange, and [`confirmed_request`] for any
    /// other confirmed service.
    ///
    /// [`confirmed_request`]: WithDataAttributes::confirmed_request
    pub fn with_data_attributes(
        &self,
        attributes: impl Into<Arc<[DataAttribute]>>,
    ) -> WithDataAttributes<'_, T> {
        WithDataAttributes {
            client: self,
            attributes: attributes.into(),
        }
    }

    /// [`Self::confirmed_request`] with every frame carrying `attributes`,
    /// for a request that goes on in another task, such as a source-audited
    /// one.
    pub(super) async fn confirmed_request_with(
        &self,
        to: &(impl ToStation + ?Sized),
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
        attributes: Attributes<'_>,
    ) -> Result<Bytes, Error> {
        to.confirmed(self, service_choice, service_data, attributes)
            .await
    }
}

impl<T: TransportPort + 'static> WithDataAttributes<'_, T> {
    /// The data attributes this view's requests carry.
    pub fn attributes(&self) -> &[DataAttribute] {
        &self.attributes
    }

    /// [`BACnetClient::confirmed_request`] with this view's attributes.
    pub async fn confirmed_request(
        &self,
        to: &(impl ToStation + ?Sized),
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        to.confirmed(
            self.client,
            service_choice,
            service_data,
            Some(&self.attributes),
        )
        .await
    }

    /// [`BACnetClient::read_property`] with this view's attributes.
    pub async fn read_property(
        &self,
        to: &(impl ToStation + ?Sized),
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        let request = bacnet_services::read_property::ReadPropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
        };
        self.client
            .call_with(to, request, Some(&self.attributes))
            .await
    }

    /// [`BACnetClient::read_property_multiple`] with this view's attributes.
    pub async fn read_property_multiple(
        &self,
        to: &(impl ToStation + ?Sized),
        specs: Vec<bacnet_types::constructed::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: specs,
        };
        self.client
            .call_with(to, request, Some(&self.attributes))
            .await
    }

    /// [`BACnetClient::read_range`] with this view's attributes.
    pub async fn read_range(
        &self,
        to: &(impl ToStation + ?Sized),
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        range: Option<bacnet_services::read_range::RangeSpec>,
    ) -> Result<bacnet_services::read_range::ReadRangeAck, Error> {
        let request = bacnet_services::read_range::ReadRangeRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            range,
        };
        let range = super::service::Range {
            request: &request,
            validation: bacnet_services::read_range::ReadRangeValidation::Strict,
        };
        self.client
            .call_with(to, range, Some(&self.attributes))
            .await
            .map(|reply| reply.ack)
    }
}

#[cfg(test)]
#[path = "data_attributes_tests.rs"]
mod tests;
