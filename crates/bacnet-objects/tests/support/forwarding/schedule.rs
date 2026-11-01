//! The probe's schedule runner and calendar state, logged under
//! `ScheduleRunner::` and `CalendarState::`: every method answers unlike
//! its default.

use super::super::fixtures::{day, oid, schedule_write};
use super::{ObjectIdentifier, ObjectType, Probe, SpecificDate, Time};
use bacnet_objects::schedule::{ScheduleTargetOutcome, ScheduleWrite};
use bacnet_objects::traits::{CalendarState, ScheduleRunner};

impl ScheduleRunner for Probe {
    fn tick_schedule(
        &mut self,
        today: SpecificDate,
        time: Time,
        calendar_active: &dyn Fn(ObjectIdentifier) -> bool,
    ) -> Option<ScheduleWrite> {
        // The calendar callback is logged by what it answers.
        let calendars = [4, 5].map(|instance| calendar_active(oid(ObjectType::CALENDAR, instance)));
        self.called("ScheduleRunner::tick_schedule", (today, time, calendars));
        Some(schedule_write(2))
    }
    fn take_owed_schedule_writes(&mut self) -> Vec<ScheduleWrite> {
        self.called("ScheduleRunner::take_owed_schedule_writes", ());
        vec![schedule_write(5)]
    }
    fn complete_schedule_write(
        &mut self,
        write: &ScheduleWrite,
        outcomes: &[ScheduleTargetOutcome],
    ) -> bool {
        self.called("ScheduleRunner::complete_schedule_write", (write, outcomes));
        true
    }
    fn retry_refusals_naming(&self, target: ObjectIdentifier) -> Option<ScheduleWrite> {
        self.called("ScheduleRunner::retry_refusals_naming", (target,));
        Some(schedule_write(7))
    }
}

impl CalendarState for Probe {
    fn state_on(&self, date: SpecificDate) -> bool {
        self.called("CalendarState::state_on", (date,));
        date == day()
    }
}
