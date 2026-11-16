use super::service::{ListChange, Raw};
use super::*;
#[cfg(test)]
use crate::acks::validate_atomic_read_file_ack;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a caller-supplied AcknowledgeAlarm request to the station `to`
    /// without fabricating fields.
    pub async fn acknowledge_alarm(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &bacnet_services::alarm_event::AcknowledgeAlarmRequest,
    ) -> Result<(), Error> {
        self.call(to, request).await
    }

    /// Read file data from the station `to` (stream or record access), the
    /// acknowledgement's service data returned raw.
    pub async fn atomic_read_file(
        &self,
        to: &(impl ToStation + ?Sized),
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileAccessMethod,
    ) -> Result<Bytes, Error> {
        let request = bacnet_services::file::AtomicReadFileRequest {
            file_identifier,
            access,
        };
        self.call(to, Raw(request)).await
    }

    /// Read and strictly decode one AtomicReadFile ACK window from the
    /// station `to`.
    ///
    /// This additive typed boundary validates that the ACK uses the requested
    /// stream/record access arm and does not return more octets or records than
    /// requested. Use [`Self::atomic_read_file`] when the raw encoded service
    /// payload is required for compatibility.
    pub async fn atomic_read_file_decoded(
        &self,
        to: &(impl ToStation + ?Sized),
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileAccessMethod,
    ) -> Result<bacnet_services::file::AtomicReadFileAck, Error> {
        let request = bacnet_services::file::AtomicReadFileRequest {
            file_identifier,
            access,
        };
        self.call(to, request).await
    }

    /// Write file data to the station `to` (stream or record access), the
    /// acknowledgement's service data returned raw.
    pub async fn atomic_write_file(
        &self,
        to: &(impl ToStation + ?Sized),
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileWriteAccessMethod,
    ) -> Result<Bytes, Error> {
        let request = bacnet_services::file::AtomicWriteFileRequest {
            file_identifier,
            access,
        };
        self.call(to, request).await
    }

    /// Add elements to a list property on the station `to`.
    ///
    /// Rejects index zero, empty elements and malformed tag framing before
    /// transaction admission or traffic. Element datatype remains target-owned.
    pub async fn add_list_element(
        &self,
        to: &(impl ToStation + ?Sized),
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        list_of_elements: Vec<u8>,
    ) -> Result<(), Error> {
        let request = bacnet_services::list_manipulation::ListElementRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            list_of_elements,
        };
        let choice = ConfirmedServiceChoice::ADD_LIST_ELEMENT;
        self.call(to, ListChange { choice, request }).await
    }

    /// Remove elements from a list property on the station `to`.
    ///
    /// Rejects index zero, empty elements and malformed tag framing before
    /// transaction admission or traffic. Element datatype remains target-owned.
    pub async fn remove_list_element(
        &self,
        to: &(impl ToStation + ?Sized),
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        list_of_elements: Vec<u8>,
    ) -> Result<(), Error> {
        let request = bacnet_services::list_manipulation::ListElementRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            list_of_elements,
        };
        let choice = ConfirmedServiceChoice::REMOVE_LIST_ELEMENT;
        self.call(to, ListChange { choice, request }).await
    }
}

#[cfg(test)]
#[path = "file_list_decoded_tests.rs"]
mod decoded_tests;

#[cfg(test)]
#[path = "routed_file_tests.rs"]
mod routed_tests;
