//! The services a one-shot command sends to a [`Destination`], each to its
//! address through the client's address entry points
//! ([`BACnetClient::confirmed_request_to`] and
//! [`BACnetClient::unconfirmed_request_to`]), encoded and checked as the
//! client's typed calls encode and check them (#2560).

use bacnet_client::acks;
use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{self, LogCursor, LogPage, LogRequester, unsigned_value};
use bacnet_services::alarm_event::{
    AcknowledgeAlarmRequest, GetEventInformationAck, GetEventInformationRequest,
};
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::device_mgmt::{
    DeviceCommunicationControlRequest, ReinitializeDeviceRequest, TimeSynchronizationRequest,
};
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_services::file::{
    AtomicReadFileAck, AtomicReadFileRequest, AtomicWriteFileRequest, FileAccessMethod,
    FileWriteAccessMethod,
};
use bacnet_services::object_mgmt::{
    CreateObjectAck, CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier,
};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::read_range::{
    ReadRangeAck, ReadRangeReply, ReadRangeRequest, ReadRangeValidation,
};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetAddress, ReadAccessSpecification};
use bacnet_types::enums::{
    ConfirmedServiceChoice, EnableDisable, PropertyIdentifier, ReinitializedState,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::{Bytes, BytesMut};

use super::Destination;

impl Destination {
    /// The address as the client takes it.
    fn station(&self) -> BACnetAddress {
        (&self.address).into()
    }

    /// One confirmed request to the station, its answer's service data.
    async fn confirmed<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        service: ConfirmedServiceChoice,
        data: &[u8],
    ) -> Result<Bytes, Error> {
        client
            .confirmed_request_to(&self.station(), service, data)
            .await
    }

    /// One checked stream or record file window.
    pub async fn atomic_read_file<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        file: ObjectIdentifier,
        access: FileAccessMethod,
    ) -> Result<AtomicReadFileAck, Error> {
        let request = AtomicReadFileRequest {
            file_identifier: file,
            access: access.clone(),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::ATOMIC_READ_FILE, &buf)
            .await?;
        acks::atomic_read_file(&access, &ack)
    }

    /// AtomicWriteFile, retaining its raw service acknowledgement.
    pub async fn atomic_write_file<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        file: ObjectIdentifier,
        access: FileWriteAccessMethod,
    ) -> Result<Bytes, Error> {
        let request = AtomicWriteFileRequest {
            file_identifier: file,
            access,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        self.confirmed(client, ConfirmedServiceChoice::ATOMIC_WRITE_FILE, &buf)
            .await
    }

    /// Create an object, returning the identifier its acknowledgement names.
    pub async fn create_object<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specifier: ObjectSpecifier,
        initial_values: Vec<BACnetPropertyValue>,
    ) -> Result<ObjectIdentifier, Error> {
        let request = CreateObjectRequest {
            object_specifier: specifier,
            list_of_initial_values: initial_values,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::CREATE_OBJECT, &buf)
            .await?;
        CreateObjectAck::decode(&ack).map(|ack| ack.object_identifier)
    }

    /// Delete the identified object.
    pub async fn delete_object<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: object,
        }
        .encode(&mut buf);
        self.confirmed(client, ConfirmedServiceChoice::DELETE_OBJECT, &buf)
            .await
            .map(drop)
    }

    /// Acknowledge an alarm with the caller's exact request.
    pub async fn acknowledge_alarm<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &AcknowledgeAlarmRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        self.confirmed(client, ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, &buf)
            .await
            .map(drop)
    }

    /// DeviceCommunicationControl.
    pub async fn device_communication_control<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        state: EnableDisable,
        duration: Option<u16>,
        password: Option<String>,
    ) -> Result<(), Error> {
        let request = DeviceCommunicationControlRequest {
            time_duration: duration,
            enable_disable: state,
            password,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        let service = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
        self.confirmed(client, service, &buf).await.map(drop)
    }

    /// ReinitializeDevice.
    pub async fn reinitialize_device<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        state: ReinitializedState,
        password: Option<String>,
    ) -> Result<(), Error> {
        let request = ReinitializeDeviceRequest {
            reinitialized_state: state,
            password,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        self.confirmed(client, ConfirmedServiceChoice::REINITIALIZE_DEVICE, &buf)
            .await
            .map(drop)
    }

    /// Send the selected unconfirmed time service once.
    pub async fn time_synchronization<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        date: Date,
        time: Time,
        utc: bool,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        TimeSynchronizationRequest { date, time }.encode(&mut buf);
        let service = if utc {
            UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
        } else {
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
        };
        client
            .unconfirmed_request_to(&self.station(), service, &buf)
            .await
    }

    /// GetEventInformation from after `last`, decoded.
    pub async fn get_event_information<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        last: Option<ObjectIdentifier>,
    ) -> Result<GetEventInformationAck, Error> {
        let mut buf = BytesMut::new();
        GetEventInformationRequest {
            last_received_object_identifier: last,
        }
        .encode(&mut buf);
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::GET_EVENT_INFORMATION, &buf)
            .await?;
        GetEventInformationAck::decode(&ack)
    }

    /// GetAlarmSummary, decoded.
    pub async fn get_alarm_summary<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
    ) -> Result<GetAlarmSummaryAck, Error> {
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::GET_ALARM_SUMMARY, &[])
            .await?;
        GetAlarmSummaryAck::decode(&ack)
    }

    /// GetEnrollmentSummary with the caller's filters, decoded.
    pub async fn get_enrollment_summary<T: TransportPort + 'static>(
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

    /// ReadRange with the selected acknowledgement validation.
    pub async fn read_range<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &ReadRangeRequest,
        validation: ReadRangeValidation,
    ) -> Result<ReadRangeReply, Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::READ_RANGE, &buf)
            .await?;
        ReadRangeReply::check(request, ReadRangeAck::decode(&ack)?, validation)
    }

    /// One log page, including all metadata and range subrequests.
    pub async fn read_log_page<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        log: ObjectIdentifier,
        cursor: LogCursor,
        page_size: u16,
    ) -> Result<LogPage, Error> {
        let at = At { client, to: self };
        log_reader::read_log_page(&at, self.address.mac(), log, cursor, page_size).await
    }

    /// ReadProperty.
    pub async fn read_property<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        let request = ReadPropertyRequest {
            object_identifier: object,
            property_identifier: property,
            property_array_index: index,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let ack = self
            .confirmed(client, ConfirmedServiceChoice::READ_PROPERTY, &buf)
            .await?;
        acks::read_property(&request, &ack)
    }

    /// ReadPropertyMultiple.
    pub async fn read_property_multiple<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specs: Vec<ReadAccessSpecification>,
    ) -> Result<ReadPropertyMultipleACK, Error> {
        let mut buf = BytesMut::new();
        ReadPropertyMultipleRequest {
            list_of_read_access_specs: specs,
        }
        .encode(&mut buf)?;
        let service = ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE;
        let ack = self.confirmed(client, service, &buf).await?;
        ReadPropertyMultipleACK::decode(&ack)
    }

    /// WriteProperty of an encoded value.
    pub async fn write_property<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        let request = WritePropertyRequest {
            object_identifier: object,
            property_identifier: property,
            property_array_index: index,
            property_value: value,
            priority,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        self.confirmed(client, ConfirmedServiceChoice::WRITE_PROPERTY, &buf)
            .await
            .map(drop)
    }

    /// WritePropertyMultiple.
    pub async fn write_property_multiple<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specs: Vec<WriteAccessSpecification>,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: specs,
        }
        .encode(&mut buf)?;
        let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
        self.confirmed(client, service, &buf).await.map(drop)
    }

    /// SubscribeCOV for `object`, from process `process`.
    pub async fn subscribe_cov<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        process: u32,
        object: ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), Error> {
        let request = SubscribeCOVRequest {
            subscriber_process_identifier: process,
            monitored_object_identifier: object,
            issue_confirmed_notifications: Some(confirmed),
            lifetime,
        };
        self.cov(client, &request).await
    }

    /// Cancel process `process`'s subscription to `object`.
    pub async fn unsubscribe_cov<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        process: u32,
        object: ObjectIdentifier,
    ) -> Result<(), Error> {
        let request = SubscribeCOVRequest {
            subscriber_process_identifier: process,
            monitored_object_identifier: object,
            issue_confirmed_notifications: None,
            lifetime: None,
        };
        self.cov(client, &request).await
    }

    /// One SubscribeCOV, a subscription or its cancellation.
    async fn cov<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &SubscribeCOVRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;
        self.confirmed(client, ConfirmedServiceChoice::SUBSCRIBE_COV, &buf)
            .await
            .map(drop)
    }
}

/// A paged log read's requests, to one destination: the MAC the reader
/// passes is the destination's own, and its address decides the route.
struct At<'a, T: TransportPort + 'static> {
    client: &'a BACnetClient<T>,
    to: &'a Destination,
}

impl<T: TransportPort + 'static> LogRequester for At<'_, T> {
    async fn read_unsigned(
        &self,
        _mac: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        let ack = self
            .to
            .read_property(self.client, log, property, None)
            .await?;
        unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        _mac: &[u8],
        request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        self.to
            .read_range(self.client, request, ReadRangeValidation::Lenient)
            .await
    }
}
