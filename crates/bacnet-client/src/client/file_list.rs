use super::*;

fn validate_read_range_ack(
    request: &bacnet_services::read_range::ReadRangeRequest,
    ack: &bacnet_services::read_range::ReadRangeAck,
) -> Result<(), Error> {
    if ack.object_identifier != request.object_identifier {
        return Err(Error::decoding(
            0,
            "ReadRange ACK object identifier does not match the request",
        ));
    }
    if ack.property_identifier != request.property_identifier {
        return Err(Error::decoding(
            0,
            "ReadRange ACK property identifier does not match the request",
        ));
    }
    if ack.property_array_index != request.property_array_index {
        return Err(Error::decoding(
            0,
            "ReadRange ACK array index does not match the request",
        ));
    }

    let sequence_range = matches!(
        request.range.as_ref(),
        Some(
            bacnet_services::read_range::RangeSpec::BySequenceNumber { .. }
                | bacnet_services::read_range::RangeSpec::ByTime { .. }
        )
    );
    let permits_first_sequence_number = sequence_range && ack.item_count > 0;
    if ack.first_sequence_number.is_some() && !permits_first_sequence_number {
        return Err(Error::decoding(
            0,
            "ReadRange ACK first sequence number is invalid for the request range",
        ));
    }

    Ok(())
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

    /// Acknowledge an alarm on a remote device.
    pub async fn acknowledge_alarm(
        &self,
        destination_mac: &[u8],
        acknowledging_process_identifier: u32,
        event_object_identifier: bacnet_types::primitives::ObjectIdentifier,
        event_state_acknowledged: u32,
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

    /// Read a range of items from a list or log-buffer property.
    pub async fn read_range(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        range: Option<bacnet_services::read_range::RangeSpec>,
    ) -> Result<bacnet_services::read_range::ReadRangeAck, Error> {
        use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};

        let request = ReadRangeRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            range,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let response_data = self
            .confirmed_request(destination_mac, ConfirmedServiceChoice::READ_RANGE, &buf)
            .await?;

        let ack = ReadRangeAck::decode(&response_data)?;
        validate_read_range_ack(&request, &ack)?;
        Ok(ack)
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
        request.encode(&mut buf);

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
        request.encode(&mut buf);

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
mod tests {
    use super::*;
    use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

    fn request(range: Option<RangeSpec>) -> ReadRangeRequest {
        ReadRangeRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: Some(1),
            range,
        }
    }

    fn ack(request: &ReadRangeRequest, item_count: u32, first: Option<u32>) -> ReadRangeAck {
        ReadRangeAck {
            object_identifier: request.object_identifier,
            property_identifier: request.property_identifier,
            property_array_index: request.property_array_index,
            result_flags: (true, true, false),
            item_count,
            item_data: Vec::new(),
            first_sequence_number: first,
        }
    }

    #[test]
    fn read_range_ack_must_echo_the_request() {
        let request = request(Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 1,
        }));
        let valid = ack(&request, 1, None);
        assert!(validate_read_range_ack(&request, &valid).is_ok());

        let mut wrong_object = valid.clone();
        wrong_object.object_identifier = ObjectIdentifier::new(ObjectType::TREND_LOG, 2).unwrap();
        assert!(validate_read_range_ack(&request, &wrong_object).is_err());

        let mut wrong_property = valid.clone();
        wrong_property.property_identifier = PropertyIdentifier::PRESENT_VALUE;
        assert!(validate_read_range_ack(&request, &wrong_property).is_err());

        let mut wrong_index = valid;
        wrong_index.property_array_index = Some(2);
        assert!(validate_read_range_ack(&request, &wrong_index).is_err());
    }

    #[test]
    fn first_sequence_number_must_match_the_requested_range() {
        let by_sequence = request(Some(RangeSpec::BySequenceNumber {
            reference_seq: 1,
            count: 1,
        }));
        assert!(validate_read_range_ack(&by_sequence, &ack(&by_sequence, 1, Some(1))).is_ok());
        assert!(validate_read_range_ack(&by_sequence, &ack(&by_sequence, 1, None)).is_ok());
        assert!(validate_read_range_ack(&by_sequence, &ack(&by_sequence, 0, None)).is_ok());
        assert!(validate_read_range_ack(&by_sequence, &ack(&by_sequence, 0, Some(1))).is_err());

        let by_time = request(Some(RangeSpec::ByTime {
            reference_time: (
                Date {
                    year: 126,
                    month: 3,
                    day: 1,
                    day_of_week: 7,
                },
                Time {
                    hour: 14,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                },
            ),
            count: 1,
        }));
        assert!(validate_read_range_ack(&by_time, &ack(&by_time, 1, Some(1))).is_ok());

        let by_position = request(Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 1,
        }));
        assert!(validate_read_range_ack(&by_position, &ack(&by_position, 1, Some(1))).is_err());
    }
}
