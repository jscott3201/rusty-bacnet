//! The calendar-state capability: a Calendar's state on a day other than
//! the one its clock reads.

use bacnet_types::calendar::SpecificDate;

/// A Calendar that evaluates its own Date_List for any day.
///
/// The object returns itself from
/// [`BACnetObject::calendar_state`](crate::traits::BACnetObject::calendar_state).
/// The bundled server's schedule pass asks it, under the database guard, for
/// the day the pass evaluates, so every Schedule that references the
/// Calendar sees the same answer. A Calendar that lends none is read through
/// its Present_Value instead. Not public API yet (#2179).
#[doc(hidden)]
pub trait CalendarState {
    /// Whether Date_List matches `day`: the Calendar's Present_Value on
    /// that day (Clause 12.9).
    fn state_on(&self, day: SpecificDate) -> bool;
}
