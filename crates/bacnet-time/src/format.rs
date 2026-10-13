//! `Debug` for [`Duration`] and [`Instant`]: nanoseconds, with the unit, as
//! `Duration(1500ns)`.

use core::fmt;

use crate::divide::div_rem;
use crate::{Duration, Instant, NANOS_PER_SEC};

impl fmt::Debug for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_nanos(f, "Duration", self.as_nanos())
    }
}

impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_nanos(f, "Instant", self.as_nanos())
    }
}

/// Writes `name(<nanos>ns)`.
///
/// The count goes out as up to three parts below 10^9, each a `u32`, split
/// off by [`div_rem`]: formatting a `u64` directly divides it by powers of
/// ten, which on a 32-bit target links a 64-bit division routine.
fn write_nanos(f: &mut fmt::Formatter<'_>, name: &str, nanos: u64) -> fmt::Result {
    let (upper, low) = div_rem(nanos, NANOS_PER_SEC);
    let (high, middle) = div_rem(upper, NANOS_PER_SEC);
    // u64::MAX is under 19 * 10^18, so `high` is at most 18, and the other
    // two are remainders below 10^9: each fits a u32.
    let (high, middle, low) = (high as u32, middle as u32, low as u32);
    if high != 0 {
        write!(f, "{name}({high}{middle:09}{low:09}ns)")
    } else if middle != 0 {
        write!(f, "{name}({middle}{low:09}ns)")
    } else {
        write!(f, "{name}({low}ns)")
    }
}
