//! Timer object (type 31) per ASHRAE 135-2020 Clause 12.57.
//!
//! A countdown timer. Present_Value is the time left in milliseconds while
//! the timer runs, and zero otherwise; Timer_State says whether it is idle,
//! running or expired. A client drives it through the Clause 12.57 state
//! machine by writing Present_Value (start with that timeout, or zero to
//! expire), Timer_Running (TRUE starts with Default_Timeout, FALSE expires)
//! or Timer_State (IDLE clears). Each transition records Last_State_Change
//! and Update_Time; Expiration_Time follows the state (`state`).
//!
//! Min_Pres_Value and Max_Pres_Value give the timeouts a write may start,
//! and Default_Timeout stays within them. While Out_Of_Service is TRUE the
//! state machine still answers writes, but a running timer doesn't count
//! down, and Reliability is writable.
//!
//! The object has no State_Change_Values, List_Of_Object_Property_References
//! or Priority_For_Writing, so a transition has no write requests to make
//! (#2206).
//!
//! A Timer keeps nothing across a restart: a rebuilt one starts IDLE with
//! Last_State_Change NONE.

use crate::prelude::*;
use alloc::borrow::Cow;
use alloc::sync::Arc;
use bacnet_types::enums::{
    EventState, ObjectType, PropertyIdentifier, Reliability, TimerState, TimerTransition,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use core::time::Duration;

use crate::clock::ClockReader;
use crate::common::{self, read_common_properties};
use crate::traits::{BACnetObject, DeadlineWaker, MonotonicClock};

mod metadata;
mod state;
#[cfg(test)]
mod tests;

use state::Countdown;

/// The shortest timeout a new Timer accepts, in milliseconds: zero is the
/// expire request, so it can't start a timer.
pub const DEFAULT_MIN_PRES_VALUE: u64 = 1;
/// The longest timeout a new Timer accepts, in milliseconds (about 49.7
/// days).
pub const DEFAULT_MAX_PRES_VALUE: u64 = u32::MAX as u64;
/// A new Timer's Default_Timeout, in milliseconds: one minute.
pub const DEFAULT_TIMEOUT: u64 = 60_000;

/// BACnet Timer object: a countdown timer driven by property writes.
pub struct TimerObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    timer_state: TimerState,
    last_state_change: TimerTransition,
    initial_timeout: u64,
    default_timeout: u64,
    min_pres_value: u64,
    max_pres_value: u64,
    countdown: Countdown,
    update_time: (Date, Time),
    /// Set on entering IDLE or EXPIRED; while RUNNING the read computes it.
    expiration_time: (Date, Time),
    status_flags: StatusFlags,
    event_state: EventState,
    out_of_service: bool,
    reliability: Reliability,
    saved_reliability: Option<Reliability>,
    clock: Option<Arc<dyn ClockReader>>,
    monotonic_clock: Option<Arc<MonotonicClock>>,
    deadline_waker: Option<Arc<DeadlineWaker>>,
    /// Elapsed time from `advance_time_internal`, used when no monotonic
    /// clock is bound.
    logical_now: Duration,
}

impl TimerObject {
    /// Create an idle Timer that accepts timeouts from
    /// [`DEFAULT_MIN_PRES_VALUE`] to [`DEFAULT_MAX_PRES_VALUE`] and starts
    /// with [`DEFAULT_TIMEOUT`] when Timer_Running is written TRUE.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::TIMER, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            timer_state: TimerState::IDLE,
            last_state_change: TimerTransition::NONE,
            initial_timeout: 0,
            default_timeout: DEFAULT_TIMEOUT,
            min_pres_value: DEFAULT_MIN_PRES_VALUE,
            max_pres_value: DEFAULT_MAX_PRES_VALUE,
            countdown: Countdown::Stopped,
            update_time: crate::clock::UNSPECIFIED_DATETIME,
            expiration_time: crate::clock::UNSPECIFIED_DATETIME,
            status_flags: StatusFlags::empty(),
            event_state: EventState::NORMAL,
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            saved_reliability: None,
            clock: None,
            monotonic_clock: None,
            deadline_waker: None,
            logical_now: Duration::ZERO,
        })
    }

    /// Set the timeouts a write may start the timer with, in milliseconds:
    /// Min_Pres_Value and Max_Pres_Value. `min` must be at least 1 and no
    /// more than `max`, and Default_Timeout must lie within the new range;
    /// otherwise this refuses with PROPERTY / VALUE_OUT_OF_RANGE and changes
    /// nothing.
    ///
    /// To narrow the range past the current Default_Timeout, call
    /// [`Self::set_default_timeout`] with a value inside the new range
    /// first. A timer already running keeps its countdown, even one whose
    /// time left now lies outside the narrowed range.
    pub fn set_pres_value_range(&mut self, min: u64, max: u64) -> Result<(), Error> {
        if min == 0 || min > max || !(min..=max).contains(&self.default_timeout) {
            return Err(common::value_out_of_range_error());
        }
        self.min_pres_value = min;
        self.max_pres_value = max;
        Ok(())
    }

    /// Set Default_Timeout, in milliseconds, as a write would: a value
    /// outside Min_Pres_Value..=Max_Pres_Value is refused with PROPERTY /
    /// VALUE_OUT_OF_RANGE.
    pub fn set_default_timeout(&mut self, timeout_ms: u64) -> Result<(), Error> {
        if !self.in_range(timeout_ms) {
            return Err(common::value_out_of_range_error());
        }
        self.default_timeout = timeout_ms;
        Ok(())
    }

    /// The timer's state.
    pub fn timer_state(&self) -> TimerState {
        self.timer_state
    }

    /// The last transition the state machine made.
    pub fn last_state_change(&self) -> TimerTransition {
        self.last_state_change
    }

    fn in_range(&self, timeout_ms: u64) -> bool {
        (self.min_pres_value..=self.max_pres_value).contains(&timeout_ms)
    }

    fn datetime(value: (Date, Time)) -> PropertyValue {
        PropertyValue::List(vec![
            PropertyValue::Date(value.0),
            PropertyValue::Time(value.1),
        ])
    }

    fn write_reliability(&mut self, value: PropertyValue) -> Result<(), Error> {
        if !self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        let PropertyValue::Enumerated(raw) = value else {
            return Err(common::invalid_data_type_error());
        };
        let reliability = Reliability::from_raw(raw);
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.reliability = reliability;
        Ok(())
    }
}

impl BACnetObject for TimerObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::TIMER.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.remaining()))
            }
            p if p == PropertyIdentifier::TIMER_STATE => {
                Ok(PropertyValue::Enumerated(self.served_state().to_raw()))
            }
            p if p == PropertyIdentifier::TIMER_RUNNING => Ok(PropertyValue::Boolean(
                self.served_state() == TimerState::RUNNING,
            )),
            p if p == PropertyIdentifier::LAST_STATE_CHANGE => Ok(PropertyValue::Enumerated(
                self.served_last_state_change().to_raw(),
            )),
            p if p == PropertyIdentifier::INITIAL_TIMEOUT => {
                Ok(PropertyValue::Unsigned(self.initial_timeout))
            }
            p if p == PropertyIdentifier::DEFAULT_TIMEOUT => {
                Ok(PropertyValue::Unsigned(self.default_timeout))
            }
            p if p == PropertyIdentifier::MIN_PRES_VALUE => {
                Ok(PropertyValue::Unsigned(self.min_pres_value))
            }
            p if p == PropertyIdentifier::MAX_PRES_VALUE => {
                Ok(PropertyValue::Unsigned(self.max_pres_value))
            }
            p if p == PropertyIdentifier::UPDATE_TIME => {
                Ok(Self::datetime(self.served_update_time()))
            }
            p if p == PropertyIdentifier::EXPIRATION_TIME => {
                Ok(Self::datetime(self.expiration_time_now()))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        // Every write, a refused one included, first applies an expiry that
        // is already due.
        self.apply_due_expiry();
        let was_out_of_service = self.out_of_service;
        if let Some(result) = common::write_out_of_service_with_reliability_restore(
            &mut self.out_of_service,
            &mut self.reliability,
            &mut self.saved_reliability,
            property,
            &value,
        ) {
            if was_out_of_service != self.out_of_service {
                self.service_changed();
            }
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                let PropertyValue::Unsigned(timeout) = value else {
                    return Err(common::invalid_data_type_error());
                };
                self.write_present_value(timeout)
            }
            p if p == PropertyIdentifier::TIMER_RUNNING => {
                let PropertyValue::Boolean(running) = value else {
                    return Err(common::invalid_data_type_error());
                };
                self.write_timer_running(running);
                Ok(())
            }
            p if p == PropertyIdentifier::TIMER_STATE => {
                let PropertyValue::Enumerated(raw) = value else {
                    return Err(common::invalid_data_type_error());
                };
                self.write_timer_state(TimerState::from_raw(raw))
            }
            p if p == PropertyIdentifier::DEFAULT_TIMEOUT => {
                let PropertyValue::Unsigned(timeout) = value else {
                    return Err(common::invalid_data_type_error());
                };
                self.set_default_timeout(timeout)
            }
            p if p == PropertyIdentifier::RELIABILITY => self.write_reliability(value),
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }

    // Only tests call this. It moves `logical_now`, which stands in for the
    // monotonic clock when none is bound, so it assumes one time domain.
    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        self.logical_now = self.logical_now.saturating_add(elapsed);
        self.expire_if_due(self.logical_now)
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.monotonic_clock = clock;
    }

    fn bind_deadline_waker_internal(&mut self, waker: Option<Arc<DeadlineWaker>>) {
        self.deadline_waker = waker;
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.expire_if_due(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        match self.countdown {
            Countdown::Counting { deadline } => Some(deadline),
            _ => None,
        }
    }
}
