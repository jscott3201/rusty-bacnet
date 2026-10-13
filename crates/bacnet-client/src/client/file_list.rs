use super::*;

fn validate_atomic_read_file_ack(
    request: &bacnet_services::file::FileAccessMethod,
    ack: &bacnet_services::file::AtomicReadFileAck,
) -> Result<(), Error> {
    use bacnet_services::file::{FileAccessMethod, FileReadAckMethod};

    match (request, &ack.access) {
        (
            FileAccessMethod::Stream {
                requested_octet_count,
                ..
            },
            FileReadAckMethod::Stream { file_data, .. },
        ) if file_data.len() <= *requested_octet_count as usize => Ok(()),
        (FileAccessMethod::Stream { .. }, FileReadAckMethod::Stream { .. }) => Err(
            Error::decoding(0, "AtomicReadFile ACK exceeds the requested octet window"),
        ),
        (
            FileAccessMethod::Record {
                requested_record_count,
                ..
            },
            FileReadAckMethod::Record {
                returned_record_count,
                file_record_data,
                ..
            },
        ) if returned_record_count <= requested_record_count
            && *returned_record_count as usize == file_record_data.len() =>
        {
            Ok(())
        }
        (FileAccessMethod::Record { .. }, FileReadAckMethod::Record { .. }) => Err(
            Error::decoding(0, "AtomicReadFile ACK exceeds the requested record window"),
        ),
        _ => Err(Error::decoding(
            0,
            "AtomicReadFile ACK access method does not match the request",
        )),
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Get event information from a remote device.
    pub async fn get_event_information(
        &self,
        destination_mac: &[u8],
        last_received_object_identifier: Option<bacnet_types::primitives::ObjectIdentifier>,
    ) -> Result<Bytes, Error> {
        use bacnet_services::alarm_event::GetEventInformationRequest;

        let request = GetEventInformationRequest {
            last_received_object_identifier,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.confirmed_request(
            destination_mac,
            ConfirmedServiceChoice::GET_EVENT_INFORMATION,
            &buf,
        )
        .await
    }

    /// Send a caller-supplied AcknowledgeAlarm request without fabricating fields.
    pub async fn acknowledge_alarm_request(
        &self,
        destination_mac: &[u8],
        request: &bacnet_services::alarm_event::AcknowledgeAlarmRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::ACKNOWLEDGE_ALARM,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Acknowledge an alarm on a remote device.
    ///
    /// This compatibility helper fabricates sequence-zero timestamps, which
    /// are not a valid general correlation mechanism. Use
    /// [`Self::acknowledge_alarm_request`] with the original notification's
    /// exact timestamp and a caller-selected acknowledgment time.
    #[deprecated(
        note = "use acknowledge_alarm_request with caller-supplied correlation timestamps"
    )]
    pub async fn acknowledge_alarm(
        &self,
        destination_mac: &[u8],
        acknowledging_process_identifier: u32,
        event_object_identifier: bacnet_types::primitives::ObjectIdentifier,
        event_state_acknowledged: bacnet_types::enums::EventState,
        acknowledgment_source: &str,
    ) -> Result<(), Error> {
        use bacnet_services::alarm_event::AcknowledgeAlarmRequest;

        let request = AcknowledgeAlarmRequest {
            acknowledging_process_identifier,
            event_object_identifier,
            event_state_acknowledged,
            timestamp: bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(0),
            acknowledgment_source: acknowledgment_source.to_string(),
            time_of_acknowledgment: bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(0),
        };
        self.acknowledge_alarm_request(destination_mac, &request)
            .await
    }

    /// Read file data from a remote device (stream or record access).
    pub async fn atomic_read_file(
        &self,
        destination_mac: &[u8],
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileAccessMethod,
    ) -> Result<Bytes, Error> {
        use bacnet_services::file::AtomicReadFileRequest;

        let request = AtomicReadFileRequest {
            file_identifier,
            access,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.confirmed_request(
            destination_mac,
            ConfirmedServiceChoice::ATOMIC_READ_FILE,
            &buf,
        )
        .await
    }

    /// Read and strictly decode one AtomicReadFile ACK window.
    ///
    /// This additive typed boundary validates that the ACK uses the requested
    /// stream/record access arm and does not return more octets or records than
    /// requested. Use [`Self::atomic_read_file`] when the raw encoded service
    /// payload is required for compatibility.
    pub async fn atomic_read_file_decoded(
        &self,
        destination_mac: &[u8],
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileAccessMethod,
    ) -> Result<bacnet_services::file::AtomicReadFileAck, Error> {
        let requested_access = access.clone();
        let response = self
            .atomic_read_file(destination_mac, file_identifier, access)
            .await?;
        let ack = bacnet_services::file::AtomicReadFileAck::decode(&response)?;
        validate_atomic_read_file_ack(&requested_access, &ack)?;
        Ok(ack)
    }

    /// Write file data to a remote device (stream or record access).
    pub async fn atomic_write_file(
        &self,
        destination_mac: &[u8],
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileWriteAccessMethod,
    ) -> Result<Bytes, Error> {
        use bacnet_services::file::AtomicWriteFileRequest;

        let request = AtomicWriteFileRequest {
            file_identifier,
            access,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.confirmed_request(
            destination_mac,
            ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
            &buf,
        )
        .await
    }

    /// Add elements to a list property on a remote device.
    ///
    /// Rejects index zero, empty elements and malformed tag framing before
    /// transaction admission or traffic. Element datatype remains target-owned.
    pub async fn add_list_element(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        list_of_elements: Vec<u8>,
    ) -> Result<(), Error> {
        use bacnet_services::list_manipulation::ListElementRequest;

        let request = ListElementRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            list_of_elements,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::ADD_LIST_ELEMENT,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Remove elements from a list property on a remote device.
    ///
    /// Rejects index zero, empty elements and malformed tag framing before
    /// transaction admission or traffic. Element datatype remains target-owned.
    pub async fn remove_list_element(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        list_of_elements: Vec<u8>,
    ) -> Result<(), Error> {
        use bacnet_services::list_manipulation::ListElementRequest;

        let request = ListElementRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            list_of_elements,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
                &buf,
            )
            .await?;

        Ok(())
    }
}

#[cfg(test)]
#[path = "file_list_decoded_tests.rs"]
mod decoded_tests;
