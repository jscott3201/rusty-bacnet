//! Feeding pulses to an Accumulator (#2259): the [`LocalInput`] that takes
//! them and the [`Timed`] window that turns them into Pulse_Rate.
//!
//! Pulse_Rate counts the pulses of the most recent Limit_Monitoring_Interval
//! (Clause 12.61.20); Clause 12.61.23 leaves the kind of window to the
//! implementation. This one is fixed, on the host's monotonic clock, and
//! integer-only: each window closes on a whole interval, publishes its count
//! and opens the next.

use alloc::boxed::Box;
use core::time::Duration;

use bacnet_types::error::Error;

use super::AccumulatorObject;
use crate::traits::capabilities::same_handle;
use crate::traits::{BACnetObject, Clocks, LocalInput, LocalInputValue, Timed, not_supported};

/// The Limit_Monitoring_Interval window fed pulses are counted in.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RateWindow {
    /// Pulses counted since the window opened, saturating.
    pulses: u64,
    /// The monotonic instant the window opened, or `None` until a clock
    /// or the first advance gives one.
    opened: Option<Duration>,
}

impl RateWindow {
    /// The instant the window closes, once it is open and the interval
    /// isn't zero.
    fn closes_at(&self, interval: u32) -> Option<Duration> {
        if interval == 0 {
            return None;
        }
        self.opened?
            .checked_add(Duration::from_secs(interval.into()))
    }

    /// Let time pass to `now`. Once a whole interval has passed, the window
    /// closes with every pulse counted in it, which this returns, and the
    /// next opens at the last whole interval before `now`, so the remainder
    /// carries and the next close is always later than `now`. A pass that
    /// spans several intervals closes only one window, holding every pulse
    /// counted since the last close.
    fn advance(&mut self, interval: u32, now: Duration) -> Option<u64> {
        if interval == 0 {
            return None;
        }
        let Some(opened) = self.opened else {
            self.opened = Some(now);
            return None;
        };
        let interval = u64::from(interval);
        let elapsed = now.saturating_sub(opened);
        if elapsed.as_secs() < interval {
            return None;
        }
        // At most `elapsed`, so the sum stays at or before `now`.
        let whole = elapsed.as_secs() / interval * interval;
        self.opened = Some(opened + Duration::from_secs(whole));
        Some(core::mem::take(&mut self.pulses))
    }
}

impl AccumulatorObject {
    /// The bound monotonic clock's reading, if one is bound.
    fn monotonic_now(&self) -> Option<Duration> {
        self.monotonic.as_ref().map(|clock| clock())
    }

    /// Start a new, empty rate window now, and wake the host's pass for
    /// its deadline. Without a bound clock, the next advance opens it.
    pub(super) fn restart_rate_window(&mut self) {
        self.rate_window = RateWindow {
            pulses: 0,
            opened: self.monotonic_now(),
        };
        if let Some(wake) = &self.deadline_waker {
            wake();
        }
    }

    /// Step Present_Value for `pulses` input pulses: one count per pulse,
    /// or Prescale's ratio when present, run as its remainder algorithm
    /// (Clause 12.61.13) in one step, and modulo Max_Pres_Value + 1
    /// (Clause 12.61.4). A modulo-divide of zero has no ratio, so it leaves
    /// Present_Value as it is.
    fn count_into_present_value(&mut self, pulses: u64) {
        let counts = match &self.prescale {
            None => u128::from(pulses),
            Some(prescale) if prescale.modulo_divide == 0 => return,
            Some(prescale) => {
                let divide = u128::from(prescale.modulo_divide);
                // Below 2^96, so it can't overflow.
                let total = u128::from(self.prescale_carry)
                    + u128::from(pulses) * u128::from(prescale.multiplier);
                self.prescale_carry =
                    u32::try_from(total % divide).expect("a remainder below a u32 divisor");
                total / divide
            }
        };
        let modulus = u128::from(self.max_pres_value) + 1;
        let stepped = (u128::from(self.present_value) + counts) % modulus;
        self.present_value = u64::try_from(stepped).expect("a value modulo at most 2^64");
    }
}

/// The pulses the application counts.
impl LocalInput for AccumulatorObject {
    /// Take [`LocalInputValue::Pulses`] and refuse every other input.
    ///
    /// The pulses always count toward the rate window, so a window that
    /// closes after a return to service holds the input's true count. While
    /// Out_Of_Service is TRUE, Present_Value is decoupled from the input
    /// (Clause 12.61.10): the pulses don't step it, and they are dropped
    /// rather than held for later, so the value a client simulated stays
    /// and counting resumes from it once back in service.
    fn apply_local_input(&mut self, input: LocalInputValue) -> Result<(), Error> {
        let LocalInputValue::Pulses(pulses) = input else {
            return Err(not_supported());
        };
        self.rate_window.pulses = self.rate_window.pulses.saturating_add(pulses);
        if !self.out_of_service {
            self.count_into_present_value(pulses);
        }
        Ok(())
    }
}

impl Timed for AccumulatorObject {
    /// A new monotonic clock is a new time base, so the window reopens on
    /// it, keeping the pulses it counted; a pass for another clock's sake
    /// leaves it alone.
    fn bind_clocks(&mut self, clocks: &Clocks) {
        self.deadline_waker = clocks.deadline_waker.clone();
        if !same_handle(self.monotonic.as_ref(), clocks.monotonic.as_ref()) {
            self.monotonic = clocks.monotonic.clone();
            self.rate_window.opened = self.monotonic_now();
        }
    }

    /// Close the window once a whole Limit_Monitoring_Interval has passed:
    /// Pulse_Rate becomes its count, `true` when that changed it. While
    /// Out_Of_Service is TRUE windows still close, but Pulse_Rate keeps a
    /// client's value (Clause 12.61.10).
    fn advance_monotonic_time(&mut self, now: Duration) -> bool {
        let Some(pulses) = self
            .rate_window
            .advance(self.limit_monitoring_interval, now)
        else {
            return false;
        };
        if self.out_of_service || self.pulse_rate == pulses {
            return false;
        }
        self.pulse_rate = pulses;
        true
    }

    fn next_monotonic_deadline(&self) -> Option<Duration> {
        self.rate_window.closes_at(self.limit_monitoring_interval)
    }

    /// The object as it reads now, without its clocks.
    fn cov_snapshot(&self) -> Option<Box<dyn BACnetObject>> {
        let mut snapshot = self.clone();
        snapshot.monotonic = None;
        snapshot.deadline_waker = None;
        Some(Box::new(snapshot))
    }
}
