//! The rows intrinsic reporting requires, and the ones it only permits, on
//! every object type here that reports intrinsically (#1485).
//!
//! Each Clause 12 table has two footnotes about intrinsic reporting: one
//! makes a row mandatory for an intrinsic reporter, the other keeps a row
//! off any object that isn't one. A row marked with both is required of
//! these objects, which all report intrinsically; a row marked with the
//! second alone is merely permitted. The fixtures below were
//! read off Tables 12-2, 12-3, 12-4, 12-6, 12-8, 12-10, 12-21, 12-22, 12-23,
//! 12-29, 12-30, 12-31, 12-35 and 12-37.

use super::*;

use crate::access_control::{AccessDoorObject, AccessZoneObject};
use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::event_log::EventLogObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

use bacnet_types::enums::EngineeringUnits;
use PropertyIdentifier as P;

/// The rows every one of these tables requires of an intrinsic reporter.
const COMMON_REQUIRED: [P; 6] = [
    P::NOTIFICATION_CLASS,
    P::EVENT_ENABLE,
    P::ACKED_TRANSITIONS,
    P::NOTIFY_TYPE,
    P::EVENT_TIME_STAMPS,
    P::EVENT_DETECTION_ENABLE,
];

/// The served rows every one of these tables only permits.
const COMMON_PERMITTED: [P; 4] = [
    P::EVENT_MESSAGE_TEXTS,
    P::EVENT_MESSAGE_TEXTS_CONFIG,
    P::EVENT_ALGORITHM_INHIBIT_REF,
    P::EVENT_ALGORITHM_INHIBIT,
];

/// One object type: a representative, the rows its table requires of an
/// intrinsic reporter beyond [`COMMON_REQUIRED`], and the served rows it
/// only permits beyond [`COMMON_PERMITTED`].
struct Case {
    object: Box<dyn BACnetObject>,
    required: &'static [P],
    permitted: &'static [P],
}

const OUT_OF_RANGE: &[P] = &[
    P::TIME_DELAY,
    P::HIGH_LIMIT,
    P::LOW_LIMIT,
    P::DEADBAND,
    P::LIMIT_ENABLE,
];
const BUFFER_READY: &[P] = &[
    P::NOTIFICATION_THRESHOLD,
    P::RECORDS_SINCE_NOTIFICATION,
    P::LAST_NOTIFY_RECORD,
];

fn cases() -> Vec<Case> {
    let case = |object: Box<dyn BACnetObject>, required, permitted| Case {
        object,
        required,
        permitted,
    };
    let delay_normal: &'static [P] = &[P::TIME_DELAY_NORMAL];
    vec![
        case(
            Box::new(AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
            OUT_OF_RANGE,
            delay_normal,
        ),
        case(
            Box::new(AnalogOutputObject::new(1, "AO", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
            OUT_OF_RANGE,
            delay_normal,
        ),
        case(
            Box::new(AnalogValueObject::new(1, "AV", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
            OUT_OF_RANGE,
            delay_normal,
        ),
        case(
            Box::new(BinaryInputObject::new(1, "BI").unwrap()),
            &[P::TIME_DELAY, P::ALARM_VALUE],
            delay_normal,
        ),
        // Feedback_Value carries only Table 12-8's "required" footnote.
        case(
            Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
            &[P::TIME_DELAY, P::FEEDBACK_VALUE],
            delay_normal,
        ),
        case(
            Box::new(BinaryValueObject::new(1, "BV").unwrap()),
            &[P::TIME_DELAY, P::ALARM_VALUE],
            delay_normal,
        ),
        case(
            Box::new(MultiStateInputObject::new(1, "MSI", 3).unwrap()),
            &[P::TIME_DELAY, P::ALARM_VALUES],
            delay_normal,
        ),
        case(
            Box::new(MultiStateOutputObject::new(1, "MSO", 3).unwrap()),
            &[P::TIME_DELAY, P::FEEDBACK_VALUE],
            delay_normal,
        ),
        case(
            Box::new(MultiStateValueObject::new(1, "MSV", 3).unwrap()),
            &[P::TIME_DELAY, P::ALARM_VALUES],
            delay_normal,
        ),
        // Door_Alarm_State carries Table 12-30's footnote 3 as well.
        case(
            Box::new(AccessDoorObject::new(1, "DOOR").unwrap()),
            &[P::TIME_DELAY, P::ALARM_VALUES, P::DOOR_ALARM_STATE],
            delay_normal,
        ),
        // Table 12-37's footnote 3 covers the occupancy-counting rows too.
        case(
            Box::new(AccessZoneObject::new(1, "ZONE").unwrap()),
            &[
                P::TIME_DELAY,
                P::ALARM_VALUES,
                P::OCCUPANCY_COUNT,
                P::OCCUPANCY_COUNT_ENABLE,
                P::ADJUST_VALUE,
            ],
            delay_normal,
        ),
        case(
            Box::new(TrendLogObject::new(1, "TL", 3).unwrap()),
            BUFFER_READY,
            &[],
        ),
        case(
            Box::new(EventLogObject::new(1, "EL", 3).unwrap()),
            BUFFER_READY,
            &[],
        ),
        case(
            Box::new(TrendLogMultipleObject::new(1, "TLM", 3).unwrap()),
            BUFFER_READY,
            &[],
        ),
    ]
}

#[test]
fn property_metadata_intrinsic_rows_are_required_or_only_permitted_per_table() {
    for Case {
        object,
        required,
        permitted,
    } in cases()
    {
        let kind = object.object_identifier().object_type();
        let metadata = object.property_metadata();
        let rpm_required = object.required_properties();
        let required: Vec<P> = COMMON_REQUIRED.iter().chain(required).copied().collect();
        let permitted: Vec<P> = COMMON_PERMITTED.iter().chain(permitted).copied().collect();
        for row in metadata.iter() {
            let p = row.property_identifier;
            let condition = row.presence_condition;
            if required.contains(&p) {
                assert_eq!(
                    condition,
                    Some(PropertyPresenceCondition::IntrinsicReportingRequired),
                    "{kind:?} {p:?}"
                );
                assert_eq!(
                    row.conformance,
                    PropertyConformance::Optional,
                    "{kind:?} {p:?}"
                );
                assert!(row.is_required(), "{kind:?} {p:?}");
                assert!(rpm_required.contains(&p), "{kind:?} {p:?}");
            } else if permitted.contains(&p) {
                assert_eq!(
                    condition,
                    Some(PropertyPresenceCondition::IntrinsicReportingOptional),
                    "{kind:?} {p:?}"
                );
                assert!(!row.is_required(), "{kind:?} {p:?}");
                assert!(!rpm_required.contains(&p), "{kind:?} {p:?}");
            } else {
                assert!(
                    !matches!(
                        condition,
                        Some(
                            PropertyPresenceCondition::IntrinsicReportingRequired
                                | PropertyPresenceCondition::IntrinsicReportingOptional
                        )
                    ),
                    "{kind:?} {p:?} has an intrinsic condition no fixture names"
                );
            }
        }
        for p in required.iter().chain(&permitted) {
            assert!(
                metadata.iter().any(|row| row.property_identifier == *p),
                "{kind:?} serves no {p:?}"
            );
        }
    }
}
