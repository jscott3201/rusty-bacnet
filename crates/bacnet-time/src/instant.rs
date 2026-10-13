//! [`Instant`]: a point on the caller's monotonic timeline.

use core::ops::{Add, AddAssign, Sub, SubAssign};

use crate::{Duration, NANOS_PER_MICRO, NANOS_PER_MILLI, NANOS_PER_SEC};

/// A point on the caller's monotonic timeline, in nanoseconds since an
/// epoch the caller chooses.
///
/// `+` and `-` with a [`Duration`] saturate at [`Instant::MAX`] and at the
/// epoch, and the time from a later instant to an earlier one, `earlier -
/// later`, is [`Duration::ZERO`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

impl Instant {
    /// The epoch.
    pub const ZERO: Self = Self(0);
    /// The latest representable instant, about 584 years after the epoch.
    pub const MAX: Self = Self(u64::MAX);

    /// The instant `nanos` nanoseconds after the epoch.
    #[must_use]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// The instant `micros` microseconds after the epoch, saturating at
    /// [`Instant::MAX`].
    #[must_use]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros.saturating_mul(NANOS_PER_MICRO))
    }

    /// The instant `millis` milliseconds after the epoch, saturating at
    /// [`Instant::MAX`].
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis.saturating_mul(NANOS_PER_MILLI))
    }

    /// The instant `secs` seconds after the epoch, saturating at
    /// [`Instant::MAX`].
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(NANOS_PER_SEC))
    }

    /// Nanoseconds since the epoch.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Whole microseconds since the epoch, rounded toward zero.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.since_epoch().as_micros()
    }

    /// Whole milliseconds since the epoch, rounded toward zero.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.since_epoch().as_millis()
    }

    /// Whole seconds since the epoch, rounded toward zero.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.since_epoch().as_secs()
    }

    /// `self + duration`, or `None` past [`Instant::MAX`].
    #[must_use]
    pub const fn checked_add(self, duration: Duration) -> Option<Self> {
        match self.0.checked_add(duration.as_nanos()) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self + duration`, clamped at [`Instant::MAX`].
    #[must_use]
    pub const fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.as_nanos()))
    }

    /// `self - duration`, or `None` before the epoch.
    #[must_use]
    pub const fn checked_sub(self, duration: Duration) -> Option<Self> {
        match self.0.checked_sub(duration.as_nanos()) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self - duration`, clamped at the epoch.
    #[must_use]
    pub const fn saturating_sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(duration.as_nanos()))
    }

    /// The time from `earlier` to `self`, or `None` when `earlier` is later.
    #[must_use]
    pub const fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        match self.0.checked_sub(earlier.0) {
            Some(nanos) => Some(Duration::from_nanos(nanos)),
            None => None,
        }
    }

    /// The time from `earlier` to `self`, or zero when `earlier` is later.
    #[must_use]
    pub const fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }

    /// The earlier of `self` and `other`: [`Ord::min`] for a `const`
    /// context.
    #[must_use]
    pub const fn min(self, other: Self) -> Self {
        if self.0 <= other.0 {
            self
        } else {
            other
        }
    }

    /// The later of `self` and `other`: [`Ord::max`] for a `const` context.
    #[must_use]
    pub const fn max(self, other: Self) -> Self {
        if self.0 >= other.0 {
            self
        } else {
            other
        }
    }

    /// The time from the epoch to `self`.
    const fn since_epoch(self) -> Duration {
        Duration::from_nanos(self.0)
    }
}

impl Add<Duration> for Instant {
    type Output = Self;

    /// Saturating, as [`Instant::saturating_add`].
    fn add(self, rhs: Duration) -> Self {
        self.saturating_add(rhs)
    }
}

impl AddAssign<Duration> for Instant {
    /// Saturating, as [`Instant::saturating_add`].
    fn add_assign(&mut self, rhs: Duration) {
        *self = self.saturating_add(rhs);
    }
}

impl Sub<Duration> for Instant {
    type Output = Self;

    /// Saturating, as [`Instant::saturating_sub`].
    fn sub(self, rhs: Duration) -> Self {
        self.saturating_sub(rhs)
    }
}

impl SubAssign<Duration> for Instant {
    /// Saturating, as [`Instant::saturating_sub`].
    fn sub_assign(&mut self, rhs: Duration) {
        *self = self.saturating_sub(rhs);
    }
}

impl Sub for Instant {
    type Output = Duration;

    /// Saturating, as [`Instant::saturating_duration_since`].
    fn sub(self, rhs: Self) -> Duration {
        self.saturating_duration_since(rhs)
    }
}
