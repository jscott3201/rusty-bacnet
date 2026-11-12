//! GetEventInformation service handler.

use super::super::*;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::event::EventTransition;
use bacnet_objects::property_metadata::has_property;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::NotifyType;

#[path = "event_information_page.rs"]
mod page;
pub(crate) use page::{EventInformationFailure, handle_get_event_information_configured};

const EVENT_SUMMARY_SIGNATURE: [PropertyIdentifier; 6] = [
    PropertyIdentifier::EVENT_STATE,
    PropertyIdentifier::ACKED_TRANSITIONS,
    PropertyIdentifier::EVENT_TIME_STAMPS,
    PropertyIdentifier::NOTIFY_TYPE,
    PropertyIdentifier::EVENT_ENABLE,
    PropertyIdentifier::NOTIFICATION_CLASS,
];

/// Why one event-initiating object's summary could not be projected. The
/// request it was projected for answers OPERATIONAL_PROBLEM.
#[derive(Debug)]
pub(crate) struct EventSummaryProblem {
    /// The object whose summary failed.
    pub(crate) object_identifier: ObjectIdentifier,
    /// What was wrong with it.
    pub(crate) fault: EventSummaryFault,
}

/// What was wrong with an event-initiating object's summary.
#[derive(Debug)]
pub(crate) enum EventSummaryFault {
    /// A required property didn't read.
    Unreadable {
        /// The property.
        property: PropertyIdentifier,
        /// Its read error.
        error: Error,
    },
    /// A required property isn't Enumerated.
    NotEnumerated(PropertyIdentifier),
    /// A required Unsigned property is past Unsigned32.
    ExceedsUnsigned32(PropertyIdentifier),
    /// A required property isn't Unsigned.
    NotUnsigned(PropertyIdentifier),
    /// A required property isn't a canonical three-bit BitString.
    NotTransitionBits(PropertyIdentifier),
    /// Event_Detection_Enable isn't Boolean.
    DetectionEnableNotBoolean,
    /// Event_Time_Stamps isn't a List.
    TimestampsNotList,
    /// Event_Time_Stamps doesn't hold exactly three items.
    TimestampsNotThree,
    /// The Event_Time_Stamps entry for this transition isn't a timestamp.
    InvalidTimestamp(EventTransition),
    /// More than one Notification Class object has this class number.
    DuplicateClass(u32),
    /// No Notification Class object has this class number.
    MissingClass(u32),
    /// The class's Priority isn't a List.
    PriorityNotList,
    /// The class's Priority doesn't hold exactly three items.
    PriorityNotThree,
    /// The class's priority for this transition is past 255.
    PriorityOver255(EventTransition),
    /// The class's priority for this transition isn't Unsigned.
    PriorityNotUnsigned(EventTransition),
}

impl EventSummaryProblem {
    /// The error the request answers with: DEVICE, OPERATIONAL_PROBLEM.
    pub(crate) fn to_error(&self) -> Error {
        Error::protocol(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
    }
}

impl From<EventSummaryProblem> for Error {
    fn from(problem: EventSummaryProblem) -> Self {
        problem.to_error()
    }
}

enum EventSummaryProjectionResult {
    NotEventInitiating,
    Excluded,
    Projected(EventSummary),
}

/// Read a complete, strictly validated event summary for one event-initiating object.
///
/// `Notification_Class` is not part of the summary: it only selects the
/// Notification Class object whose `Priority` fills `event_priorities`.
fn project_event_summary(
    object: &dyn BACnetObject,
    db: &ObjectDatabase,
) -> Result<EventSummaryProjectionResult, EventSummaryProblem> {
    let advertised = object.property_metadata();
    if !EVENT_SUMMARY_SIGNATURE
        .iter()
        .all(|&property| has_property(&advertised, property))
    {
        return Ok(EventSummaryProjectionResult::NotEventInitiating);
    }

    let object_identifier = object.object_identifier();
    if has_property(&advertised, PropertyIdentifier::EVENT_DETECTION_ENABLE) {
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
                    EventSummaryFault::DetectionEnableNotBoolean,
                ));
            }
        }
    }

    let event_state = EventState::from_raw(read_enumerated(
        object,
        object_identifier,
        PropertyIdentifier::EVENT_STATE,
    )?);
    let acknowledged_transitions = read_transition_bits(
        object,
        object_identifier,
        PropertyIdentifier::ACKED_TRANSITIONS,
    )?;
    let event_timestamps = read_event_timestamps(object, object_identifier)?;
    let notify_type = NotifyType::from_raw(read_enumerated(
        object,
        object_identifier,
        PropertyIdentifier::NOTIFY_TYPE,
    )?);
    let event_enable =
        read_transition_bits(object, object_identifier, PropertyIdentifier::EVENT_ENABLE)?;
    let notification_class = read_unsigned_u32(
        object,
        object_identifier,
        PropertyIdentifier::NOTIFICATION_CLASS,
    )?;
    let event_priorities =
        read_notification_class_priorities(db, object_identifier, notification_class)?;

    Ok(EventSummaryProjectionResult::Projected(EventSummary {
        object_identifier,
        event_state,
        acknowledged_transitions,
        event_timestamps,
        notify_type,
        event_enable,
        event_priorities,
    }))
}

/// Whether the summary describes an active event state (Clause 13.12): an event
/// state other than NORMAL, or at least one unacknowledged transition.
fn is_active_event_state(summary: &EventSummary) -> bool {
    summary.event_state != EventState::NORMAL
        || summary.acknowledged_transitions != EventTransitionBits::all()
}

/// Handle a GetEventInformation request without service-level byte pagination.
///
/// Server dispatch uses a separate configured admission and strict page policy.
/// This wrapper remains unbounded for existing direct callers.
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
    let request =
        GetEventInformationRequest::decode(service_data).map_err(Error::into_request_reject)?;
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
        let summary = match project_event_summary(object, db)? {
            EventSummaryProjectionResult::NotEventInitiating
            | EventSummaryProjectionResult::Excluded => continue,
            EventSummaryProjectionResult::Projected(summary) => summary,
        };
        if is_active_event_state(&summary) {
            summaries.push(summary);
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
) -> Result<PropertyValue, EventSummaryProblem> {
    object.read_property(property, None).map_err(|error| {
        operational_problem(
            object_identifier,
            EventSummaryFault::Unreadable { property, error },
        )
    })
}

fn read_enumerated(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<u32, EventSummaryProblem> {
    match read_required(object, object_identifier, property)? {
        PropertyValue::Enumerated(value) => Ok(value),
        _ => Err(operational_problem(
            object_identifier,
            EventSummaryFault::NotEnumerated(property),
        )),
    }
}

fn read_unsigned_u32(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<u32, EventSummaryProblem> {
    match read_required(object, object_identifier, property)? {
        PropertyValue::Unsigned(value) => u32::try_from(value).map_err(|_| {
            operational_problem(
                object_identifier,
                EventSummaryFault::ExceedsUnsigned32(property),
            )
        }),
        _ => Err(operational_problem(
            object_identifier,
            EventSummaryFault::NotUnsigned(property),
        )),
    }
}

fn read_transition_bits(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<EventTransitionBits, EventSummaryProblem> {
    match read_required(object, object_identifier, property)? {
        PropertyValue::BitString { unused_bits, data }
            if unused_bits == 5 && data.len() == 1 && data[0] & 0x1f == 0 =>
        {
            Ok(EventTransitionBits::from_bacnet(&data))
        }
        _ => Err(operational_problem(
            object_identifier,
            EventSummaryFault::NotTransitionBits(property),
        )),
    }
}

fn read_event_timestamps(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
) -> Result<[BACnetTimeStamp; 3], EventSummaryProblem> {
    let PropertyValue::List(items) = read_required(
        object,
        object_identifier,
        PropertyIdentifier::EVENT_TIME_STAMPS,
    )?
    else {
        return Err(operational_problem(
            object_identifier,
            EventSummaryFault::TimestampsNotList,
        ));
    };
    let [offnormal, fault, normal] = items.as_slice() else {
        return Err(operational_problem(
            object_identifier,
            EventSummaryFault::TimestampsNotThree,
        ));
    };

    let decode = |value, transition| {
        decode_event_timestamp(value).ok_or_else(|| {
            operational_problem(
                object_identifier,
                EventSummaryFault::InvalidTimestamp(transition),
            )
        })
    };
    Ok([
        decode(offnormal, EventTransition::ToOffnormal)?,
        decode(fault, EventTransition::ToFault)?,
        decode(normal, EventTransition::ToNormal)?,
    ])
}

fn read_notification_class_priorities(
    db: &ObjectDatabase,
    event_object_identifier: ObjectIdentifier,
    notification_class: u32,
) -> Result<[u32; 3], EventSummaryProblem> {
    let mut matching = None;
    for class_oid in db.find_by_type(ObjectType::NOTIFICATION_CLASS) {
        let Some(class_object) = db.get(&class_oid) else {
            continue;
        };
        let Some(class_number) = read_notification_class_number(class_object) else {
            continue;
        };
        if class_number == notification_class && matching.replace(class_object).is_some() {
            return Err(operational_problem(
                event_object_identifier,
                EventSummaryFault::DuplicateClass(notification_class),
            ));
        }
    }
    let class_object = matching.ok_or_else(|| {
        operational_problem(
            event_object_identifier,
            EventSummaryFault::MissingClass(notification_class),
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
            EventSummaryFault::PriorityNotList,
        ));
    };
    let [offnormal, fault, normal] = priorities.as_slice() else {
        return Err(operational_problem(
            event_object_identifier,
            EventSummaryFault::PriorityNotThree,
        ));
    };

    Ok([
        read_priority_coordinate(
            event_object_identifier,
            offnormal,
            EventTransition::ToOffnormal,
        )?,
        read_priority_coordinate(event_object_identifier, fault, EventTransition::ToFault)?,
        read_priority_coordinate(event_object_identifier, normal, EventTransition::ToNormal)?,
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
    transition: EventTransition,
) -> Result<u32, EventSummaryProblem> {
    match value {
        PropertyValue::Unsigned(priority) if *priority <= 255 => Ok(*priority as u32),
        PropertyValue::Unsigned(_) => Err(operational_problem(
            object_identifier,
            EventSummaryFault::PriorityOver255(transition),
        )),
        _ => Err(operational_problem(
            object_identifier,
            EventSummaryFault::PriorityNotUnsigned(transition),
        )),
    }
}

fn operational_problem(
    object_identifier: ObjectIdentifier,
    fault: EventSummaryFault,
) -> EventSummaryProblem {
    EventSummaryProblem {
        object_identifier,
        fault,
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
