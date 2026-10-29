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

    /// Send exact caller-supplied alarm acknowledgment fields to a discovered device.
    ///
    /// Routes when needed and preserves both timestamps, confirmation and errors
    /// from [`Self::acknowledge_alarm_request`]. A missing device fails before sending.
    pub async fn acknowledge_alarm_request_from_device(
        &self,
        device_instance: u32,
        request: &bacnet_services::alarm_event::AcknowledgeAlarmRequest,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((network, address)) = routing {
            let mut buf = BytesMut::new();
            request.encode(&mut buf)?;
            self.confirmed_request_routed(
                &mac,
                network,
                &address,
                ConfirmedServiceChoice::ACKNOWLEDGE_ALARM,
                &buf,
            )
            .await?;
            Ok(())
        } else {
            self.acknowledge_alarm_request(&mac, request).await
        }
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

    /// Read raw file acknowledgement bytes from a discovered device.
    ///
    /// Routes stream and record access when needed. The device address is
    /// resolved once for this request; a missing device fails before sending.
    /// Use [`Self::atomic_read_file_decoded_from_device`] for a checked ACK.
    pub async fn atomic_read_file_from_device(
        &self,
        device_instance: u32,
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileAccessMethod,
    ) -> Result<Bytes, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((network, address)) = routing {
            let request = bacnet_services::file::AtomicReadFileRequest {
                file_identifier,
                access,
            };
            let mut buf = BytesMut::new();
            request.encode(&mut buf);
            self.confirmed_request_routed(
                &mac,
                network,
                &address,
                ConfirmedServiceChoice::ATOMIC_READ_FILE,
                &buf,
            )
            .await
        } else {
            self.atomic_read_file(&mac, file_identifier, access).await
        }
    }

    /// Read one checked file window from a discovered device, routing if needed.
    ///
    /// Preserves the access-arm and window checks of
    /// [`Self::atomic_read_file_decoded`]. Successive calls resolve separately;
    /// a sequence of windows is not an atomic snapshot of the remote file.
    pub async fn atomic_read_file_decoded_from_device(
        &self,
        device_instance: u32,
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileAccessMethod,
    ) -> Result<bacnet_services::file::AtomicReadFileAck, Error> {
        let requested_access = access.clone();
        let response = self
            .atomic_read_file_from_device(device_instance, file_identifier, access)
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

    /// Write stream or record data to a discovered device, routing if needed.
    ///
    /// Returns the raw acknowledgement, as in [`Self::atomic_write_file`].
    /// A missing device fails before sending.
    pub async fn atomic_write_file_from_device(
        &self,
        device_instance: u32,
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileWriteAccessMethod,
    ) -> Result<Bytes, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((network, address)) = routing {
            let request = bacnet_services::file::AtomicWriteFileRequest {
                file_identifier,
                access,
            };
            let mut buf = BytesMut::new();
            request.encode(&mut buf);
            self.confirmed_request_routed(
                &mac,
                network,
                &address,
                ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
                &buf,
            )
            .await
        } else {
            self.atomic_write_file(&mac, file_identifier, access).await
        }
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

#[cfg(test)]
#[path = "routed_file_tests.rs"]
mod routed_tests;
