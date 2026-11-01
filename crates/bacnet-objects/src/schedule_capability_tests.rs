//! Exactly the Schedule lends `ScheduleRunner`, through both accessors that
//! lend the same object, and exactly the Calendar lends `CalendarState`
//! (#2179). A lender dropped from, or added to, the catalogue fails here
//! until its table changes with it.

use crate::catalogue::representatives;
use crate::traits::ScheduleRunner;
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
fn exactly_the_schedule_lends_schedule_runner_through_both_accessors() {
    let mut seen = BTreeSet::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        let shared = object
            .schedule_runner()
            .map(|runner| (runner as *const dyn ScheduleRunner).cast::<()>());
        let exclusive = object
            .schedule_runner_mut()
            .map(|runner| (runner as *mut dyn ScheduleRunner).cast::<()>());
        assert_eq!(
            shared,
            exclusive.map(|pointer| pointer.cast_const()),
            "{object_type:?}: the two accessors must lend the same object"
        );
        lend_rule(
            object_type,
            &SCHEDULE_RUNNER_LENDERS,
            shared.is_some(),
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
