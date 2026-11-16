//! ReadRange: one page, checked strictly or leniently against its request.
use super::service::{Range, device_target};
use super::*;

use bacnet_services::read_range::{
    RangeSpec, ReadRangeAck, ReadRangeReply, ReadRangeRequest, ReadRangeValidation,
};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use crate::log_reader::{LogCursor, LogPage, LogRequester};

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

    /// Read a range from a discovered device, routing when needed.
    ///
    /// The acknowledgement is checked strictly, as in [`Self::read_range`].
    /// A missing device fails before sending.
    pub async fn read_range_from_device(
        &self,
        device_instance: u32,
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
        self.read_range_from_device_with(device_instance, &request, ReadRangeValidation::Strict)
            .await
            .map(|reply| reply.ack)
    }

    /// Read a range from a discovered device with strict or lenient validation.
    ///
    /// This resolves the device once and preserves the acknowledgement checks
    /// of [`Self::read_range_with`], including decode failures in either mode.
    pub async fn read_range_from_device_with(
        &self,
        device_instance: u32,
        request: &ReadRangeRequest,
        validation: ReadRangeValidation,
    ) -> Result<ReadRangeReply, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let range = Range {
            request,
            validation,
        };
        self.call_via(device_target(&mac, &routing), range).await
    }

    /// Read one log page from a discovered device, routing every subrequest.
    ///
    /// The router, remote network and station are resolved once for this page;
    /// metadata property reads and range requests all use that address snapshot.
    /// A later page resolves the device again. Cursor, page-size and reply rules
    /// are those of [`Self::read_log_page`]; a missing device fails before sending.
    pub async fn read_log_page_from_device(
        &self,
        device_instance: u32,
        log: ObjectIdentifier,
        cursor: LogCursor,
        page_size: u16,
    ) -> Result<LogPage, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((network, address)) = routing {
            let requester = RoutedLogRequester {
                client: self,
                network,
                address: &address,
            };
            crate::log_reader::read_log_page(&requester, &mac, log, cursor, page_size).await
        } else {
            self.read_log_page(&mac, log, cursor, page_size).await
        }
    }
}

/// A page's remote station, fixed across metadata reads and range retries.
struct RoutedLogRequester<'a, T: TransportPort> {
    client: &'a BACnetClient<T>,
    network: u16,
    address: &'a [u8],
}

impl<'a, T: TransportPort> RoutedLogRequester<'a, T> {
    /// The station through the router the reader names.
    fn target<'r>(&self, router_mac: &'r [u8]) -> ConfirmedTarget<'r>
    where
        'a: 'r,
    {
        ConfirmedTarget::Routed {
            router_mac,
            dest_network: self.network,
            dest_mac: self.address,
        }
    }
}

impl<T: TransportPort + 'static> LogRequester for RoutedLogRequester<'_, T> {
    async fn read_unsigned(
        &self,
        router_mac: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        let request = bacnet_services::read_property::ReadPropertyRequest {
            object_identifier: log,
            property_identifier: property,
            property_array_index: None,
        };
        let ack = self
            .client
            .call_via(self.target(router_mac), request)
            .await?;
        crate::log_reader::unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        router_mac: &[u8],
        request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        let range = Range {
            request,
            validation: ReadRangeValidation::Lenient,
        };
        self.client.call_via(self.target(router_mac), range).await
    }
}

#[cfg(test)]
#[path = "routed_read_services_tests.rs"]
mod tests;
