//! [`Duration`]: a length of time in nanoseconds.

use core::ops::{Add, AddAssign, Sub, SubAssign};

use crate::divide::div_rem;
use crate::{NANOS_PER_MICRO, NANOS_PER_MILLI, NANOS_PER_SEC};

/// A length of time, in nanoseconds on a `u64`: at most about 584 years.
///
/// `+` and `-` saturate at [`Duration::MAX`] and at zero. The default is
/// [`Duration::ZERO`].
///
/// The `try_from_*` constructors return `None` where the others saturate:
/// the `try_` prefix on an `Option`, as chrono's `TimeDelta::try_seconds`
/// has it.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(u64);

impl Duration {
    /// No time.
    pub const ZERO: Self = Self(0);
    /// The longest representable duration, `u64::MAX` nanoseconds: about
    /// 584 years.
    pub const MAX: Self = Self(u64::MAX);

    /// `nanos` nanoseconds.
    #[must_use]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// `micros` microseconds, saturating at [`Duration::MAX`].
    #[must_use]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros.saturating_mul(NANOS_PER_MICRO))
    }

    /// `millis` milliseconds, saturating at [`Duration::MAX`].
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis.saturating_mul(NANOS_PER_MILLI))
    }

    /// `secs` seconds, saturating at [`Duration::MAX`].
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(NANOS_PER_SEC))
    }

    /// `micros` microseconds, or `None` past [`Duration::MAX`].
    #[must_use]
    pub const fn try_from_micros(micros: u64) -> Option<Self> {
        match micros.checked_mul(NANOS_PER_MICRO) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `millis` milliseconds, or `None` past [`Duration::MAX`].
    #[must_use]
    pub const fn try_from_millis(millis: u64) -> Option<Self> {
        match millis.checked_mul(NANOS_PER_MILLI) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `secs` seconds, or `None` past [`Duration::MAX`].
    #[must_use]
    pub const fn try_from_secs(secs: u64) -> Option<Self> {
        match secs.checked_mul(NANOS_PER_SEC) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// The length in nanoseconds.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Whether the length is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// The whole microseconds in the length, rounded toward zero.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        div_rem(self.0, NANOS_PER_MICRO).0
    }

    /// The whole milliseconds in the length, rounded toward zero.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        div_rem(self.0, NANOS_PER_MILLI).0
    }

    /// The whole seconds in the length, rounded toward zero.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        div_rem(self.0, NANOS_PER_SEC).0
    }

    /// `self + other`, or `None` past [`Duration::MAX`].
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self + other`, clamped at [`Duration::MAX`].
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// `self - other`, or `None` below zero.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self - other`, clamped at zero.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// `self` times `factor`, or `None` past [`Duration::MAX`].
    #[must_use]
    pub const fn checked_mul(self, factor: u32) -> Option<Self> {
        match self.0.checked_mul(factor as u64) {
            Some(nanos) => Some(Self(nanos)),
            None => None,
        }
    }

    /// `self` times `factor`, clamped at [`Duration::MAX`].
    #[must_use]
    pub const fn saturating_mul(self, factor: u32) -> Self {
        Self(self.0.saturating_mul(factor as u64))
    }

    /// The shorter of `self` and `other`: [`Ord::min`] for a `const`
    /// context.
    #[must_use]
    pub const fn min(self, other: Self) -> Self {
        if self.0 <= other.0 { self } else { other }
    }

    /// The longer of `self` and `other`: [`Ord::max`] for a `const`
    /// context.
    #[must_use]
    pub const fn max(self, other: Self) -> Self {
        if self.0 >= other.0 { self } else { other }
    }

    /// What is left of `self` after taking out as many whole `divisor`s as
    /// fit, or `None` for a zero `divisor`: the offset into a repeating
    /// cycle, say.
    ///
    /// It takes one step per bit of the quotient, without a 64-bit division
    /// routine (see the [crate docs](crate#no-64-bit-division)).
    #[must_use]
    pub const fn checked_rem(self, divisor: Self) -> Option<Self> {
        if divisor.0 == 0 {
            None
        } else {
            Some(Self(div_rem(self.0, divisor.0).1))
        }
    }
}

impl Add for Duration {
    type Output = Self;

    /// Saturating, as [`Duration::saturating_add`].
    fn add(self, rhs: Self) -> Self {
        self.saturating_add(rhs)
    }
}

impl AddAssign for Duration {
    /// Saturating, as [`Duration::saturating_add`].
    fn add_assign(&mut self, rhs: Self) {
        *self = self.saturating_add(rhs);
    }
}

impl Sub for Duration {
    type Output = Self;

    /// Saturating, as [`Duration::saturating_sub`].
    fn sub(self, rhs: Self) -> Self {
        self.saturating_sub(rhs)
    }
}

impl SubAssign for Duration {
    /// Saturating, as [`Duration::saturating_sub`].
    fn sub_assign(&mut self, rhs: Self) {
        *self = self.saturating_sub(rhs);
    }
}
