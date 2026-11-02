//! Rows that come as a group, checked over the whole catalogue (#2175).
//!
//! - A commanded object's Priority_Array, Relinquish_Default and
//!   Current_Command_Priority come together: Clause 19.2 pairs the first
//!   two for every commandable property, and the object tables require the
//!   third of a commandable object.
//! - An object that reports events serves the whole reporting group its
//!   table requires of an intrinsic reporter, or unconditionally for the
//!   Event and Alert Enrollments: Event_State, Event_Detection_Enable,
//!   Notification_Class, Event_Enable, Acked_Transitions, Notify_Type and
//!   Event_Time_Stamps. An object that doesn't report has none of them but
//!   Event_State, which many tables require on their own.
//! - Event_Algorithm_Inhibit_Ref comes with Event_Algorithm_Inhibit.
//!
//! `intrinsic` pins which rows each table makes required or only permitted;
//! this module pins that no catalogue type serves part of a group.

use super::*;

use crate::catalogue::representatives;
use crate::property_metadata::has_property;

use PropertyIdentifier as P;

const COMMAND_GROUP: [P; 3] = [
    P::PRIORITY_ARRAY,
    P::RELINQUISH_DEFAULT,
    P::CURRENT_COMMAND_PRIORITY,
];

/// The catalogue's commandable types: the analog, binary and multi-state
/// outputs and values, the twelve value types, the Access Door and the two
/// lighting outputs. A type gaining or losing Priority_Array changes it.
const COMMANDABLE_TYPES: usize = 21;

/// The reporting rows beyond Event_State.
#[cfg(feature = "event-reporting")]
const REPORTING_GROUP: [P; 6] = [
    P::EVENT_DETECTION_ENABLE,
    P::NOTIFICATION_CLASS,
    P::EVENT_ENABLE,
    P::ACKED_TRANSITIONS,
    P::NOTIFY_TYPE,
    P::EVENT_TIME_STAMPS,
];

/// Each catalogue object, with the Staging object started as a server
/// starts it, so every row reads.
fn started_catalogue() -> Vec<Box<dyn BACnetObject>> {
    let mut objects = representatives();
    for object in objects
        .iter_mut()
        .filter(|object| object.object_identifier().object_type() == ObjectType::STAGING)
    {
        object
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(5.0), None)
            .unwrap();
    }
    objects
}

/// The members of `group` that `object` has a row for.
fn present(object: &dyn BACnetObject, group: &[P]) -> Vec<P> {
    let rows = object.property_metadata();
    group
        .iter()
        .copied()
        .filter(|&property| has_property(&rows, property))
        .collect()
}

#[test]
fn command_rows_come_together() {
    let mut commanded = 0;
    for object in started_catalogue() {
        let object_type = object.object_identifier().object_type();
        let have = present(object.as_ref(), &COMMAND_GROUP);
        if have.is_empty() {
            continue;
        }
        assert_eq!(
            have, COMMAND_GROUP,
            "{object_type:?} serves part of the group"
        );
        for property in COMMAND_GROUP {
            assert!(
                object.read_property(property, None).is_ok(),
                "{object_type:?} {property:?} must read"
            );
        }
        commanded += 1;
    }
    assert_eq!(commanded, COMMANDABLE_TYPES, "commandable catalogue types");
}

#[test]
fn the_inhibit_reference_comes_with_the_inhibit() {
    for object in representatives() {
        let have = present(
            object.as_ref(),
            &[P::EVENT_ALGORITHM_INHIBIT_REF, P::EVENT_ALGORITHM_INHIBIT],
        );
        assert!(
            have.len() != 1,
            "{:?} serves only {have:?} of the inhibit pair",
            object.object_identifier().object_type()
        );
    }
}

/// The objects that lend event reporting serve the whole group, each row
/// required and readable; the others serve none of it.
#[cfg(feature = "event-reporting")]
#[test]
fn every_reporter_serves_the_whole_reporting_group() {
    let mut reporters = 0;
    for object in started_catalogue() {
        let object_type = object.object_identifier().object_type();
        let have = present(object.as_ref(), &REPORTING_GROUP);
        if object.event_reporting().is_none() {
            // The Notification Class object's own number is no reporting row.
            let have: Vec<P> = have
                .into_iter()
                .filter(|&property| {
                    !(object_type == ObjectType::NOTIFICATION_CLASS
                        && property == P::NOTIFICATION_CLASS)
                })
                .collect();
            assert!(
                have.is_empty(),
                "{object_type:?} doesn't report but serves {have:?}"
            );
            continue;
        }
        let rows = object.property_metadata();
        let required = crate::test_rows::required(&rows);
        for property in [P::EVENT_STATE].iter().chain(&REPORTING_GROUP) {
            assert!(
                has_property(&rows, *property),
                "{object_type:?} reports but serves no {property:?}"
            );
            assert!(
                required.contains(property),
                "{object_type:?} {property:?} is required of a reporter"
            );
            assert!(
                object.read_property(*property, None).is_ok(),
                "{object_type:?} {property:?} must read"
            );
        }
        reporters += 1;
    }
    assert_eq!(reporters, 16, "the reporters `event_reporting_tests` lists");
}
