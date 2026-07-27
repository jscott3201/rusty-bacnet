//! GetAlarmSummary service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;

/// Handle a GetAlarmSummary request.
///
/// Returns objects with event_state != NORMAL.
pub fn handle_get_alarm_summary(db: &ObjectDatabase, buf: &mut BytesMut) -> Result<(), Error> {
    use bacnet_services::alarm_summary::{AlarmSummaryEntry, GetAlarmSummaryAck};

    let mut entries = Vec::new();
    for (_oid, object) in db.iter_objects() {
        // Clause 12.12: Event_Detection_Enable controls whether the object is
        // considered by the event-summarization services.
        if !super::event_detection_enabled(object) {
            continue;
        }

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
