//! Alarm and event service handlers (ASHRAE 135-2020 Clause 13).

use super::*;

/// Whether an object may be reported by the event-summarization services.
///
/// ASHRAE 135-2020 Clause 12.12 defines `Event_Detection_Enable` as controlling
/// "whether (TRUE) or not (FALSE) the object will be considered by event
/// summarization services", so GetAlarmSummary, GetEnrollmentSummary and
/// GetEventInformation must all skip an object that reads FALSE.
///
/// Only an explicit `Boolean(false)` disables. The property is required (R) on
/// Event Enrollment but optional or absent elsewhere, and an object that does
/// not model it returns `UNKNOWN_PROPERTY` here — which must read as *enabled*,
/// not as disabled, or every object lacking the property would vanish from
/// these services.
///
/// This is checked directly rather than inferred from `Event_State == NORMAL`.
/// The two are equivalent only for objects that actually apply the
/// Clause 13.2.2.1 reset, and `AlertEnrollmentObject` currently does not
/// (tracked as #205), so inferring it would leak a disabled object into these
/// responses.
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
