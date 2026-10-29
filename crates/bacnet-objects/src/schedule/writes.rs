//! Network writes of Weekly_Schedule, Exception_Schedule and Effective_Period
//! (#1057).
//!
//! The server passes these properties on as their raw wire bytes, one
//! `ApplicationData`. A read returns the two arrays as a `List` with one
//! `ApplicationData` per element, and a write takes that shape as well, so a
//! value read can be written back unchanged. The bytes are decoded with the
//! shared Clause 21 codecs in `bacnet_encoding::constructed` and then checked
//! by the same functions the local setters call, so both paths refuse the same
//! values with the same errors. A refused write changes nothing.
//!
//! Errors follow the Clause 15.9.1.3 split. An element that starts with a tag
//! the property's datatype never begins with (an application-tagged Real
//! where a daily schedule belongs, say) is INVALID_DATA_TYPE. One that starts
//! right but doesn't decode is INVALID_DATA_ENCODING. After decoding, an
//! event priority outside 1 to 16 or a time that isn't specific is
//! VALUE_OUT_OF_RANGE, as from the local setters (the codec decodes any
//! Unsigned priority and leaves its range to [`evaluation::check_special_event`],
//! #1087), a time given twice in one list is DUPLICATE_ENTRY (Clauses 12.24.7
//! and 12.24.8), and an Exception_Schedule longer than [`MAX_EXCEPTIONS`] is
//! NO_SPACE_TO_WRITE_PROPERTY.
//!
//! Weekly_Schedule always has seven elements: a whole write must carry seven
//! daily schedules (VALUE_OUT_OF_RANGE otherwise) and index 0 can't be
//! written. Exception_Schedule takes any length up to the cap; writing its
//! index 0 resizes it, truncating or appending empty special events.
//! Clause 12.24.8 asks only that an appended event have no time-values; this
//! object gives it a calendar-entry date with every octet unspecified and
//! event priority 16, which never supplies a value.

use crate::prelude::*;
use bacnet_encoding::constructed::{
    decode_daily_schedule, decode_date_range, decode_special_event,
};
use bacnet_encoding::tags::{self, TagClass, app_tag};
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetSpecialEvent, BACnetTimeValue, SpecialEventPeriod,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::{ScheduleObject, evaluation, unspecified_date};
use crate::common;

/// Resource cap on Exception_Schedule elements, the bound Calendar puts on
/// its Date_List.
pub(crate) const MAX_EXCEPTIONS: usize = 1024;

/// The error for an Exception_Schedule longer than [`MAX_EXCEPTIONS`].
pub(super) fn no_space_error() -> Error {
    common::protocol_error(ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)
}

/// The element appended when a write to index 0 lengthens Exception_Schedule.
fn empty_special_event() -> BACnetSpecialEvent {
    BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(unspecified_date())),
        list_of_time_values: Vec::new(),
        event_priority: 16,
    }
}

/// The daily schedules in a written Weekly_Schedule value: each opens with
/// context tag `[0]`.
fn decode_days(value: PropertyValue) -> Result<Vec<Vec<BACnetTimeValue>>, Error> {
    common::decode_elements(&value, |tag| tag.is_opening_tag(0), decode_daily_schedule)
}

/// The special events in a written Exception_Schedule value: each opens with
/// its period, a calendar entry under `[0]` or a Calendar reference `[1]`.
fn decode_events(value: PropertyValue) -> Result<Vec<BACnetSpecialEvent>, Error> {
    common::decode_elements(
        &value,
        |tag| tag.is_opening_tag(0) || tag.is_context(1),
        decode_special_event,
    )
}

/// The one element an indexed write carries.
fn single<T>(mut elements: Vec<T>) -> Result<T, Error> {
    match elements.pop() {
        Some(element) if elements.is_empty() => Ok(element),
        _ => Err(common::invalid_data_encoding_error()),
    }
}

/// The zero-based slot of a one-based array index, if it is below `len`.
fn slot(index: u32, len: usize) -> Result<usize, Error> {
    usize::try_from(index - 1)
        .ok()
        .filter(|slot| *slot < len)
        .ok_or_else(common::invalid_array_index_error)
}

impl ScheduleObject {
    /// WriteProperty of Weekly_Schedule, whole or one day.
    pub(super) fn write_weekly_schedule(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        match array_index {
            None => {
                let days: [Vec<BACnetTimeValue>; 7] = decode_days(value)?
                    .try_into()
                    .map_err(|_| common::value_out_of_range_error())?;
                for day in &days {
                    evaluation::check_time_values(day)?;
                }
                self.weekly_schedule = days;
                self.contents_changed();
                Ok(())
            }
            Some(0) => Err(common::write_access_denied_error()),
            Some(index) => {
                let day = slot(index, self.weekly_schedule.len())?;
                self.set_weekly_schedule(day, single(decode_days(value)?)?)
            }
        }
    }

    /// WriteProperty of Exception_Schedule: whole, one event, or its size at
    /// index 0.
    pub(super) fn write_exception_schedule(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        match array_index {
            None => {
                let events = decode_events(value)?;
                if events.len() > MAX_EXCEPTIONS {
                    return Err(no_space_error());
                }
                for event in &events {
                    evaluation::check_special_event(event)?;
                }
                self.exception_schedule = events;
            }
            Some(0) => {
                let PropertyValue::Unsigned(size) = value else {
                    return Err(common::invalid_data_type_error());
                };
                let size = usize::try_from(size)
                    .ok()
                    .filter(|size| *size <= MAX_EXCEPTIONS)
                    .ok_or_else(no_space_error)?;
                self.exception_schedule
                    .resize_with(size, empty_special_event);
            }
            Some(index) => {
                let slot = slot(index, self.exception_schedule.len())?;
                let event = single(decode_events(value)?)?;
                evaluation::check_special_event(&event)?;
                self.exception_schedule[slot] = event;
            }
        }
        self.contents_changed();
        Ok(())
    }

    /// WriteProperty of Effective_Period: one date range, two application
    /// Dates, through the local setter's check.
    pub(super) fn write_effective_period(&mut self, value: PropertyValue) -> Result<(), Error> {
        let PropertyValue::ApplicationData(bytes) = value else {
            return Err(common::invalid_data_type_error());
        };
        match tags::decode_tag(&bytes, 0) {
            Ok((tag, _)) if tag.class == TagClass::Application && tag.number == app_tag::DATE => {}
            Ok(_) => return Err(common::invalid_data_type_error()),
            Err(_) => return Err(common::invalid_data_encoding_error()),
        }
        let (period, end) =
            decode_date_range(&bytes, 0).map_err(|_| common::invalid_data_encoding_error())?;
        if end != bytes.len() {
            return Err(common::invalid_data_encoding_error());
        }
        self.set_effective_period(period)
    }
}
