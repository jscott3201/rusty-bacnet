//! The services a one-shot command sends to a [`Destination`]: each one the
//! client's typed call to the destination's address (#2560, #2588), so the
//! CLI encodes and checks nothing itself.

use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{LogCursor, LogPage};
use bacnet_services::alarm_event::{AcknowledgeAlarmRequest, GetEventInformationAck};
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_services::file::{AtomicReadFileAck, FileAccessMethod, FileWriteAccessMethod};
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::read_range::{ReadRangeReply, ReadRangeRequest, ReadRangeValidation};
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetAddress, ReadAccessSpecification};
use bacnet_types::enums::{EnableDisable, PropertyIdentifier, ReinitializedState};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::Bytes;

use super::Destination;

impl Destination {
    /// The address as the client takes it.
    fn station(&self) -> BACnetAddress {
        (&self.address).into()
    }

    /// One checked stream or record file window.
    pub async fn atomic_read_file<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        file: ObjectIdentifier,
        access: FileAccessMethod,
    ) -> Result<AtomicReadFileAck, Error> {
        client
            .atomic_read_file_decoded(&self.station(), file, access)
            .await
    }

    /// AtomicWriteFile, retaining its raw service acknowledgement.
    pub async fn atomic_write_file<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        file: ObjectIdentifier,
        access: FileWriteAccessMethod,
    ) -> Result<Bytes, Error> {
        client
            .atomic_write_file(&self.station(), file, access)
            .await
    }

    /// Create an object, returning the identifier its acknowledgement names.
    pub async fn create_object<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specifier: ObjectSpecifier,
        initial_values: Vec<BACnetPropertyValue>,
    ) -> Result<ObjectIdentifier, Error> {
        client
            .create_object(&self.station(), specifier, initial_values)
            .await
    }

    /// Delete the identified object.
    pub async fn delete_object<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
    ) -> Result<(), Error> {
        client.delete_object(&self.station(), object).await
    }

    /// Acknowledge an alarm with the caller's exact request.
    pub async fn acknowledge_alarm<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &AcknowledgeAlarmRequest,
    ) -> Result<(), Error> {
        client.acknowledge_alarm(&self.station(), request).await
    }

    /// DeviceCommunicationControl.
    pub async fn device_communication_control<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        state: EnableDisable,
        duration: Option<u16>,
        password: Option<String>,
    ) -> Result<(), Error> {
        client
            .device_communication_control(&self.station(), state, duration, password)
            .await
    }

    /// ReinitializeDevice.
    pub async fn reinitialize_device<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        state: ReinitializedState,
        password: Option<String>,
    ) -> Result<(), Error> {
        client
            .reinitialize_device(&self.station(), state, password)
            .await
    }

    /// Send the selected unconfirmed time service once.
    pub async fn time_synchronization<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        date: Date,
        time: Time,
        utc: bool,
    ) -> Result<(), Error> {
        let to = self.station();
        if utc {
            client.utc_time_synchronization(&to, date, time).await
        } else {
            client.time_synchronization(&to, date, time).await
        }
    }

    /// GetEventInformation from after `last`, decoded.
    pub async fn get_event_information<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        last: Option<ObjectIdentifier>,
    ) -> Result<GetEventInformationAck, Error> {
        client.get_event_information(&self.station(), last).await
    }

    /// GetAlarmSummary, decoded.
    pub async fn get_alarm_summary<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
    ) -> Result<GetAlarmSummaryAck, Error> {
        client.get_alarm_summary(&self.station()).await
    }

    /// GetEnrollmentSummary with the caller's filters, decoded.
    pub async fn get_enrollment_summary<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        client
            .get_enrollment_summary(&self.station(), request)
            .await
    }

    /// ReadRange with the selected acknowledgement validation.
    pub async fn read_range<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        request: &ReadRangeRequest,
        validation: ReadRangeValidation,
    ) -> Result<ReadRangeReply, Error> {
        client
            .read_range_with(&self.station(), request, validation)
            .await
    }

    /// One log page, including all metadata and range subrequests.
    pub async fn read_log_page<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        log: ObjectIdentifier,
        cursor: LogCursor,
        page_size: u16,
    ) -> Result<LogPage, Error> {
        client
            .read_log_page(&self.station(), log, cursor, page_size)
            .await
    }

    /// ReadProperty.
    pub async fn read_property<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        client
            .read_property(&self.station(), object, property, index)
            .await
    }

    /// ReadPropertyMultiple.
    pub async fn read_property_multiple<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specs: Vec<ReadAccessSpecification>,
    ) -> Result<ReadPropertyMultipleACK, Error> {
        client.read_property_multiple(&self.station(), specs).await
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
        client
            .write_property(&self.station(), object, property, index, value, priority)
            .await
    }

    /// WritePropertyMultiple.
    pub async fn write_property_multiple<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        specs: Vec<WriteAccessSpecification>,
    ) -> Result<(), Error> {
        client.write_property_multiple(&self.station(), specs).await
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
        client
            .subscribe_cov(&self.station(), process, object, confirmed, lifetime)
            .await
    }

    /// Cancel process `process`'s subscription to `object`.
    pub async fn unsubscribe_cov<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        process: u32,
        object: ObjectIdentifier,
    ) -> Result<(), Error> {
        client
            .unsubscribe_cov(&self.station(), process, object)
            .await
    }
}
