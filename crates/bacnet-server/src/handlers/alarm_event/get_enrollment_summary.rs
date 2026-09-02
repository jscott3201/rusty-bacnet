//! GetEnrollmentSummary service handler.

use super::super::*;
use bacnet_objects::event::{EnrollmentSummaryCapability, EventTransition};
use bacnet_objects::notification_class::resolve_enrollment_summary_class_internal;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::enrollment_summary::{
    EnrollmentSummaryEntry, GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest,
};
use bacnet_types::enums::EnrollmentSummaryEventStateFilter;

/// Handle the deprecated GetEnrollmentSummary interoperability service.
///
/// Candidates opt in through object-owned event capability. Every mandatory
/// projected value is then read and validated strictly; a malformed candidate
/// fails DEVICE / OPERATIONAL_PROBLEM rather than fabricating a summary.
pub fn handle_get_enrollment_summary(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = GetEnrollmentSummaryRequest::decode(service_data)?;

    let mut entries = Vec::new();
    for (_oid, object) in db.iter_objects() {
        let Some(capability) = object.enrollment_summary_capability_internal() else {
            continue;
        };
        let object_identifier = object.object_identifier();
        let advertised = object.property_list();

        if advertised.contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE) {
            match read_required(
                object,
                object_identifier,
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
            )? {
                PropertyValue::Boolean(false) => continue,
                PropertyValue::Boolean(true) => {}
                _ => {
                    return Err(operational_problem(
                        object_identifier,
                        "Event_Detection_Enable is not Boolean",
                    ))
                }
            }
        }

        let event_state = read_event_state(object, object_identifier)?;
        let acknowledged_transitions = read_acknowledged_transitions(object, object_identifier)?;
        let transition = latest_transition(
            capability,
            event_state,
            acknowledged_transitions,
            object_identifier,
        )?;
        let notification_class = read_notification_class(object, object_identifier)?;
        let class_projection = resolve_enrollment_summary_class_internal(
            db,
            notification_class,
            transition,
            request
                .enrollment_filter
                .as_ref()
                .map(|filter| (&filter.recipient, filter.process_identifier)),
        )
        .map_err(|error| {
            operational_problem(
                object_identifier,
                format!("Notification Class {notification_class} projection failed: {error}"),
            )
        })?;

        if !acknowledgment_matches(request.acknowledgment_filter, acknowledged_transitions)
            || !class_projection.enrollment_member
            || !event_state_matches(request.event_state_filter, event_state)
            || request
                .event_type_filter
                .is_some_and(|filter| filter != capability.event_type)
            || request.priority_filter.is_some_and(|filter| {
                class_projection.priority < filter.min_priority
                    || class_projection.priority > filter.max_priority
            })
            || request
                .notification_class_filter
                .is_some_and(|filter| filter != notification_class)
        {
            continue;
        }

        entries.push(EnrollmentSummaryEntry {
            object_identifier,
            event_type: capability.event_type,
            event_state,
            priority: class_projection.priority,
            notification_class: Some(notification_class),
        });
    }

    GetEnrollmentSummaryAck { entries }.encode(buf);
    Ok(())
}

fn latest_transition(
    capability: EnrollmentSummaryCapability,
    event_state: EventState,
    acknowledged_transitions: u8,
    object_identifier: ObjectIdentifier,
) -> Result<EventTransition, Error> {
    if let Some(transition) = capability.last_transition {
        return Ok(transition);
    }
    if event_state == EventState::NORMAL && acknowledged_transitions == 0b111 {
        return Ok(EventTransition::ToNormal);
    }
    Err(operational_problem(
        object_identifier,
        "no committed transition and initial event invariants are inconsistent",
    ))
}

fn acknowledgment_matches(filter: u32, acknowledged_transitions: u8) -> bool {
    match filter {
        0 => true,
        1 => acknowledged_transitions == 0b111,
        2 => acknowledged_transitions != 0b111,
        _ => unreachable!("request decoder rejects undefined acknowledgment filters"),
    }
}

fn event_state_matches(
    filter: Option<EnrollmentSummaryEventStateFilter>,
    event_state: EventState,
) -> bool {
    match filter {
        None => true,
        Some(filter) if filter == EnrollmentSummaryEventStateFilter::ALL => true,
        Some(filter) if filter == EnrollmentSummaryEventStateFilter::ACTIVE => {
            event_state != EventState::NORMAL
        }
        Some(filter) if filter == EnrollmentSummaryEventStateFilter::OFFNORMAL => {
            event_state == EventState::OFFNORMAL
        }
        Some(filter) if filter == EnrollmentSummaryEventStateFilter::FAULT => {
            event_state == EventState::FAULT
        }
        Some(filter) if filter == EnrollmentSummaryEventStateFilter::NORMAL => {
            event_state == EventState::NORMAL
        }
        Some(_) => false,
    }
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

fn read_event_state(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
) -> Result<EventState, Error> {
    match read_required(object, object_identifier, PropertyIdentifier::EVENT_STATE)? {
        PropertyValue::Enumerated(value) => Ok(EventState::from_raw(value)),
        _ => Err(operational_problem(
            object_identifier,
            "Event_State is not Enumerated",
        )),
    }
}

fn read_acknowledged_transitions(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
) -> Result<u8, Error> {
    match read_required(
        object,
        object_identifier,
        PropertyIdentifier::ACKED_TRANSITIONS,
    )? {
        PropertyValue::BitString { unused_bits, data }
            if unused_bits == 5 && data.len() == 1 && data[0] & 0x1f == 0 =>
        {
            Ok(bacnet_types::bitstring::unpack_octet(&data, 3))
        }
        _ => Err(operational_problem(
            object_identifier,
            "Acked_Transitions is not a canonical three-bit BitString",
        )),
    }
}

fn read_notification_class(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
) -> Result<u32, Error> {
    match read_required(
        object,
        object_identifier,
        PropertyIdentifier::NOTIFICATION_CLASS,
    )? {
        PropertyValue::Unsigned(value) => u32::try_from(value).map_err(|_| {
            operational_problem(object_identifier, "Notification_Class exceeds Unsigned32")
        }),
        _ => Err(operational_problem(
            object_identifier,
            "Notification_Class is not Unsigned32",
        )),
    }
}

fn operational_problem(
    object_identifier: ObjectIdentifier,
    detail: impl std::fmt::Display,
) -> Error {
    tracing::warn!(?object_identifier, reason = %detail, "GetEnrollmentSummary projection failed");
    Error::Protocol {
        class: ErrorClass::DEVICE.to_raw() as u32,
        code: ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32,
    }
}
