//! Exactly the Schedule lends `ScheduleRunner` and exactly the Calendar
//! lends `CalendarState` (#2179). A lender dropped from, or added to, the
//! catalogue fails here until its table changes with it.

use crate::catalogue::representatives;
use alloc::collections::BTreeSet;
use bacnet_types::enums::ObjectType;

/// The built-in type that lends a schedule runner.
const SCHEDULE_RUNNER_LENDERS: [ObjectType; 1] = [ObjectType::SCHEDULE];

/// The built-in type that lends a calendar state.
const CALENDAR_STATE_LENDERS: [ObjectType; 1] = [ObjectType::CALENDAR];

fn lend_rule(object_type: ObjectType, lenders: &[ObjectType], lends: bool, capability: &str) {
    let expected = lenders.contains(&object_type);
    assert_eq!(
        lends,
        expected,
        "{object_type:?} {} lend {capability}",
        if expected { "must" } else { "must not" }
    );
}

#[test]
fn exactly_the_schedule_lends_schedule_runner() {
    let mut seen = BTreeSet::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        lend_rule(
            object_type,
            &SCHEDULE_RUNNER_LENDERS,
            object.schedule_runner().is_some(),
            "ScheduleRunner",
        );
        seen.insert(object_type.to_raw());
    }
    for object_type in SCHEDULE_RUNNER_LENDERS {
        assert!(seen.contains(&object_type.to_raw()), "{object_type:?}");
    }
}

#[test]
fn exactly_the_calendar_lends_calendar_state() {
    let mut seen = BTreeSet::new();
    for object in representatives() {
        let object_type = object.object_identifier().object_type();
        lend_rule(
            object_type,
            &CALENDAR_STATE_LENDERS,
            object.calendar_state().is_some(),
            "CalendarState",
        );
        seen.insert(object_type.to_raw());
    }
    for object_type in CALENDAR_STATE_LENDERS {
        assert!(seen.contains(&object_type.to_raw()), "{object_type:?}");
    }
}
