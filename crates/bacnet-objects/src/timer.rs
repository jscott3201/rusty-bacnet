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
//! Without State_Change_Values, List_Of_Object_Property_References and
//! Priority_For_Writing a transition makes no writes. With them
//! ([`TimerObject::set_state_change`]) each transition queues writes of its
//! configured value to every referenced property, which the bundled server
//! makes as it makes a Command's (`state_change`, #2206).
//!
//! **On deletion** (#2265) a Timer releases nothing and makes no transition's
//! writes: deletion isn't a transition (Clause 12.57), and each
//! transition's writes are one-shot. A configuration that wants a slot given
//! up says so with a NULL State_Change_Value (Clause 12.57.20). So a running
//! Timer deleted leaves its start writes (IDLE_TO_RUNNING and the like) in
//! its targets' slots, and its expiry writes never come; to have
//! RUNNING_TO_IDLE's writes made, clear the Timer (Timer_State IDLE) before
//! deleting it. The writes of a transition still in progress stop as a
//! Command's do, with the same gap for a request to another device that
//! already passed its last check.
//!
//! A rebuilt Timer starts IDLE with Last_State_Change NONE. Built
//! [`with_persistence`](TimerObject::with_persistence), it keeps the three
//! properties above once a peer has written State_Change_Values or the
//! references (`saving`, #2243), and a Default_Timeout a peer wrote
//! (#2312); nothing else survives a restart.

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
mod saving;
#[cfg(test)]
mod saving_tests;
mod state;
mod state_change;
#[cfg(test)]
mod state_change_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod timeout_saving_tests;
#[cfg(all(test, feature = "durable-writes"))]
mod unchanged_setter_tests;

use state::Countdown;
pub use state_change::{MAX_TIMER_REFERENCES, StateChangeValues};

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
    /// What a write may change, with the range it is checked against.
    config: TimerConfig,
    /// Default_Timeout as the application last set it: what applies again
    /// when a saved, written one no longer fits the range.
    configured_timeout: u64,
    countdown: Countdown,
    update_time: (Date, Time),
    /// Set on entering IDLE or EXPIRED; while RUNNING the read computes it.
    expiration_time: (Date, Time),
    status_flags: StatusFlags,
    event_state: EventState,
    out_of_service: bool,
    reliability: Reliability,
    saved_reliability: Option<Reliability>,
    /// The run the set drives, and how the last one ended.
    run: state_change::StateChangeRun,
    /// Where a written configuration is saved, with persistence.
    config_storage: Option<crate::write_config::saving::Storage>,
    /// Changes to the set so far, so a staged write can tell whether
    /// another came between.
    config_writes: u64,
    clock: Option<Arc<dyn ClockReader>>,
    monotonic_clock: Option<Arc<MonotonicClock>>,
    deadline_waker: Option<Arc<DeadlineWaker>>,
    /// Elapsed time from the test-only `advance_time`, used when no monotonic
    /// clock is bound.
    logical_now: Duration,
}

/// What a Timer serves that a peer's write may change, as one value, so a
/// staged write can take it whole (`saving`): the set, Default_Timeout, and
/// the range a written Default_Timeout must lie in, which only the
/// application sets.
#[derive(Clone)]
pub(crate) struct TimerConfig {
    /// The three properties, once [`TimerObject::set_state_change`] or a
    /// restore has given them.
    set: Option<state_change::TimerSet>,
    default_timeout: u64,
    /// A write set Default_Timeout, now or before a restart: with storage,
    /// it is kept.
    timeout_written: bool,
    /// A saved, written Default_Timeout the range a new Timer takes doesn't
    /// hold, kept aside unserved until the application's range decides it
    /// (`set_pres_value_range`). Storage keeps it meanwhile.
    held_timeout: Option<u64>,
    min_pres_value: u64,
    max_pres_value: u64,
    /// Description as a write set it, while storage keeps it (#2364); the
    /// Timer serves the same text. `None` without storage, until a write,
    /// and after `clear_written_config`.
    description: Option<String>,
}

impl TimerConfig {
    fn in_range(&self, timeout_ms: u64) -> bool {
        (self.min_pres_value..=self.max_pres_value).contains(&timeout_ms)
    }
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
            config: TimerConfig {
                set: None,
                default_timeout: DEFAULT_TIMEOUT,
                timeout_written: false,
                held_timeout: None,
                min_pres_value: DEFAULT_MIN_PRES_VALUE,
                max_pres_value: DEFAULT_MAX_PRES_VALUE,
                description: None,
            },
            configured_timeout: DEFAULT_TIMEOUT,
            countdown: Countdown::Stopped,
            update_time: crate::clock::UNSPECIFIED_DATETIME,
            expiration_time: crate::clock::UNSPECIFIED_DATETIME,
            status_flags: StatusFlags::empty(),
            event_state: EventState::NORMAL,
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            saved_reliability: None,
            run: state_change::StateChangeRun::new(),
            config_storage: None,
            config_writes: 0,
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
    ///
    /// On a Timer [`with_persistence`](Self::with_persistence) serving a
    /// saved Default_Timeout a peer wrote
    /// ([`default_timeout_saved`](Self::default_timeout_saved)), the check
    /// is of the application's own value, the last one `set_default_timeout`
    /// took. If the saved value lies outside the new range, the application
    /// narrowed the range since the write: the saved value is dropped with a
    /// warning, the application's value applies, and storage is saved
    /// without it (#2312).
    ///
    /// A saved value [`with_persistence`](Self::with_persistence) held aside,
    /// one past the range a new Timer takes, is decided here too: served,
    /// and kept, if the new range holds it, dropped as above if not. Until a
    /// range is set it stays held: storage keeps it, and the application's
    /// value is served.
    ///
    /// The range already served changes nothing a request being saved
    /// depends on, so it leaves that request alone, unless a held or saved
    /// Default_Timeout is decided by it: that changes what is served and what
    /// storage holds, and makes the request give way (#2552).
    pub fn set_pres_value_range(&mut self, min: u64, max: u64) -> Result<(), Error> {
        let saved = self.default_timeout_saved();
        let kept = if saved {
            self.configured_timeout
        } else {
            self.config.default_timeout
        };
        if min == 0 || min > max || !(min..=max).contains(&kept) {
            return Err(common::value_out_of_range_error());
        }
        let unchanged = (self.config.min_pres_value, self.config.max_pres_value) == (min, max);
        self.config.min_pres_value = min;
        self.config.max_pres_value = max;
        // A held value is decided and a saved one past the range dropped
        // whatever the range was: either changes what is served and what
        // storage holds, so those count as a change too (#2552).
        let decides = self.config.held_timeout.is_some()
            || (saved && !self.config.in_range(self.config.default_timeout));
        // A staged write worked out under the old range gives way.
        if !unchanged || decides {
            self.config_changed();
        }
        if let Some(held) = self.config.held_timeout {
            if self.config.in_range(held) {
                self.config.default_timeout = held;
                self.config.timeout_written = true;
                self.config.held_timeout = None;
            } else {
                self.drop_saved_timeout(held);
            }
        } else if saved && !self.config.in_range(self.config.default_timeout) {
            self.drop_saved_timeout(self.config.default_timeout);
        }
        Ok(())
    }

    /// Set Default_Timeout, in milliseconds, as a write would: a value
    /// outside Min_Pres_Value..=Max_Pres_Value is refused with PROPERTY /
    /// VALUE_OUT_OF_RANGE.
    ///
    /// On a Timer [`with_persistence`](Self::with_persistence) whose storage
    /// holds a Default_Timeout a peer wrote
    /// ([`default_timeout_saved`](Self::default_timeout_saved)), the saved
    /// value wins, as for [`set_state_change`](Self::set_state_change): the
    /// argument is checked and remembered, but not served, until
    /// [`clear_written_config`](Self::clear_written_config) or a narrowed
    /// range drops the saved value.
    pub fn set_default_timeout(&mut self, timeout_ms: u64) -> Result<(), Error> {
        if !self.in_range(timeout_ms) {
            return Err(common::value_out_of_range_error());
        }
        self.configured_timeout = timeout_ms;
        if self.default_timeout_saved() {
            log_debug!(timer = %self.oid, "Saved Default_Timeout kept over a configured one");
            return Ok(());
        }
        if self.config.default_timeout != timeout_ms {
            self.config.default_timeout = timeout_ms;
            self.config_changed();
        }
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
        self.config.in_range(timeout_ms)
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
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
        match property {
            PropertyIdentifier::RELIABILITY => {
                return Ok(PropertyValue::Enumerated(
                    self.served_reliability().to_raw(),
                ));
            }
            PropertyIdentifier::STATUS_FLAGS => {
                return Ok(common::compute_status_flags(
                    self.status_flags,
                    self.served_reliability(),
                    self.out_of_service,
                    self.event_state,
                ));
            }
            _ => {}
        }
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        if let Some(result) = self.read_state_change(property, array_index) {
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
                Ok(PropertyValue::Unsigned(self.config.default_timeout))
            }
            p if p == PropertyIdentifier::MIN_PRES_VALUE => {
                Ok(PropertyValue::Unsigned(self.config.min_pres_value))
            }
            p if p == PropertyIdentifier::MAX_PRES_VALUE => {
                Ok(PropertyValue::Unsigned(self.config.max_pres_value))
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        // Every write, a refused one included, first applies an expiry that
        // is already due.
        self.apply_due_expiry();
        let was_out_of_service = self.out_of_service;
        if property == PropertyIdentifier::OUT_OF_SERVICE && !was_out_of_service {
            // A client simulates Reliability from what it reads now.
            self.reliability = self.served_reliability();
        }
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
        // Saved first with storage (`saving`, #2364).
        if self.config_storage.is_some()
            && property == PropertyIdentifier::DESCRIPTION
            && !crate::write_config::description::null_write(property, &value)
        {
            let write = crate::durable::PendingWrite {
                property,
                array_index,
                value,
            };
            return crate::write_config::saving::write(self, write);
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) = self.write_state_change(property, array_index, &value) {
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
            // Saved first with storage (`saving`, #2312).
            p if p == PropertyIdentifier::DEFAULT_TIMEOUT => crate::write_config::saving::write(
                self,
                crate::durable::PendingWrite {
                    property,
                    array_index,
                    value,
                },
            ),
            p if p == PropertyIdentifier::RELIABILITY => self.write_reliability(value),
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn timed(&mut self) -> Option<&mut dyn crate::traits::Timed> {
        Some(self)
    }

    fn command_runner(&self) -> Option<&dyn crate::traits::CommandRunner> {
        self.config
            .set
            .is_some()
            .then_some(self as &dyn crate::traits::CommandRunner)
    }

    fn command_runner_mut(&mut self) -> Option<&mut dyn crate::traits::CommandRunner> {
        match self.config.set {
            Some(_) => Some(self),
            None => None,
        }
    }

    #[cfg(feature = "durable-writes")]
    fn durable_writes(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        match self.config_storage {
            Some(_) => Some(self),
            None => None,
        }
    }
}

impl crate::traits::Timed for TimerObject {
    fn bind_clocks(&mut self, clocks: &crate::traits::Clocks) {
        self.clock = clocks.clock.clone();
        self.monotonic_clock = clocks.monotonic.clone();
        self.deadline_waker = clocks.deadline_waker.clone();
    }

    /// `true` when a countdown expired, and also while a transition's writes
    /// wait to be taken: a refused write can make one (the due expiry it
    /// applies first) with nothing to take it, so the pass takes it.
    fn advance_monotonic_time(&mut self, now: Duration) -> bool {
        self.expire_if_due(now) | self.has_pending_writes()
    }

    fn next_monotonic_deadline(&self) -> Option<Duration> {
        match self.countdown {
            Countdown::Counting { deadline } => Some(deadline),
            _ => None,
        }
    }
}

#[cfg(test)]
impl TimerObject {
    // Only tests call this. It moves `logical_now`, which stands in for the
    // monotonic clock when none is bound, so it assumes one time domain.
    pub(crate) fn advance_time(&mut self, elapsed: Duration) -> bool {
        self.logical_now = self.logical_now.saturating_add(elapsed);
        self.expire_if_due(self.logical_now)
    }
}
