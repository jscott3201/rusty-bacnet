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
    let mut more_events = false;
    let cursor = request
        .last_received_object_identifier
        .map(|identifier| identifier.encode());
    let mut object_identifiers = db.list_objects();
    object_identifiers.sort_unstable_by_key(ObjectIdentifier::encode);
    let notification_class_identifiers = db.find_by_type(ObjectType::NOTIFICATION_CLASS);

    for oid in object_identifiers {
        if cursor.is_some_and(|cursor| oid.encode() <= cursor) {
            continue;
        }
        let Some(object) = db.get(&oid) else {
            continue;
        };

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
                        PropertyValue::Unsigned(n) => u32::try_from(n).ok(),
                        _ => None,
                    })
                    .unwrap_or(0);

                let event_enable = object
                    .read_property(PropertyIdentifier::EVENT_ENABLE, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::BitString { data, .. } => {
                            Some(bacnet_types::bitstring::unpack_octet(&data, 3))
                        }
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

                let matching_notification_class =
                    notification_class_identifiers.iter().find_map(|nc_oid| {
                        let nc_obj = db.get(nc_oid)?;
                        matches!(
                            nc_obj.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None),
                            Ok(PropertyValue::Unsigned(class_number))
                                if class_number == u64::from(notification_class)
                        )
                        .then_some(nc_obj)
                    });
                let event_priorities = matching_notification_class
                    .and_then(|nc_obj| {
                        nc_obj
                            .read_property(PropertyIdentifier::PRIORITY, None)
                            .ok()
                    })
                    .and_then(|v| match v {
                        PropertyValue::List(items) => {
                            let [
                                PropertyValue::Unsigned(offnormal),
                                PropertyValue::Unsigned(fault),
                                PropertyValue::Unsigned(normal),
                            ] = items.as_slice()
                            else {
                                return None;
                            };
                            Some([
                                u32::try_from(*offnormal).ok()?,
                                u32::try_from(*fault).ok()?,
                                u32::try_from(*normal).ok()?,
                            ])
                        }
                        _ => None,
                    })
                    .unwrap_or([0, 0, 0]);

                let acked = object
                    .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::BitString { data, .. } => {
                            Some(bacnet_types::bitstring::unpack_octet(&data, 3))
                        }
                        _ => None,
                    })
                    .unwrap_or(0x07);

                let event_timestamps = object
                    .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::List(items) => {
                            let [
                                PropertyValue::Unsigned(offnormal),
                                PropertyValue::Unsigned(fault),
                                PropertyValue::Unsigned(normal),
                            ] = items.as_slice()
                            else {
                                return None;
                            };
                            let offnormal = u16::try_from(*offnormal).ok()?;
                            let fault = u16::try_from(*fault).ok()?;
                            let normal = u16::try_from(*normal).ok()?;
                            Some([
                                BACnetTimeStamp::SequenceNumber(offnormal),
                                BACnetTimeStamp::SequenceNumber(fault),
                                BACnetTimeStamp::SequenceNumber(normal),
                            ])
                        }
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

    ack.encode(buf)?;
    Ok(())
}
