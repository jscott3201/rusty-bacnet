//! The time and clock capability: objects whose state moves with time.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::constructed::BACnetObjectPropertyReference;

use crate::clock::ClockReader;
use crate::traits::BACnetObject;

/// A process-local monotonic time source: the time since an origin the
/// host picks, which never goes back. The server's is the time since its
/// monotonic operation task started.
pub type MonotonicClock = dyn Fn() -> Duration + Send + Sync;

/// Wakes the host's monotonic pass to read object deadlines again (#1384),
/// for a write that arms a deadline sooner than the next routine pass.
pub type DeadlineWaker = dyn Fn() + Send + Sync;

/// The clocks an [`ObjectDatabase`](crate::database::ObjectDatabase) lends
/// every object it holds. Each is `None` until the host binds one.
///
/// Build one with [`Clocks::new`], or from [`Clocks::default`] with the
/// `with_*` setters; it may gain fields, so it has no struct literal outside
/// this crate.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct Clocks {
    /// The Device's wall clock, for stamping dates and times.
    pub clock: Option<Arc<dyn ClockReader>>,
    /// The monotonic source a write reads when it arms a deadline, on the
    /// time base [`Timed::advance_monotonic_time`] is called with.
    pub monotonic: Option<Arc<MonotonicClock>>,
    /// The waker to call when a write arms a deadline.
    pub deadline_waker: Option<Arc<DeadlineWaker>>,
}

impl Clocks {
    /// The three clocks, each `None` when it isn't bound.
    pub fn new(
        clock: Option<Arc<dyn ClockReader>>,
        monotonic: Option<Arc<MonotonicClock>>,
        deadline_waker: Option<Arc<DeadlineWaker>>,
    ) -> Self {
        Self {
            clock,
            monotonic,
            deadline_waker,
        }
    }

    /// These clocks with the Device's wall clock replaced.
    #[must_use]
    pub fn with_clock(mut self, clock: Option<Arc<dyn ClockReader>>) -> Self {
        self.clock = clock;
        self
    }

    /// These clocks with the monotonic clock replaced.
    #[must_use]
    pub fn with_monotonic(mut self, monotonic: Option<Arc<MonotonicClock>>) -> Self {
        self.monotonic = monotonic;
        self
    }

    /// These clocks with the deadline waker replaced.
    #[must_use]
    pub fn with_deadline_waker(mut self, deadline_waker: Option<Arc<DeadlineWaker>>) -> Self {
        self.deadline_waker = deadline_waker;
        self
    }
}

/// Whether two optional handles are the same one, or both absent. An object
/// whose binding restarts something uses it to skip a clock that didn't
/// change when [`Timed::bind_clocks`] runs for another clock's sake.
pub(crate) fn same_handle<T: ?Sized>(held: Option<&Arc<T>>, offered: Option<&Arc<T>>) -> bool {
    match (held, offered) {
        (Some(held), Some(offered)) => Arc::ptr_eq(held, offered),
        (None, None) => true,
        _ => false,
    }
}

/// How an object leaves its database, for [`Timed::retire`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Retirement {
    /// Removed for good: what the object keeps in storage follows the end
    /// of what it ran.
    Removed,
    /// Let go with its storage left as it was, for what comes next: an
    /// object put in its place, which may restore from the same storage,
    /// or the next start after one that failed.
    Released,
}

/// An object whose state moves with time: it stamps the wall clock, or
/// runs timers, fades, ramps or expiries off the monotonic clock.
///
/// The object returns itself from [`BACnetObject::timed`]. Every method
/// has a no-op default, so an object overrides only what it uses. Hosts call
/// these hooks with the object borrowed exclusively (the server holds the
/// database's write guard) and never hand an object a lock.
///
/// Expiring a staged write is not a timed behaviour: that is
/// [`DurableWrites::expire_staged`](crate::durable::DurableWrites::expire_staged).
pub trait Timed {
    /// Take the database's clocks. It runs as the object joins a database
    /// and again each time the host binds or removes any one of them, so an
    /// object whose binding restarts state checks whether the clock it uses
    /// changed.
    fn bind_clocks(&mut self, _clocks: &Clocks) {}

    /// Advance operations to `now`, an instant on the monotonic clock's time
    /// base. `true` when readable state changed, so the host takes a
    /// [`cov_snapshot`](Self::cov_snapshot) and reports COV from it.
    fn advance_monotonic_time(&mut self, _now: Duration) -> bool {
        false
    }

    /// The next instant at which [`advance_monotonic_time`] has work to do,
    /// if any. The host's pass wakes no later than that.
    ///
    /// [`advance_monotonic_time`]: Self::advance_monotonic_time
    fn next_monotonic_deadline(&self) -> Option<Duration> {
        None
    }

    /// Take the finest COV increment an active property subscription to
    /// Tracking_Value asks for, or `None` when no such subscription gives
    /// one (#1510).
    ///
    /// The host passes it before each advance, so an object whose fade or
    /// ramp samples Tracking_Value for COV samples it at least that finely,
    /// still no more often than its sample grid allows. The value is finite
    /// and not negative.
    fn set_tracking_cov_increment(&mut self, _finest: Option<f64>) {}

    /// The device is starting: drive the object's output as a start does.
    ///
    /// The server calls this once for every object as it starts, before it
    /// shares the database. The Color and Color Temperature objects drive an
    /// output nothing has driven yet to its default (#1526).
    fn start_output(&mut self) {}

    /// The object is leaving its database (#2479): `retirement` says how.
    /// End what it runs outside the database, such as a Load Control's
    /// shed, whose application is told to stop. The database calls this
    /// before it hands a removed object back or drops a replaced one, and a
    /// host whose start fails calls it on every object
    /// ([`release_outputs`]); dropping an object, or a copy of one, calls
    /// nothing.
    ///
    /// [`release_outputs`]: crate::database::ObjectDatabase::release_outputs
    fn retire(&mut self, _retirement: Retirement) {}

    /// A frozen copy of the COV-readable state, taken while the host still
    /// holds the object, for the COV report of an advance that changed it.
    /// `None` reports nothing.
    fn cov_snapshot(&self) -> Option<Box<dyn BACnetObject>> {
        None
    }

    /// Claim the Averaging sample due at the monotonic instant `now`: step
    /// the schedule past it and return the property to read, which the host
    /// reads and records as a
    /// [`LocalInputValue::AveragingSample`](crate::traits::LocalInputValue::AveragingSample).
    /// The due time is the one [`next_monotonic_deadline`] reports, so an
    /// adapter that forwards [`timed`](crate::traits::BACnetObject::timed)
    /// keeps the deadline and the sampling together. The default has
    /// nothing to sample. Not public API yet (#2179).
    ///
    /// [`next_monotonic_deadline`]: Self::next_monotonic_deadline
    #[doc(hidden)]
    fn take_due_sample(&mut self, _now: Duration) -> Option<BACnetObjectPropertyReference> {
        None
    }
}
