//! The schedule-runner capability: the writes a Schedule owes the
//! object-property references it controls, and how its targets took them.

use crate::prelude::*;

use bacnet_types::calendar::SpecificDate;
use bacnet_types::primitives::{ObjectIdentifier, Time};

use crate::schedule::{ScheduleTargetOutcome, ScheduleWrite};

/// A Schedule whose writes to its controlled references the host makes.
///
/// The object returns itself from
/// [`BACnetObject::schedule_runner_mut`](crate::traits::BACnetObject::schedule_runner_mut)
/// for a schedule pass, and from
/// [`schedule_runner`](crate::traits::BACnetObject::schedule_runner) for
/// the query a host makes while it only borrows the database, whether a
/// refused reference names an object just added. An object that lends one
/// lends the other. The built-in [`ScheduleObject`](crate::schedule::ScheduleObject)
/// lends it.
///
/// The bundled server runs a pass every 60 seconds, and again for a
/// Schedule a write has just changed, under the database's write guard.
/// For each Schedule it first takes the
/// [owed writes](Self::take_owed_schedule_writes), then
/// [ticks](Self::tick_schedule) it against the Device clock, then writes
/// each value to every reference at the write's priority and reports how
/// each target took it through
/// [`complete_schedule_write`](Self::complete_schedule_write). Every target
/// is local: a Schedule's references name objects of its own device.
pub trait ScheduleRunner {
    /// Evaluate the schedule at `time` on `today` (Clause 12.24.4).
    ///
    /// `calendar_active` answers for a special event whose period
    /// references a Calendar: whether that Calendar is TRUE on `today`.
    /// Returns the write owed when Present_Value changed, the object has
    /// just entered its Effective_Period, or its references or priority
    /// changed since the last write, with the complete local references,
    /// target array indices included. Otherwise, while some references
    /// refused their last write, returns a
    /// [`retry`](ScheduleWrite::retry) of the current value to those
    /// references only (#1436), and `None` when nothing is owed.
    fn tick_schedule(
        &mut self,
        today: SpecificDate,
        time: Time,
        calendar_active: &dyn Fn(ObjectIdentifier) -> bool,
    ) -> Option<ScheduleWrite>;

    /// Take the writes the schedule owes its targets apart from its
    /// calculation, each once, in order: a NULL to every priority slot a
    /// change of List_Of_Object_Property_References or Priority_For_Writing
    /// left behind (#1088), then the Present_Value owed out of service, for
    /// a client's write (Clause 12.24.14) or a change of those two
    /// properties.
    ///
    /// A pass takes them before it calls
    /// [`tick_schedule`](Self::tick_schedule), so they reach the targets
    /// ahead of any calculated value; they need no clock. The default owes
    /// none.
    fn take_owed_schedule_writes(&mut self) -> Vec<ScheduleWrite> {
        Vec::new()
    }

    /// Report how each target took `write`, one of this schedule's writes:
    /// `outcomes` holds one entry per member of `write.references`, in
    /// order.
    ///
    /// A Schedule judges the reference half of its Reliability from these
    /// (Clause 12.24.13, #1086). Returns whether a readable property, such
    /// as Reliability, changed, so the host owes COV for it. The default
    /// records nothing and returns `false`.
    fn complete_schedule_write(
        &mut self,
        _write: &ScheduleWrite,
        _outcomes: &[ScheduleTargetOutcome],
    ) -> bool {
        false
    }

    /// The [`retry`](ScheduleWrite::retry) of the current value to the
    /// references whose last write was refused and that name `target`, an
    /// object just added to the database (#1440), so the refusal can clear
    /// without waiting for the next pass.
    ///
    /// `None` when no such refusal stands, or when a pass would retry
    /// nothing: out of service, outside Effective_Period, or with a NULL
    /// Present_Value. The default returns `None`.
    fn retry_refusals_naming(&self, _target: ObjectIdentifier) -> Option<ScheduleWrite> {
        None
    }
}

/// The two accessors of an object that lends itself as its
/// [`ScheduleRunner`], for inside its `BACnetObject` impl.
macro_rules! lend_schedule_runner {
    () => {
        fn schedule_runner(&self) -> Option<&dyn $crate::traits::ScheduleRunner> {
            Some(self)
        }

        fn schedule_runner_mut(&mut self) -> Option<&mut dyn $crate::traits::ScheduleRunner> {
            Some(self)
        }
    };
}

pub(crate) use lend_schedule_runner;
