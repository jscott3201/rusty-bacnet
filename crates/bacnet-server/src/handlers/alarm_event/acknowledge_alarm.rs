//! AcknowledgeAlarm service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;
use bacnet_objects::event::EventStateChange;

/// Exact context the bundled server can use for ACK_NOTIFICATION distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcknowledgmentNotificationContext {
    /// Exact historical transition correlated by the request.
    pub change: EventStateChange,
    /// Effective Event Type for the acknowledged transition.
    pub event_type: EventType,
    /// Current Event_Enable decision for the transition coordinate.
    pub distribute: bool,
}

/// A successfully correlated and applied acknowledgment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedAcknowledgeAlarm {
    /// Object whose acknowledgment bit was accepted.
    pub event_object_identifier: ObjectIdentifier,
    /// Exact notification context, when the object can provide it.
    ///
    /// Custom objects implementing only the legacy coarse acknowledgment hook
    /// return `None`; their accepted acknowledgment remains successful.
    pub notification: Option<AcknowledgmentNotificationContext>,
}

/// Handle an AcknowledgeAlarm request.
///
/// Correlates the request with one committed transition before acknowledging it.
pub fn handle_acknowledge_alarm(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<AcceptedAcknowledgeAlarm, Error> {
    let request = AcknowledgeAlarmRequest::decode(service_data)?;
    let event_state = EventState::from_raw(request.event_state_acknowledged);

    let object = db
        .get_mut(&request.event_object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    let change =
        object.acknowledge_alarm_correlated_detailed_internal(event_state, &request.timestamp)?;

    let notification = change.and_then(|change| {
        let algorithm = object.enrollment_summary_capability_internal()?.event_type;
        let PropertyValue::BitString { unused_bits, data } = object
            .read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .ok()?
        else {
            return None;
        };
        if unused_bits != 5 || data.len() != 1 || data[0] & 0x1f != 0 {
            return None;
        }
        let enabled = bacnet_types::bitstring::unpack_octet(&data, 3);
        let distribute = enabled & change.transition().bit_mask() != 0;
        let event_type = change.event_type(algorithm);
        Some(AcknowledgmentNotificationContext {
            change,
            event_type,
            distribute,
        })
    });

    // These request fields are validated by decoding but intentionally are not
    // persisted. ACK_NOTIFICATION construction uses object history and a fresh
    // send-time timestamp rather than either caller timestamp.
    let _ = (
        request.acknowledging_process_identifier,
        request.acknowledgment_source,
        request.time_of_acknowledgment,
    );

    Ok(AcceptedAcknowledgeAlarm {
        event_object_identifier: request.event_object_identifier,
        notification,
    })
}
