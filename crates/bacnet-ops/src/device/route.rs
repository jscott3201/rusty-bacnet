//! Requests by address: to a station on this network by its MAC, and to one
//! behind a router through the router the low-level client's discovery
//! table holds for it (Clause 6.2.2), never through an instance. A routed
//! address the table has no row for has no route, and fails with
//! [`Error::UnknownDevice`] before anything is sent.

use bacnet_client::client::BACnetClient;
use bacnet_services::alarm_event::{GetEventInformationAck, GetEventInformationRequest};
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::read_range::ReadRangeValidation;
use bacnet_services::read_range::{ReadRangeAck, ReadRangeReply, ReadRangeRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::constructed::ReadAccessSpecification;
use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};

use super::DeviceAddress;

/// How a request reaches a station.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Path {
    /// On this network, at its MAC.
    Local(MacAddr),
    /// Behind the router at `router`, on `network` at `mac`.
    Routed {
        router: MacAddr,
        network: u16,
        mac: MacAddr,
    },
}

impl Path {
    /// The path to `address`: its MAC for a local one, and for a routed one
    /// the router its row in `client`'s discovery table names.
    pub(crate) async fn to<T: TransportPort + 'static>(
        client: &BACnetClient<T>,
        address: &DeviceAddress,
    ) -> Result<Self, Error> {
        if address.is_local() {
            return Ok(Self::Local(MacAddr::from_slice(address.mac())));
        }
        match client.get_device_at(address.network(), address.mac()).await {
            Some(row) => Ok(Self::Routed {
                router: row.mac_address,
                network: address.network(),
                mac: MacAddr::from_slice(address.mac()),
            }),
            None => Err(Error::UnknownDevice {
                device: address.to_string(),
            }),
        }
    }

    /// One confirmed request, its answer's service data.
    pub(crate) async fn confirmed<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        service: ConfirmedServiceChoice,
        data: &[u8],
    ) -> Result<Bytes, Error> {
        match self {
            Self::Local(mac) => client.confirmed_request(mac, service, data).await,
            Self::Routed {
                router,
                network,
                mac,
            } => {
                client
                    .confirmed_request_routed(router, *network, mac, service, data)
                    .await
            }
        }
    }

    /// ReadProperty, its acknowledgement checked against the request as the
    /// client checks it.
    pub(crate) async fn read_property<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        match self {
            Self::Local(mac) => client.read_property(mac, object, property, index).await,
            Self::Routed {
                router,
                network,
                mac,
            } => {
                client
                    .read_property_routed(router, *network, mac, object, property, index)
                    .await
            }
        }
    }

    /// ReadPropertyMultiple.
    pub(crate) async fn read_property_multiple<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specs: Vec<ReadAccessSpecification>,
    ) -> Result<ReadPropertyMultipleACK, Error> {
        if let Self::Local(mac) = self {
            return client.read_property_multiple(mac, specs).await;
        }
        let mut buf = BytesMut::new();
        ReadPropertyMultipleRequest {
            list_of_read_access_specs: specs,
        }
        .encode(&mut buf)?;
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE, &buf)
            .await?;
        ReadPropertyMultipleACK::decode(&ack)
    }

    /// ReadRange, checked leniently, as a log page reads it.
    pub(crate) async fn read_range_lenient<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::READ_RANGE, &buf)
            .await?;
        ReadRangeReply::check(
            request,
            ReadRangeAck::decode(&ack)?,
            ReadRangeValidation::Lenient,
        )
    }

    /// GetEventInformation from after `after`.
    pub(crate) async fn event_information<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        after: Option<ObjectIdentifier>,
    ) -> Result<GetEventInformationAck, Error> {
        let mut buf = BytesMut::new();
        GetEventInformationRequest {
            last_received_object_identifier: after,
        }
        .encode(&mut buf);
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::GET_EVENT_INFORMATION, &buf)
            .await?;
        GetEventInformationAck::decode(&ack)
    }

    /// GetAlarmSummary.
    pub(crate) async fn alarm_summary<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
    ) -> Result<GetAlarmSummaryAck, Error> {
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::GET_ALARM_SUMMARY, &[])
            .await?;
        GetAlarmSummaryAck::decode(&ack)
    }

    /// GetEnrollmentSummary for `request`'s filters.
    pub(crate) async fn enrollment_summary<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        let mut buf = BytesMut::new();
        request.try_encode(&mut buf)?;
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY, &buf)
            .await?;
        GetEnrollmentSummaryAck::decode(&ack)
    }
}
