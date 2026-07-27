//! GetEventInformation service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;

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

        // Clause 12.12: Event_Detection_Enable controls whether the object is
        // considered by the event-summarization services. Checked after the
        // pagination skip so a disabled object cannot break resumption.
        if !super::event_detection_enabled(object) {
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
