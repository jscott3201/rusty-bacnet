use super::*;

/// Handle a GetEventInformation request.
///
/// Returns event summaries for objects whose event_state is not NORMAL.
/// Supports pagination via `last_received_object_identifier`.
pub fn handle_get_event_information(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    const MAX_SUMMARIES: usize = 25;

    let request = GetEventInformationRequest::decode(service_data)?;

    let mut summaries = Vec::new();
    let mut skipping = request.last_received_object_identifier.is_some();
    let mut more_events = false;

    for (oid, object) in db.iter_objects() {
        if skipping {
            if Some(oid) == request.last_received_object_identifier {
                skipping = false;
            }
            continue;
        }

        if let Ok(PropertyValue::Enumerated(state)) =
            object.read_property(PropertyIdentifier::EVENT_STATE, None)
        {
            if state != bacnet_types::enums::EventState::NORMAL.to_raw() {
                if summaries.len() >= MAX_SUMMARIES {
                    more_events = true;
                    break;
                }

                let notification_class = object
                    .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::Unsigned(n) => Some(n as u32),
                        _ => None,
                    })
                    .unwrap_or(0);

                let event_enable = object
                    .read_property(PropertyIdentifier::EVENT_ENABLE, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::BitString { data, .. } => data.first().map(|b| b >> 5),
                        _ => None,
                    })
                    .unwrap_or(0x07);

                let notify_type = object
                    .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::Enumerated(n) => Some(n),
                        _ => None,
                    })
                    .unwrap_or(0);

                let event_priorities =
                    ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, notification_class)
                        .ok()
                        .and_then(|nc_oid| db.get(&nc_oid))
                        .and_then(|nc_obj| {
                            nc_obj
                                .read_property(PropertyIdentifier::PRIORITY, None)
                                .ok()
                        })
                        .and_then(|v| match v {
                            PropertyValue::OctetString(bytes) if bytes.len() == 3 => {
                                Some([bytes[0] as u32, bytes[1] as u32, bytes[2] as u32])
                            }
                            _ => None,
                        })
                        .unwrap_or([0, 0, 0]);

                let acked = object
                    .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::BitString { data, .. } => data.first().map(|b| b >> 5),
                        _ => None,
                    })
                    .unwrap_or(0x07);

                let event_timestamps = object
                    .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::List(items) if items.len() == 3 => None,
                        _ => None,
                    })
                    .unwrap_or([
                        BACnetTimeStamp::SequenceNumber(0),
                        BACnetTimeStamp::SequenceNumber(0),
                        BACnetTimeStamp::SequenceNumber(0),
                    ]);

                summaries.push(EventSummary {
                    object_identifier: oid,
                    event_state: state,
                    acknowledged_transitions: acked,
                    event_timestamps,
                    notify_type,
                    event_enable,
                    event_priorities,
                    notification_class,
                });
            }
        }
    }

    let ack = GetEventInformationAck {
        list_of_event_summaries: summaries,
        more_events,
    };

    ack.encode(buf);
    Ok(())
}

/// Handle an AcknowledgeAlarm request.
///
/// Updates the acknowledged_transitions bitfield on the referenced object.
pub fn handle_acknowledge_alarm(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    let request = AcknowledgeAlarmRequest::decode(service_data)?;

    let transition_bit: u8 = match EventState::from_raw(request.event_state_acknowledged) {
        s if s == EventState::NORMAL => 0x04, // TO_NORMAL
        s if s == EventState::FAULT => 0x02,  // TO_FAULT
        _ => 0x01,                            // TO_OFFNORMAL
    };

    let object = db
        .get_mut(&request.event_object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    object.acknowledge_alarm(transition_bit)?;

    Ok(())
}
/// Handle a GetEnrollmentSummary request.
///
/// Returns event-enrollment objects that match the filter criteria.
pub fn handle_get_enrollment_summary(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::enrollment_summary::{
        EnrollmentSummaryEntry, GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest,
    };

    let request = GetEnrollmentSummaryRequest::decode(service_data)?;

    let mut entries = Vec::new();
    for (_oid, object) in db.iter_objects() {
        let oid = object.object_identifier();

        let event_state = object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .ok()
            .and_then(|v| match v {
                PropertyValue::Enumerated(e) => Some(e),
                _ => None,
            })
            .unwrap_or(0);

        if let Some(filter_state) = request.event_state_filter {
            if event_state != filter_state.to_raw() {
                continue;
            }
        }

        let notification_class = object
            .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
            .ok()
            .and_then(|v| match v {
                PropertyValue::Unsigned(n) => Some(n as u16),
                _ => None,
            })
            .unwrap_or(0);

        if let Some(nc_filter) = request.notification_class_filter {
            if notification_class != nc_filter {
                continue;
            }
        }

        if let Some(ref pf) = request.priority_filter {
            // Look up priority from the notification class object for the current event state
            let priority = ObjectIdentifier::new(
                bacnet_types::enums::ObjectType::NOTIFICATION_CLASS,
                notification_class as u32,
            )
            .ok()
            .and_then(|nc_oid| db.get(&nc_oid))
            .and_then(|nc_obj| {
                // PRIORITY property returns an array of 3 priorities (TO_OFFNORMAL, TO_FAULT, TO_NORMAL)
                nc_obj
                    .read_property(PropertyIdentifier::PRIORITY, Some(1))
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::Unsigned(p) => Some(p as u8),
                        _ => None,
                    })
            })
            .unwrap_or(0);
            if priority < pf.min_priority || priority > pf.max_priority {
                continue;
            }
        }

        if event_state == 0 && request.event_state_filter.is_none() {
            continue;
        }

        entries.push(EnrollmentSummaryEntry {
            object_identifier: oid,
            event_type: bacnet_types::enums::EventType::CHANGE_OF_STATE,
            event_state: bacnet_types::enums::EventState::from_raw(event_state),
            priority: 0,
            notification_class,
        });
    }

    let ack = GetEnrollmentSummaryAck { entries };
    ack.encode(buf);
    Ok(())
}

/// Handle a ConfirmedTextMessage request.
///
/// Returns the decoded request for the application layer.
pub fn handle_text_message(
    service_data: &[u8],
) -> Result<bacnet_services::text_message::TextMessageRequest, Error> {
    bacnet_services::text_message::TextMessageRequest::decode(service_data)
}

/// Handle a LifeSafetyOperation request.
///
/// Decodes the request and returns Ok(()) for SimpleACK.
pub fn handle_life_safety_operation(service_data: &[u8]) -> Result<(), Error> {
    let _request = bacnet_services::life_safety::LifeSafetyOperationRequest::decode(service_data)?;
    Ok(())
}

/// Handle a GetAlarmSummary request.
///
/// Returns objects with event_state != NORMAL.
pub fn handle_get_alarm_summary(db: &ObjectDatabase, buf: &mut BytesMut) -> Result<(), Error> {
    use bacnet_services::alarm_summary::{AlarmSummaryEntry, GetAlarmSummaryAck};

    let mut entries = Vec::new();
    for (_oid, object) in db.iter_objects() {
        let oid = object.object_identifier();
        let event_state = object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .ok()
            .and_then(|v| match v {
                PropertyValue::Enumerated(e) => Some(e),
                _ => None,
            })
            .unwrap_or(0);

        if event_state != 0 {
            let acked = object
                .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::BitString {
                        unused_bits, data, ..
                    } => Some((unused_bits, data)),
                    _ => None,
                })
                .unwrap_or((5, vec![0xE0])); // all acknowledged by default

            entries.push(AlarmSummaryEntry {
                object_identifier: oid,
                alarm_state: bacnet_types::enums::EventState::from_raw(event_state),
                acknowledged_transitions: acked,
            });
        }
    }

    let ack = GetAlarmSummaryAck { entries };
    ack.encode(buf);
    Ok(())
}
