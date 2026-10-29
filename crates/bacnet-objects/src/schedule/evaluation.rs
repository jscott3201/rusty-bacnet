//! Present_Value calculation for the Schedule object (Clause 12.24.4) and the
//! value checks its schedules share (#1028).
//!
//! Within Effective_Period, the value comes from the first of these sources
//! that has one:
//!
//! 1. Exception_Schedule: among the special events in effect today, the one
//!    with the best event priority (1 is best; the lower array index breaks a
//!    tie) whose current value is not NULL. An event is in effect when its
//!    inline calendar entry matches today or the Calendar it references is
//!    TRUE today.
//! 2. Today's Weekly_Schedule entry, if its current value is not NULL.
//! 3. Schedule_Default, which may itself be NULL.
//!
//! A list of time-values' current value is the value of its latest entry at
//! or before the current time; before the first entry it has none. A NULL
//! value therefore ends a scheduled period and hands control to the next
//! source. Outside Effective_Period the object is inactive: Present_Value
//! keeps its last value and nothing is written.

use crate::collections::HashSet;
use crate::prelude::*;

use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{
    BACnetObjectPropertyReference, BACnetSpecialEvent, BACnetTimeValue, SpecialEventPeriod,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};

use super::ScheduleObject;
use crate::common;

/// The writes one Schedule evaluation owes to List_Of_Object_Property_References.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleWrite {
    /// The new Present_Value in the scheduled value's own datatype. A NULL
    /// relinquishes the slot at `priority` in a commandable target.
    pub value: PropertyValue,
    /// Priority_For_Writing, 1 to 16: the priority each target is written at.
    pub priority: u8,
    /// The complete local references, target array indices included.
    pub references: Vec<BACnetObjectPropertyReference>,
    /// Whether this write only offers the value the references already owe
    /// again, to those whose last write was refused (#1436). A Schedule makes
    /// it on every pass with nothing else to send while a refusal stands, so
    /// the server logs a target's failure of it at debug, not warn.
    pub retry: bool,
}

/// How one target took a [`ScheduleWrite`], as the server reports it back
/// through
/// [`complete_schedule_write`](crate::traits::BACnetObject::complete_schedule_write)
/// (#1086).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleTargetOutcome {
    /// The target took the value.
    Accepted,
    /// The target refused the value's datatype: INVALID_DATA_TYPE or
    /// DATATYPE_NOT_SUPPORTED.
    DatatypeRefused,
    /// The reference names nothing the target can write, whatever the value
    /// (#1433): UNKNOWN_OBJECT (a missing target object included),
    /// UNKNOWN_PROPERTY, PROPERTY_IS_NOT_AN_ARRAY or INVALID_ARRAY_INDEX.
    ReferenceRefused,
    /// Any other failure. It says nothing about the configuration:
    /// WRITE_ACCESS_DENIED, for one, can come from the target's state
    /// (Out_Of_Service, a lock) and pass with it. On a
    /// [`retry`](ScheduleWrite::retry) it ends the member's refusal, since
    /// the target no longer refuses it (#1436).
    Failed,
}

impl ScheduleTargetOutcome {
    /// Classify the result of one target write.
    pub fn of(result: &Result<(), Error>) -> Self {
        const DATATYPE: [ErrorCode; 2] = [
            ErrorCode::INVALID_DATA_TYPE,
            ErrorCode::DATATYPE_NOT_SUPPORTED,
        ];
        const REFERENCE: [ErrorCode; 4] = [
            ErrorCode::UNKNOWN_OBJECT,
            ErrorCode::UNKNOWN_PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            ErrorCode::INVALID_ARRAY_INDEX,
        ];
        let code = match result {
            Ok(()) => return Self::Accepted,
            Err(
                Error::Remote(RemoteError::Error { code, .. }) | Error::Structured { code, .. },
            ) => *code,
            Err(_) => return Self::Failed,
        };
        let names = |codes: &[ErrorCode]| codes.contains(&code);
        if names(&DATATYPE) {
            Self::DatatypeRefused
        } else if names(&REFERENCE) {
            Self::ReferenceRefused
        } else {
            Self::Failed
        }
    }
}

impl ScheduleObject {
    /// Present_Value per Clause 12.24.4 at `time` on `today`, or `None` when
    /// `today` is outside Effective_Period.
    ///
    /// `calendar_active` answers for a special event whose period references
    /// a Calendar: whether that Calendar is TRUE on `today`. A reference to a
    /// missing object or to anything but a Calendar should answer `false`.
    /// Out_Of_Service does not change the calculation.
    pub fn evaluate(
        &self,
        today: SpecificDate,
        time: Time,
        calendar_active: &dyn Fn(ObjectIdentifier) -> bool,
    ) -> Option<PropertyValue> {
        if !self.effective_period.contains(today) {
            return None;
        }
        let mut exception: Option<(u64, &PropertyValue)> = None;
        for event in &self.exception_schedule {
            // Iterating in array order, a later event wins only with a
            // strictly better priority.
            if exception.is_some_and(|(best, _)| best <= event.event_priority) {
                continue;
            }
            let in_effect = match &event.period {
                SpecialEventPeriod::CalendarEntry(entry) => entry.matches(today),
                SpecialEventPeriod::CalendarReference(calendar) => calendar_active(*calendar),
            };
            if !in_effect {
                continue;
            }
            if let Some(value) = current_value(&event.list_of_time_values, time) {
                exception = Some((event.event_priority, value));
            }
        }
        if let Some((_, value)) = exception {
            return Some(value.clone());
        }
        let weekly = &self.weekly_schedule[usize::from(today.weekday() - 1)];
        Some(
            current_value(weekly, time)
                .unwrap_or(&self.schedule_default)
                .clone(),
        )
    }
}

/// The current value of a list of time-values at `time`: that of the latest
/// entry at or before `time`, or `None` before the first entry or when that
/// entry's value is NULL.
fn current_value(entries: &[BACnetTimeValue], time: Time) -> Option<&PropertyValue> {
    let key = |t: &Time| (t.hour, t.minute, t.second, t.hundredths);
    entries
        .iter()
        .filter(|entry| key(&entry.time) <= key(&time))
        .max_by_key(|entry| key(&entry.time))
        .map(|entry| &entry.value)
        .filter(|value| **value != PropertyValue::Null)
}

/// Check one list of time-values: each time specific (VALUE_OUT_OF_RANGE),
/// each value primitive (INVALID_DATA_TYPE), and no time twice
/// (DUPLICATE_ENTRY, as Clauses 12.24.7 and 12.24.8 ask of a written day).
pub(super) fn check_time_values(entries: &[BACnetTimeValue]) -> Result<(), Error> {
    let mut times = HashSet::with_capacity(entries.len());
    for entry in entries {
        if !entry.time.is_specific() {
            return Err(common::value_out_of_range_error());
        }
        if !entry.value.is_primitive() {
            return Err(common::invalid_data_type_error());
        }
        if !times.insert(entry.time) {
            return Err(common::protocol_error(
                ErrorClass::PROPERTY,
                ErrorCode::DUPLICATE_ENTRY,
            ));
        }
    }
    Ok(())
}

/// Check a special event: event priority 1 to 16 and an inline calendar entry
/// in range (VALUE_OUT_OF_RANGE), then its time-values.
pub(super) fn check_special_event(event: &BACnetSpecialEvent) -> Result<(), Error> {
    let period_ok = match &event.period {
        SpecialEventPeriod::CalendarEntry(entry) => entry.is_valid(),
        SpecialEventPeriod::CalendarReference(_) => true,
    };
    if !(1..=16).contains(&event.event_priority) || !period_ok {
        return Err(common::value_out_of_range_error());
    }
    check_time_values(&event.list_of_time_values)
}
