//! The Event and Alert Enrollments' writability before #2177 part 2, when
//! each overrode `is_writable_property` with its own match, frozen here
//! against the rows that now answer it alone: equal for every property.

use super::super::*;
use crate::property_metadata::is_writable_property;
use PropertyIdentifier as P;

fn old_event_enrollment(property: P) -> bool {
    matches!(
        property,
        P::OBJECT_NAME
            | P::DESCRIPTION
            | P::NOTIFY_TYPE
            | P::NOTIFICATION_CLASS
            | P::EVENT_ENABLE
            | P::EVENT_DETECTION_ENABLE
            | P::EVENT_PARAMETERS
            | P::FAULT_PARAMETERS
            | P::TIME_DELAY_NORMAL
            | P::EVENT_MESSAGE_TEXTS_CONFIG
            | P::EVENT_ALGORITHM_INHIBIT_REF
            | P::EVENT_ALGORITHM_INHIBIT
    )
}

fn old_alert_enrollment(property: P) -> bool {
    matches!(
        property,
        P::DESCRIPTION
            | P::EVENT_DETECTION_ENABLE
            | P::EVENT_ENABLE
            | P::NOTIFICATION_CLASS
            | P::NOTIFY_TYPE
            | P::EVENT_MESSAGE_TEXTS_CONFIG
            | P::EVENT_ALGORITHM_INHIBIT_REF
            | P::EVENT_ALGORITHM_INHIBIT
    )
}

#[test]
fn enrollment_rows_answer_writability_as_the_old_overrides_did() {
    let event = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    let source = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let alert = AlertEnrollmentObject::new(1, "AE-1", source).unwrap();
    let properties = (0..=1023).map(P::from_raw).chain(
        P::ALL_NAMED
            .iter()
            .map(|&(_, property)| property)
            .filter(|property| property.to_raw() > 1023),
    );
    for property in properties {
        assert_eq!(
            is_writable_property(&event, property),
            old_event_enrollment(property),
            "Event Enrollment {property:?}"
        );
        assert_eq!(
            is_writable_property(&alert, property),
            old_alert_enrollment(property),
            "Alert Enrollment {property:?}"
        );
    }
}
