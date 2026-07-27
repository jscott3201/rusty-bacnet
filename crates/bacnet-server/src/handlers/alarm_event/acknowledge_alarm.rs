//! AcknowledgeAlarm service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;

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
