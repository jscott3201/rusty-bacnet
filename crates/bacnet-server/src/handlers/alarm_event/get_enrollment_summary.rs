//! GetEnrollmentSummary service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;

/// Handle a GetEnrollmentSummary request.
///
/// Returns event-enrollment objects that match the filter criteria.
pub fn handle_get_enrollment_summary(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::enrollment_summary::{
        EnrollmentSummaryEntry, GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest,
    };

    let request = GetEnrollmentSummaryRequest::decode(service_data)?;

    let mut entries = Vec::new();
    for (_oid, object) in db.iter_objects() {
        // Clause 12.12: Event_Detection_Enable controls whether the object is
        // considered by the event-summarization services. This service does not
        // filter on Event_State by default, so unlike GetAlarmSummary and
        // GetEventInformation the exclusion cannot fall out of the
        // forced-NORMAL invariant — it has to be checked explicitly.
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

        if let Some(filter_state) = request.event_state_filter {
            if event_state != filter_state.to_raw() {
                continue;
            }
        }

        let notification_class = object
            .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
            .ok()
            .and_then(|v| match v {
                PropertyValue::Unsigned(n) => Some(n as u16),
                _ => None,
            })
            .unwrap_or(0);

        if let Some(nc_filter) = request.notification_class_filter {
            if notification_class != nc_filter {
                continue;
            }
        }

        if let Some(ref pf) = request.priority_filter {
            // Look up priority from the notification class object for the current event state
            let priority = ObjectIdentifier::new(
                bacnet_types::enums::ObjectType::NOTIFICATION_CLASS,
                notification_class as u32,
            )
            .ok()
            .and_then(|nc_oid| db.get(&nc_oid))
            .and_then(|nc_obj| {
                // PRIORITY property returns an array of 3 priorities (TO_OFFNORMAL, TO_FAULT, TO_NORMAL)
                nc_obj
                    .read_property(PropertyIdentifier::PRIORITY, Some(1))
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::Unsigned(p) => Some(p as u8),
                        _ => None,
                    })
            })
            .unwrap_or(0);
            if priority < pf.min_priority || priority > pf.max_priority {
                continue;
            }
        }

        if event_state == 0 && request.event_state_filter.is_none() {
            continue;
        }

        entries.push(EnrollmentSummaryEntry {
            object_identifier: oid,
            event_type: bacnet_types::enums::EventType::CHANGE_OF_STATE,
            event_state: bacnet_types::enums::EventState::from_raw(event_state),
            priority: 0,
            notification_class,
        });
    }

    let ack = GetEnrollmentSummaryAck { entries };
    ack.encode(buf);
    Ok(())
}
