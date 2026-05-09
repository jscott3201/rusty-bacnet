use super::*;

/// Handle a ReadRange request.
///
/// Reads items from a list property with optional range filtering by
/// position, sequence number, or time.
pub fn handle_read_range(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};

    let request = ReadRangeRequest::decode(service_data)?;

    let object = db.get(&request.object_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    let value = object.read_property(request.property_identifier, request.property_array_index)?;

    let items = match value {
        PropertyValue::List(items) => items,
        _ => {
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::PROPERTY_IS_NOT_A_LIST.to_raw() as u32,
            });
        }
    };

    let total = items.len();

    let (selected, first_item, last_item) = match &request.range {
        None => (items, true, true),
        Some(RangeSpec::ByPosition {
            reference_index,
            count,
        }) => {
            let ref_idx = *reference_index as usize;
            let cnt = *count;
            if cnt == 0 || total == 0 || ref_idx == 0 || ref_idx > total {
                (Vec::new(), true, true)
            } else if cnt > 0 {
                let start = ref_idx - 1; // 1-based to 0-based
                let end = (start + cnt as usize).min(total);
                let first = start == 0;
                let last = end >= total;
                (items[start..end].to_vec(), first, last)
            } else {
                let abs_count = cnt.unsigned_abs() as usize;
                let end = ref_idx; // ref_idx is 1-based, used as exclusive end in 0-based
                let start = end.saturating_sub(abs_count);
                let first = start == 0;
                let last = end >= total;
                (items[start..end].to_vec(), first, last)
            }
        }
        Some(RangeSpec::BySequenceNumber {
            reference_seq,
            count,
        }) => {
            let ref_idx = *reference_seq as usize;
            let cnt = *count;
            if cnt == 0 || total == 0 {
                (Vec::new(), true, true)
            } else if cnt > 0 {
                let start = ref_idx.min(total).saturating_sub(1);
                let end = (start + cnt as usize).min(total);
                let first = start == 0;
                let last = end >= total;
                (items[start..end].to_vec(), first, last)
            } else {
                let abs_count = cnt.unsigned_abs() as usize;
                let end = ref_idx.min(total);
                let start = end.saturating_sub(abs_count);
                let first = start == 0;
                let last = end >= total;
                (items[start..end].to_vec(), first, last)
            }
        }
        Some(RangeSpec::ByTime { .. }) => {
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            });
        }
    };

    let mut item_data = BytesMut::new();
    let mut encoded_count: u32 = 0;
    for item in &selected {
        if encode_property_value(&mut item_data, item).is_err() {
            continue;
        }
        encoded_count += 1;
    }
    let item_count = encoded_count;

    let ack = ReadRangeAck {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        result_flags: (first_item, last_item, false),
        item_count,
        item_data: item_data.to_vec(),
        first_sequence_number: None,
    };

    ack.encode(buf);
    Ok(())
}

/// Handle an AtomicReadFile request.
pub fn handle_atomic_read_file(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::file::{
        AtomicReadFileAck, AtomicReadFileRequest, FileAccessMethod, FileReadAckMethod,
    };

    let request = AtomicReadFileRequest::decode(service_data)?;

    let object = db.get(&request.file_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    if request.file_identifier.object_type() != ObjectType::FILE {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32,
        });
    }

    let file_size = object
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .ok()
        .and_then(|v| match v {
            PropertyValue::Unsigned(n) => Some(n),
            _ => None,
        })
        .unwrap_or(0);

    match request.access {
        FileAccessMethod::Stream {
            file_start_position,
            requested_octet_count,
        } => {
            let start = file_start_position.max(0) as u64;
            let count = requested_octet_count as u64;
            let end_of_file = start + count >= file_size;

            let file_data = object
                .read_property(PropertyIdentifier::from_raw(PROP_FILE_DATA), None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::OctetString(d) => Some(d),
                    _ => None,
                })
                .unwrap_or_default();

            let s = start as usize;
            let e = (s + count as usize).min(file_data.len());
            let data = if s < file_data.len() {
                file_data[s..e].to_vec()
            } else {
                Vec::new()
            };

            let ack = AtomicReadFileAck {
                end_of_file,
                access: FileReadAckMethod::Stream {
                    file_start_position,
                    file_data: data,
                },
            };
            ack.encode(buf);
            Ok(())
        }
        FileAccessMethod::Record {
            file_start_record,
            requested_record_count,
        } => {
            let start = file_start_record.max(0) as usize;
            let count = requested_record_count as usize;

            let record_count = object
                .read_property(PropertyIdentifier::RECORD_COUNT, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::Unsigned(n) => Some(n as usize),
                    _ => None,
                })
                .unwrap_or(0);

            let end = (start + count).min(record_count);
            let end_of_file = end >= record_count;

            let records_data: Vec<Vec<u8>> = (start..end).map(|_| Vec::new()).collect();

            let ack = AtomicReadFileAck {
                end_of_file,
                access: FileReadAckMethod::Record {
                    file_start_record,
                    returned_record_count: records_data.len() as u32,
                    file_record_data: records_data,
                },
            };
            ack.encode(buf);
            Ok(())
        }
    }
}

/// Handle an AtomicWriteFile request.
pub fn handle_atomic_write_file(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::file::{
        AtomicWriteFileAck, AtomicWriteFileRequest, FileWriteAccessMethod, FileWriteAckMethod,
    };

    let request = AtomicWriteFileRequest::decode(service_data)?;

    let object = db.get(&request.file_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    if request.file_identifier.object_type() != ObjectType::FILE {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32,
        });
    }

    let read_only = object
        .read_property(PropertyIdentifier::READ_ONLY, None)
        .ok()
        .and_then(|v| match v {
            PropertyValue::Boolean(b) => Some(b),
            _ => None,
        })
        .unwrap_or(false);

    if read_only {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::FILE_ACCESS_DENIED.to_raw() as u32,
        });
    }

    match request.access {
        FileWriteAccessMethod::Stream {
            file_start_position,
            file_data,
        } => {
            let object = db
                .get_mut(&request.file_identifier)
                .ok_or(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
                })?;

            let mut existing = object
                .read_property(PropertyIdentifier::from_raw(PROP_FILE_DATA), None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::OctetString(d) => Some(d),
                    _ => None,
                })
                .unwrap_or_default();

            let start = file_start_position.max(0) as usize;
            if start + file_data.len() > existing.len() {
                existing.resize(start + file_data.len(), 0);
            }
            existing[start..start + file_data.len()].copy_from_slice(&file_data);

            object.write_property(
                PropertyIdentifier::from_raw(PROP_FILE_DATA),
                None,
                PropertyValue::OctetString(existing),
                None,
            )?;

            let ack = AtomicWriteFileAck {
                access: FileWriteAckMethod::Stream {
                    file_start_position,
                },
            };
            ack.encode(buf);
            Ok(())
        }
        FileWriteAccessMethod::Record {
            file_start_record, ..
        } => {
            let ack = AtomicWriteFileAck {
                access: FileWriteAckMethod::Record { file_start_record },
            };
            ack.encode(buf);
            Ok(())
        }
    }
}
