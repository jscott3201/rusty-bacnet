//! Conversions to and from `core::time::Duration`.

use core::fmt;

use crate::divide::div_rem;
use crate::{Duration, NANOS_PER_SEC};

/// A `core::time::Duration` too long for a [`Duration`]: more than
/// `u64::MAX` nanoseconds, about 584 years. What
/// `Duration::try_from(core::time::Duration)` returns for one;
/// [`Duration::saturating_from_core`] clamps it instead.
///
/// Only the conversion builds one, as with the standard library's
/// `TryFromIntError`, so the type can carry more detail later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DurationTooLong(());

impl fmt::Display for DurationTooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("duration longer than u64::MAX nanoseconds (about 584 years)")
    }
}

impl core::error::Error for DurationTooLong {}

impl Duration {
    /// `duration` in nanoseconds, or `None` past [`Duration::MAX`].
    const fn checked_from_core(duration: core::time::Duration) -> Option<Self> {
        match duration.as_secs().checked_mul(NANOS_PER_SEC) {
            Some(whole) => match whole.checked_add(duration.subsec_nanos() as u64) {
                Some(nanos) => Some(Self::from_nanos(nanos)),
                None => None,
            },
            None => None,
        }
    }

    /// `duration`, clamped at [`Duration::MAX`], about 584 years: for a
    /// host mapping its own clock onto the timeline, where a time that far
    /// out means never. A length that must fit, from configuration say,
    /// goes through `Duration::try_from` instead, which refuses one too long.
    #[must_use]
    pub const fn saturating_from_core(duration: core::time::Duration) -> Self {
        match Self::checked_from_core(duration) {
            Some(duration) => duration,
            None => Self::MAX,
        }
    }
}

impl TryFrom<core::time::Duration> for Duration {
    type Error = DurationTooLong;

    /// The same length, or [`DurationTooLong`] for one past
    /// [`Duration::MAX`]: a `core::time::Duration` reaches about 584
    /// billion years.
    fn try_from(duration: core::time::Duration) -> Result<Self, DurationTooLong> {
        Self::checked_from_core(duration).ok_or(DurationTooLong(()))
    }
}

impl From<Duration> for core::time::Duration {
    /// The same length, exactly. Splitting it into seconds and nanoseconds
    /// takes no 64-bit division routine (see the
    /// [crate docs](crate#no-64-bit-division)).
    fn from(duration: Duration) -> Self {
        let (secs, nanos) = div_rem(duration.as_nanos(), NANOS_PER_SEC);
        // The remainder is under one second, so it fits a u32.
        Self::new(secs, nanos as u32)
    }
}
