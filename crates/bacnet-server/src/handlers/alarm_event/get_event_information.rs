//! GetEventInformation service handler.

use super::super::*;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::traits::BACnetObject;

const EVENT_SUMMARY_SIGNATURE: [PropertyIdentifier; 6] = [
    PropertyIdentifier::EVENT_STATE,
    PropertyIdentifier::ACKED_TRANSITIONS,
    PropertyIdentifier::EVENT_TIME_STAMPS,
    PropertyIdentifier::NOTIFY_TYPE,
    PropertyIdentifier::EVENT_ENABLE,
    PropertyIdentifier::NOTIFICATION_CLASS,
];

/// A complete, strictly validated snapshot of one event-initiating object.
struct EventSummaryProjection {
    object_identifier: ObjectIdentifier,
    event_state: u32,
    acknowledged_transitions: u8,
    event_timestamps: [BACnetTimeStamp; 3],
    notify_type: u32,
    event_enable: u8,
    event_priorities: [u32; 3],
    notification_class: u32,
}

enum EventSummaryProjectionResult {
    NotEventInitiating,
    Excluded,
    Projected(EventSummaryProjection),
}

impl EventSummaryProjection {
    fn read(
        object: &dyn BACnetObject,
        db: &ObjectDatabase,
    ) -> Result<EventSummaryProjectionResult, Error> {
        let advertised = object.property_list();
        if !EVENT_SUMMARY_SIGNATURE
            .iter()
            .all(|property| advertised.contains(property))
        {
            return Ok(EventSummaryProjectionResult::NotEventInitiating);
        }

        let object_identifier = object.object_identifier();
        if advertised.contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE) {
            match read_required(
                object,
                object_identifier,
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
            )? {
                PropertyValue::Boolean(false) => return Ok(EventSummaryProjectionResult::Excluded),
                PropertyValue::Boolean(true) => {}
                _ => {
                    return Err(operational_problem(
                        object_identifier,
                        "Event_Detection_Enable is not Boolean",
                    ))
                }
            }
        }

        let event_state =
            read_enumerated(object, object_identifier, PropertyIdentifier::EVENT_STATE)?;
        let acknowledged_transitions = read_transition_bits(
            object,
            object_identifier,
            PropertyIdentifier::ACKED_TRANSITIONS,
        )?;
        let event_timestamps = read_event_timestamps(object, object_identifier)?;
        let notify_type =
            read_enumerated(object, object_identifier, PropertyIdentifier::NOTIFY_TYPE)?;
        let event_enable =
            read_transition_bits(object, object_identifier, PropertyIdentifier::EVENT_ENABLE)?;
        let notification_class = read_unsigned_u32(
            object,
            object_identifier,
            PropertyIdentifier::NOTIFICATION_CLASS,
        )?;
        let event_priorities =
            read_notification_class_priorities(db, object_identifier, notification_class)?;

        Ok(EventSummaryProjectionResult::Projected(Self {
            object_identifier,
            event_state,
            acknowledged_transitions,
            event_timestamps,
            notify_type,
            event_enable,
            event_priorities,
            notification_class,
        }))
    }

    fn is_selected(&self) -> bool {
        self.event_state != EventState::NORMAL.to_raw() || self.acknowledged_transitions != 0b111
    }
}

impl From<EventSummaryProjection> for EventSummary {
    fn from(value: EventSummaryProjection) -> Self {
        Self {
            object_identifier: value.object_identifier,
            event_state: value.event_state,
            acknowledged_transitions: value.acknowledged_transitions,
            event_timestamps: value.event_timestamps,
            notify_type: value.notify_type,
            event_enable: value.event_enable,
            event_priorities: value.event_priorities,
            notification_class: value.notification_class,
        }
    }
}

/// Handle a GetEventInformation request without service-level byte pagination.
///
/// Server dispatch uses the budget-aware variant when segmented transmission is
/// unavailable. This wrapper remains unbounded for existing direct callers.
pub fn handle_get_event_information(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    handle_get_event_information_with_budget(db, service_data, buf, None)
}

/// Build a GetEventInformation ACK within an optional service-ACK byte budget.
///
/// The budget excludes the unsegmented ComplexACK envelope. If even the first
/// complete summary does not fit, it is retained so dispatch's generic
/// segmentation/Abort path remains authoritative and cannot emit an empty page
/// with `More_Events` set.
pub(crate) fn handle_get_event_information_with_budget(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
    max_service_ack_bytes: Option<usize>,
) -> Result<(), Error> {
    let request = GetEventInformationRequest::decode(service_data)?;
    let cursor = request
        .last_received_object_identifier
        .map(|identifier| identifier.encode());
    let mut object_identifiers = db.list_objects();
    object_identifiers.sort_unstable_by_key(ObjectIdentifier::encode);

    let mut summaries = Vec::new();
    for oid in object_identifiers {
        if cursor.is_some_and(|cursor| oid.encode() <= cursor) {
            continue;
        }
        let Some(object) = db.get(&oid) else {
            continue;
        };
        let projection = match EventSummaryProjection::read(object, db)? {
            EventSummaryProjectionResult::NotEventInitiating
            | EventSummaryProjectionResult::Excluded => continue,
            EventSummaryProjectionResult::Projected(projection) => projection,
        };
        if projection.is_selected() {
            summaries.push(projection.into());
        }
    }

    let ack = paginate_ack(summaries, max_service_ack_bytes)?;
    let mut encoded = BytesMut::new();
    ack.encode(&mut encoded)?;
    buf.extend_from_slice(&encoded);
    Ok(())
}

fn paginate_ack(
    summaries: Vec<EventSummary>,
    max_service_ack_bytes: Option<usize>,
) -> Result<GetEventInformationAck, Error> {
    let Some(max_service_ack_bytes) = max_service_ack_bytes else {
        return Ok(GetEventInformationAck {
            list_of_event_summaries: summaries,
            more_events: false,
        });
    };

    let full = GetEventInformationAck {
        list_of_event_summaries: summaries.clone(),
        more_events: false,
    };
    if summaries.is_empty() || encoded_ack_len(&full)? <= max_service_ack_bytes {
        return Ok(full);
    }

    // Candidate length grows monotonically with each complete summary. Trial
    // encode whole ACK candidates (including wrappers and More_Events) while a
    // binary search locates the maximal fitting prefix without quadratic work.
    let mut low = 0usize;
    let mut high = summaries.len() - 1;
    while low < high {
        let count = low + (high - low).div_ceil(2);
        let candidate = GetEventInformationAck {
            list_of_event_summaries: summaries[..count].to_vec(),
            more_events: true,
        };
        if encoded_ack_len(&candidate)? <= max_service_ack_bytes {
            low = count;
        } else {
            high = count - 1;
        }
    }

    let count = low.max(1);
    Ok(GetEventInformationAck {
        list_of_event_summaries: summaries[..count].to_vec(),
        more_events: count < summaries.len(),
    })
}

fn encoded_ack_len(ack: &GetEventInformationAck) -> Result<usize, Error> {
    let mut encoded = BytesMut::new();
    ack.encode(&mut encoded)?;
    Ok(encoded.len())
}

fn read_required(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<PropertyValue, Error> {
    object.read_property(property, None).map_err(|error| {
        operational_problem(
            object_identifier,
            format!("required property {property:?} is unreadable: {error}"),
        )
    })
}

fn read_enumerated(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<u32, Error> {
    match read_required(object, object_identifier, property)? {
        PropertyValue::Enumerated(value) => Ok(value),
        _ => Err(operational_problem(
            object_identifier,
            format!("required property {property:?} is not Enumerated"),
        )),
    }
}

fn read_unsigned_u32(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<u32, Error> {
    match read_required(object, object_identifier, property)? {
        PropertyValue::Unsigned(value) => u32::try_from(value).map_err(|_| {
            operational_problem(
                object_identifier,
                format!("required property {property:?} exceeds Unsigned32"),
            )
        }),
        _ => Err(operational_problem(
            object_identifier,
            format!("required property {property:?} is not Unsigned"),
        )),
    }
}

fn read_transition_bits(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<u8, Error> {
    match read_required(object, object_identifier, property)? {
        PropertyValue::BitString { unused_bits, data }
            if unused_bits == 5 && data.len() == 1 && data[0] & 0x1f == 0 =>
        {
            Ok(bacnet_types::bitstring::unpack_octet(&data, 3))
        }
        _ => Err(operational_problem(
            object_identifier,
            format!("required property {property:?} is not a canonical three-bit BitString"),
        )),
    }
}

fn read_event_timestamps(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
) -> Result<[BACnetTimeStamp; 3], Error> {
    let PropertyValue::List(items) = read_required(
        object,
        object_identifier,
        PropertyIdentifier::EVENT_TIME_STAMPS,
    )?
    else {
        return Err(operational_problem(
            object_identifier,
            "Event_Time_Stamps is not a three-item List",
        ));
    };
    let [offnormal, fault, normal] = items.as_slice() else {
        return Err(operational_problem(
            object_identifier,
            "Event_Time_Stamps does not contain exactly three items",
        ));
    };

    Ok([
        decode_event_timestamp(offnormal).ok_or_else(|| {
            operational_problem(object_identifier, "invalid TO_OFFNORMAL event timestamp")
        })?,
        decode_event_timestamp(fault).ok_or_else(|| {
            operational_problem(object_identifier, "invalid TO_FAULT event timestamp")
        })?,
        decode_event_timestamp(normal).ok_or_else(|| {
            operational_problem(object_identifier, "invalid TO_NORMAL event timestamp")
        })?,
    ])
}

fn read_notification_class_priorities(
    db: &ObjectDatabase,
    event_object_identifier: ObjectIdentifier,
    notification_class: u32,
) -> Result<[u32; 3], Error> {
    let mut matching = None;
    for class_oid in db.find_by_type(ObjectType::NOTIFICATION_CLASS) {
        let Some(class_object) = db.get(&class_oid) else {
            continue;
        };
        let Some(class_number) = read_notification_class_number(class_object) else {
            continue;
        };
        if class_number == notification_class {
            if matching.replace(class_object).is_some() {
                return Err(operational_problem(
                    event_object_identifier,
                    format!("multiple Notification Class objects match {notification_class}"),
                ));
            }
        }
    }
    let class_object = matching.ok_or_else(|| {
        operational_problem(
            event_object_identifier,
            format!("no Notification Class object matches {notification_class}"),
        )
    })?;

    let PropertyValue::List(priorities) = read_required(
        class_object,
        event_object_identifier,
        PropertyIdentifier::PRIORITY,
    )?
    else {
        return Err(operational_problem(
            event_object_identifier,
            "Notification Class Priority is not a three-item List",
        ));
    };
    let [offnormal, fault, normal] = priorities.as_slice() else {
        return Err(operational_problem(
            event_object_identifier,
            "Notification Class Priority does not contain exactly three items",
        ));
    };

    Ok([
        read_priority_coordinate(event_object_identifier, offnormal, "TO_OFFNORMAL")?,
        read_priority_coordinate(event_object_identifier, fault, "TO_FAULT")?,
        read_priority_coordinate(event_object_identifier, normal, "TO_NORMAL")?,
    ])
}

fn read_notification_class_number(object: &dyn BACnetObject) -> Option<u32> {
    match object
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .ok()?
    {
        PropertyValue::Unsigned(value) => u32::try_from(value).ok(),
        _ => None,
    }
}

fn read_priority_coordinate(
    object_identifier: ObjectIdentifier,
    value: &PropertyValue,
    coordinate: &str,
) -> Result<u32, Error> {
    match value {
        PropertyValue::Unsigned(priority) if *priority <= 255 => Ok(*priority as u32),
        PropertyValue::Unsigned(_) => Err(operational_problem(
            object_identifier,
            format!("Notification Class {coordinate} Priority exceeds 255"),
        )),
        _ => Err(operational_problem(
            object_identifier,
            format!("Notification Class {coordinate} Priority is not Unsigned"),
        )),
    }
}

fn operational_problem(
    object_identifier: ObjectIdentifier,
    detail: impl std::fmt::Display,
) -> Error {
    tracing::warn!(?object_identifier, reason = %detail, "GetEventInformation projection failed");
    Error::Protocol {
        class: ErrorClass::DEVICE.to_raw() as u32,
        code: ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32,
    }
}

fn decode_event_timestamp(value: &PropertyValue) -> Option<BACnetTimeStamp> {
    match value {
        PropertyValue::ApplicationData(encoded) => {
            let (timestamp, consumed) = decode_timestamp_choice(encoded, 0).ok()?;
            (consumed == encoded.len()).then_some(timestamp)
        }
        PropertyValue::Unsigned(sequence_number) => u16::try_from(*sequence_number)
            .ok()
            .map(BACnetTimeStamp::SequenceNumber),
        _ => None,
    }
}
