//! AcknowledgeAlarm service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;

/// Handle an AcknowledgeAlarm request.
///
/// Correlates the request with one committed transition before acknowledging it.
pub fn handle_acknowledge_alarm(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    let request = AcknowledgeAlarmRequest::decode(service_data)?;
    let event_state = EventState::from_raw(request.event_state_acknowledged);

    let object = db
        .get_mut(&request.event_object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    object.acknowledge_alarm_correlated_internal(event_state, &request.timestamp)?;

    // ACK_NOTIFICATION distribution and acknowledgment metadata retention are
    // deferred to #175. This core slice validates every required request field
    // but intentionally retains none of this metadata after successful use.
    let _ = (
        request.acknowledging_process_identifier,
        request.acknowledgment_source,
        request.time_of_acknowledgment,
    );

    Ok(())
}
