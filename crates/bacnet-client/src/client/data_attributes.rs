//! Data attributes on one request's frames (#2723): what the endpoint's
//! removed requester took as each read's `data_attributes` argument.
//!
//! A link's data attributes go with an NPDU: on BACnet/SC its header data
//! options (Annex AB), such as Secure Path, which the transport checks when
//! it sends; other links carry none. [`BACnetClient::with_data_attributes`]
//! gives a view whose calls send every frame of their confirmed requests
//! with them: the first, each retry and each segment, to any station form.
//!
//! The attributes ride in a task-local value for the life of the call,
//! read where the request path sends a frame. The request path and its
//! futures carry nothing for them, so a request made without them pays
//! one task-local read per frame. A source-audited request, which its node
//! runs in a task of its own, takes them along. The client's own answers
//! and acknowledgments (a SegmentACK for a segmented answer, an Abort)
//! and unconfirmed requests carry none.

use std::future::Future;

use bacnet_transport::port::DataAttribute;

use super::*;

tokio::task_local! {
    /// The data attributes of the confirmed request being made.
    static DATA_ATTRIBUTES: Arc<[DataAttribute]>;
}

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
    /// sends; a link that carries none ignores them. The view has the read
    /// calls the endpoint's removed requester took them on, and
    /// [`run`](WithDataAttributes::run) for any other call.
    pub fn with_data_attributes(
        &self,
        attributes: impl Into<Arc<[DataAttribute]>>,
    ) -> WithDataAttributes<'_, T> {
        WithDataAttributes {
            client: self,
            attributes: attributes.into(),
        }
    }
}

impl<'a, T: TransportPort + 'static> WithDataAttributes<'a, T> {
    /// The data attributes this view's requests carry.
    pub fn attributes(&self) -> &[DataAttribute] {
        &self.attributes
    }

    /// Make `request`'s call on the client with this view's attributes:
    /// every confirmed request it makes carries them, as in
    /// `secure.run(|client| client.write_property(&station, ...))`.
    pub async fn run<F, Fut>(&self, request: F) -> Fut::Output
    where
        F: FnOnce(&'a BACnetClient<T>) -> Fut,
        Fut: Future,
    {
        DATA_ATTRIBUTES
            .scope(Arc::clone(&self.attributes), request(self.client))
            .await
    }

    /// [`BACnetClient::confirmed_request`] with this view's attributes.
    pub async fn confirmed_request(
        &self,
        to: &(impl ToStation + ?Sized),
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        self.run(|client| client.confirmed_request(to, service_choice, service_data))
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
        self.run(|client| {
            client.read_property(
                to,
                object_identifier,
                property_identifier,
                property_array_index,
            )
        })
        .await
    }

    /// [`BACnetClient::read_property_multiple`] with this view's attributes.
    pub async fn read_property_multiple(
        &self,
        to: &(impl ToStation + ?Sized),
        specs: Vec<bacnet_types::constructed::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        self.run(|client| client.read_property_multiple(to, specs))
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
        self.run(|client| {
            client.read_range(
                to,
                object_identifier,
                property_identifier,
                property_array_index,
                range,
            )
        })
        .await
    }
}

/// The data attributes of the confirmed request being made, when it is
/// made through a [`WithDataAttributes`] view.
pub(super) fn scoped() -> Option<Arc<[DataAttribute]>> {
    DATA_ATTRIBUTES.try_with(Arc::clone).ok()
}

/// `request` made with `attributes`, or as it is with none: for a request
/// that goes on in another task, such as a source-audited one.
pub(super) async fn within<F: Future>(
    attributes: Option<Arc<[DataAttribute]>>,
    request: F,
) -> F::Output {
    match attributes {
        Some(attributes) => DATA_ATTRIBUTES.scope(attributes, request).await,
        None => request.await,
    }
}

#[cfg(test)]
#[path = "data_attributes_tests.rs"]
mod tests;
