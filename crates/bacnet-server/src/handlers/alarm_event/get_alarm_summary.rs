//! GetAlarmSummary service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::NotifyType;

const ALARM_SUMMARY_SIGNATURE: [PropertyIdentifier; 3] = [
    PropertyIdentifier::EVENT_STATE,
    PropertyIdentifier::NOTIFY_TYPE,
    PropertyIdentifier::ACKED_TRANSITIONS,
];

struct AlarmSummaryProjection {
    object_identifier: ObjectIdentifier,
    event_state: u32,
    acknowledged_transitions: (u8, Vec<u8>),
}

enum AlarmSummaryProjectionResult {
    NotEventInitiating,
    Excluded,
    Projected(AlarmSummaryProjection),
}

impl AlarmSummaryProjection {
    fn read(object: &dyn BACnetObject) -> Result<AlarmSummaryProjectionResult, Error> {
        let advertised = object.property_list();
        if !ALARM_SUMMARY_SIGNATURE
            .iter()
            .all(|property| advertised.contains(property))
        {
            return Ok(AlarmSummaryProjectionResult::NotEventInitiating);
        }

        let object_identifier = object.object_identifier();
        if advertised.contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE) {
            match read_required(
                object,
                object_identifier,
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
            )? {
                PropertyValue::Boolean(false) => return Ok(AlarmSummaryProjectionResult::Excluded),
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
        if event_state == EventState::NORMAL.to_raw() {
            return Ok(AlarmSummaryProjectionResult::Excluded);
        }

        let notify_type =
            read_enumerated(object, object_identifier, PropertyIdentifier::NOTIFY_TYPE)?;
        if notify_type != NotifyType::ALARM.to_raw() {
            return Ok(AlarmSummaryProjectionResult::Excluded);
        }

        let acknowledged_transitions = read_acknowledged_transitions(object, object_identifier)?;
        Ok(AlarmSummaryProjectionResult::Projected(Self {
            object_identifier,
            event_state,
            acknowledged_transitions,
        }))
    }
}

/// Handle a GetAlarmSummary request.
///
/// Clause 13.10 selects event-initiating objects whose Event_State is not
/// NORMAL and whose Notify_Type is ALARM. As a local strict-projection policy,
/// malformed advertised candidate fields fail the service instead of being
/// replaced with fabricated values.
pub fn handle_get_alarm_summary(db: &ObjectDatabase, buf: &mut BytesMut) -> Result<(), Error> {
    use bacnet_services::alarm_summary::{AlarmSummaryEntry, GetAlarmSummaryAck};

    let mut entries = Vec::new();
    for (_oid, object) in db.iter_objects() {
        let projection = match AlarmSummaryProjection::read(object)? {
            AlarmSummaryProjectionResult::NotEventInitiating
            | AlarmSummaryProjectionResult::Excluded => continue,
            AlarmSummaryProjectionResult::Projected(projection) => projection,
        };
        entries.push(AlarmSummaryEntry {
            object_identifier: projection.object_identifier,
            alarm_state: EventState::from_raw(projection.event_state),
            acknowledged_transitions: projection.acknowledged_transitions,
        });
    }

    let ack = GetAlarmSummaryAck { entries };
    ack.encode(buf);
    Ok(())
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

fn read_acknowledged_transitions(
    object: &dyn BACnetObject,
    object_identifier: ObjectIdentifier,
) -> Result<(u8, Vec<u8>), Error> {
    match read_required(
        object,
        object_identifier,
        PropertyIdentifier::ACKED_TRANSITIONS,
    )? {
        PropertyValue::BitString { unused_bits, data }
            if unused_bits == 5 && data.len() == 1 && data[0] & 0x1f == 0 =>
        {
            Ok((unused_bits, data))
        }
        _ => Err(operational_problem(
            object_identifier,
            "Acked_Transitions is not a canonical three-bit BitString",
        )),
    }
}

fn operational_problem(
    object_identifier: ObjectIdentifier,
    detail: impl std::fmt::Display,
) -> Error {
    tracing::warn!(?object_identifier, reason = %detail, "GetAlarmSummary projection failed");
    Error::Protocol {
        class: ErrorClass::DEVICE.to_raw() as u32,
        code: ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32,
    }
}
