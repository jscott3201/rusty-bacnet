//! Alarm and event service handlers (ASHRAE 135-2020 Clause 13).

use super::*;

/// Whether an object may be reported by the event-summarization services.
///
/// ASHRAE 135-2020 Clause 12.12 defines `Event_Detection_Enable` as controlling
/// "whether (TRUE) or not (FALSE) the object will be considered by event
/// summarization services", and the Service Procedure of each of Clauses 13.10,
/// 13.11 and 13.12 repeats the exclusion independently, so GetAlarmSummary,
/// GetEnrollmentSummary and GetEventInformation must all skip an object that
/// reads FALSE.
///
/// Only an explicit `Boolean(false)` disables. The property is required (R) on
/// Event Enrollment (Table 12-14) and Alert Enrollment (Table 12-61), and
/// optional or absent elsewhere; an object that does not model it returns
/// `UNKNOWN_PROPERTY` here — which must read as *enabled*, not as disabled, or
/// every object lacking the property would vanish from these services.
///
/// This is checked directly rather than inferred from `Event_State == NORMAL`:
/// GetEnrollmentSummary has no default Event_State filter, and the standard
/// defines the exclusion independently for each service.
pub(crate) fn event_detection_enabled(object: &dyn bacnet_objects::traits::BACnetObject) -> bool {
    !matches!(
        object.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None),
        Ok(PropertyValue::Boolean(false))
    )
}

mod acknowledge_alarm;
mod get_alarm_summary;
mod get_enrollment_summary;
mod get_event_information;
mod life_safety_operation;
mod text_message;

pub use acknowledge_alarm::*;
pub use get_alarm_summary::*;
pub use get_enrollment_summary::*;
pub use get_event_information::*;
pub use life_safety_operation::*;
pub use text_message::*;
