//! The time the portable BACnet cores share: a monotonic [`Instant`] and a
//! [`Duration`], both counted in nanoseconds on a `u64`.
//!
//! The cores read no clock. Each call that depends on time takes `now`, an
//! [`Instant`] on whatever monotonic timeline the caller keeps, counted from
//! an epoch the caller chooses. A `u64` of nanoseconds lasts about 584 years
//! from that epoch. One type for every core lets firmware that runs several
//! of them, an MS/TP port and a B/IP port say, feed them all one clock.
//!
//! - A host maps its monotonic clock onto the timeline exactly: nanoseconds
//!   are the finest unit `core::time::Duration` has. A [`Duration`] converts
//!   into one with [`From`], and one converts back with [`TryFrom`], which
//!   refuses a length past [`Duration::MAX`] ([`DurationTooLong`]), or with
//!   [`Duration::saturating_from_core`], which clamps it.
//! - Firmware with a millisecond or microsecond tick builds its `now` with
//!   [`Instant::from_millis`] or [`Instant::from_micros`].
//! - With the `std` feature, `HostClock` does the mapping for a host: it
//!   fixes one `std::time::Instant` to a point of the timeline and converts
//!   both ways, exactly.
//!
//! A core with several timers hands back its next deadline as an
//! `Option<Instant>`, `None` for none. [`earliest`] merges such deadlines;
//! `Option::min` would not, since it orders `None` first.
//!
//! # Arithmetic
//!
//! Nothing panics. The `+` and `-` operators saturate, at [`Instant::MAX`]
//! or [`Duration::MAX`] and at zero, and an earlier instant less a later one
//! is [`Duration::ZERO`]. The `checked_*` methods return `None` instead, for
//! a caller that must tell.
//!
//! The unit constructors (`from_micros`, `from_millis`, `from_secs`)
//! multiply, so they are exact or saturate. The unit accessors (`as_micros`,
//! `as_millis`, `as_secs`) round toward zero. `min` and `max` are `const`.
//!
//! `Debug` prints nanoseconds with the unit: `Duration(1500ns)`,
//! `Instant(1500ns)`.
//!
//! # No 64-bit division
//!
//! A 32-bit target has no instruction that divides a 64-bit value, so a
//! division there links a routine from compiler-builtins, close to 1 KB,
//! which the firmware images of rusty-bacnet don't allow. The unit accessors,
//! [`Duration::checked_rem`], the conversion into `core::time::Duration` and
//! `Debug` divide by binary long division instead: shifts, compares and
//! subtractions, one step per bit of the quotient. Nothing else divides.
//!
//! # Features
//!
//! No dependencies, and no features by default: the crate is `no_std` and
//! needs no allocator.
//!
//! - `std`: `HostClock`, which maps a host's `std::time::Instant` onto the
//!   timeline and back.
//!
//! # Example
//!
//! ```
//! use bacnet_time::{Duration, Instant};
//!
//! // A deadline 500 ms after a millisecond tick of 1250.
//! let now = Instant::from_millis(1_250);
//! let deadline = now + Duration::from_millis(500);
//! assert_eq!(deadline.saturating_duration_since(now), Duration::from_millis(500));
//!
//! // Arithmetic saturates; the checked forms say when it would have.
//! assert_eq!(now - deadline, Duration::ZERO);
//! assert_eq!(Instant::ZERO.checked_sub(Duration::from_secs(1)), None);
//!
//! // A host's clock, through core::time::Duration.
//! let elapsed = core::time::Duration::from_micros(2_500);
//! assert_eq!(Duration::try_from(elapsed), Ok(Duration::from_micros(2_500)));
//! ```

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, clippy::must_use_candidate)]

#[cfg(feature = "std")]
extern crate std;

mod convert;
mod deadline;
mod divide;
mod duration;
mod format;
#[cfg(feature = "std")]
mod host;
mod instant;

pub use convert::DurationTooLong;
pub use deadline::earliest;
pub use duration::Duration;
#[cfg(feature = "std")]
pub use host::HostClock;
pub use instant::Instant;

/// Nanoseconds in a microsecond.
const NANOS_PER_MICRO: u64 = 1_000;
/// Nanoseconds in a millisecond.
const NANOS_PER_MILLI: u64 = 1_000_000;
/// Nanoseconds in a second.
const NANOS_PER_SEC: u64 = 1_000_000_000;

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
