//! What the crate takes from std on a hosted build and from elsewhere
//! without it (#1615).
//!
//! The objects build without std on a target with a global allocator and
//! atomic compare-and-swap. Most of what they used from std lives in `core`
//! or `alloc` under the same name; the rest comes from here, so the object
//! code is the same in both builds:
//!
//! - [`prelude`]: the `alloc` names std's prelude adds (`String`, `Vec`,
//!   `Box`, `ToString`, `ToOwned`). Only modules that need them import it.
//! - [`collections`]: std's hash maps on a hosted build, hashbrown's
//!   without std. Neither's iteration order is fixed, so no output depends
//!   on it in either build.
//! - [`sync`]: a mutex (std's on a hosted build, a spin lock without std),
//!   and an `AtomicU64` that falls back to that mutex on a target without
//!   64-bit atomics.
//! - [`float`]: the `f64` rounding core lacks.
//! - [`catch_panic`]: std's `catch_unwind`, for calls into application
//!   code. Without std a panic isn't caught.
//! - `log_warn!` and `log_debug!`: tracing's `warn!` and `debug!` on a
//!   hosted build. Without std they log nothing, since tracing needs atomics
//!   that the no_std crates don't assume; they only borrow the variables the
//!   line names, so none goes unused.

/// The `alloc` names std's prelude brings into scope.
pub(crate) mod prelude {
    pub(crate) use alloc::borrow::ToOwned;
    pub(crate) use alloc::boxed::Box;
    pub(crate) use alloc::string::{String, ToString};
    pub(crate) use alloc::vec::Vec;
}

/// Hash maps and sets: std's with std, hashbrown's without.
pub(crate) mod collections {
    #[cfg(not(feature = "std"))]
    pub(crate) use hashbrown::{HashMap, HashSet};
    #[cfg(feature = "std")]
    pub(crate) use std::collections::{HashMap, HashSet};
}

/// A mutex whose [`lock`](sync::Mutex::lock) hands back the guard.
pub(crate) mod sync {
    /// A mutual-exclusion lock: std's with std, a spin lock without.
    ///
    /// A poisoned std mutex still hands back its guard. Every value these
    /// locks guard is whole between statements, so a panic elsewhere leaves
    /// nothing half written.
    #[derive(Debug, Default)]
    pub(crate) struct Mutex<T>(Inner<T>);

    #[cfg(feature = "std")]
    type Inner<T> = std::sync::Mutex<T>;
    #[cfg(not(feature = "std"))]
    type Inner<T> = spinning_top::Spinlock<T>;

    /// The guard [`Mutex::lock`] returns.
    #[cfg(feature = "std")]
    pub(crate) type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;
    /// The guard [`Mutex::lock`] returns.
    #[cfg(not(feature = "std"))]
    pub(crate) type MutexGuard<'a, T> = spinning_top::SpinlockGuard<'a, T>;

    impl<T> Mutex<T> {
        /// A mutex holding `value`.
        pub(crate) const fn new(value: T) -> Self {
            Self(Inner::new(value))
        }

        /// Wait for the lock and take it.
        pub(crate) fn lock(&self) -> MutexGuard<'_, T> {
            #[cfg(feature = "std")]
            {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            }
            #[cfg(not(feature = "std"))]
            {
                self.0.lock()
            }
        }
    }

    #[cfg(not(target_has_atomic = "64"))]
    pub(crate) use LockedU64 as AtomicU64;
    #[cfg(target_has_atomic = "64")]
    pub(crate) use core::sync::atomic::AtomicU64;

    /// The calls the crate makes on an `AtomicU64`, behind a [`Mutex`], for a
    /// target without 64-bit atomics, such as 32-bit Arm and RISC-V. The
    /// lock orders every call, so the orderings go unused. Tests build it on
    /// every target.
    #[cfg(any(test, not(target_has_atomic = "64")))]
    #[derive(Debug, Default)]
    pub(crate) struct LockedU64(Mutex<u64>);

    #[cfg(any(test, not(target_has_atomic = "64")))]
    impl LockedU64 {
        /// A counter holding `value`.
        pub(crate) const fn new(value: u64) -> Self {
            Self(Mutex::new(value))
        }

        /// The value.
        pub(crate) fn load(&self, _: core::sync::atomic::Ordering) -> u64 {
            *self.0.lock()
        }

        /// Replace the value.
        pub(crate) fn store(&self, value: u64, _: core::sync::atomic::Ordering) {
            *self.0.lock() = value;
        }

        /// Add `delta`, wrapping, and return the value before.
        pub(crate) fn fetch_add(&self, delta: u64, _: core::sync::atomic::Ordering) -> u64 {
            let mut value = self.0.lock();
            let before = *value;
            *value = before.wrapping_add(delta);
            before
        }

        /// Replace the value with what `f` makes of it, unless `f` returns
        /// `None`: the value before in `Ok`, or in `Err` when unchanged.
        pub(crate) fn try_update(
            &self,
            _: core::sync::atomic::Ordering,
            _: core::sync::atomic::Ordering,
            f: impl FnMut(u64) -> Option<u64>,
        ) -> Result<u64, u64> {
            let mut f = f;
            let mut value = self.0.lock();
            let before = *value;
            match f(before) {
                Some(next) => {
                    *value = next;
                    Ok(before)
                }
                None => Err(before),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

        use super::LockedU64;

        /// The fallback answers each call as core's `AtomicU64` does.
        #[test]
        fn locked_u64_matches_atomic_u64() {
            let (locked, atomic) = (LockedU64::new(u64::MAX - 1), AtomicU64::new(u64::MAX - 1));
            assert_eq!(locked.fetch_add(3, Relaxed), atomic.fetch_add(3, Relaxed));
            assert_eq!(locked.load(Relaxed), atomic.load(Relaxed));
            assert_eq!(locked.load(Relaxed), 1);
            let bump = |n: u64| n.checked_add(1);
            assert_eq!(
                locked.try_update(Relaxed, Relaxed, bump),
                atomic.try_update(Relaxed, Relaxed, bump)
            );
            locked.store(u64::MAX, Relaxed);
            atomic.store(u64::MAX, Relaxed);
            assert_eq!(
                locked.try_update(Relaxed, Relaxed, bump),
                atomic.try_update(Relaxed, Relaxed, bump)
            );
            assert_eq!(locked.try_update(Relaxed, Relaxed, bump), Err(u64::MAX));
            assert_eq!(locked.load(Relaxed), atomic.load(Relaxed));
        }
    }
}

/// The `f64` rounding std has and core lacks: std's with std, libm's
/// without. Each is exact in both, but for `hypot`, which may differ in the
/// last place.
pub(crate) mod float {
    /// `x` rounded toward zero.
    pub(crate) fn trunc(x: f64) -> f64 {
        #[cfg(feature = "std")]
        {
            x.trunc()
        }
        #[cfg(not(feature = "std"))]
        {
            libm::trunc(x)
        }
    }

    /// The smallest integer at or above `x`.
    pub(crate) fn ceil(x: f64) -> f64 {
        #[cfg(feature = "std")]
        {
            x.ceil()
        }
        #[cfg(not(feature = "std"))]
        {
            libm::ceil(x)
        }
    }

    /// `x` rounded to the nearest integer, halves away from zero.
    pub(crate) fn round(x: f64) -> f64 {
        #[cfg(feature = "std")]
        {
            x.round()
        }
        #[cfg(not(feature = "std"))]
        {
            libm::round(x)
        }
    }

    /// The length of the hypotenuse of a right triangle with legs `x` and
    /// `y`.
    pub(crate) fn hypot(x: f64, y: f64) -> f64 {
        #[cfg(feature = "std")]
        {
            x.hypot(y)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::hypot(x, y)
        }
    }
}

/// Run `f`, which calls application code: `None` if it panics. Only std
/// can catch a panic, so without it a panic goes on up.
pub(crate) fn catch_panic<R>(f: impl FnOnce() -> R) -> Option<R> {
    #[cfg(feature = "std")]
    {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
    }
    #[cfg(not(feature = "std"))]
    {
        Some(f())
    }
}

/// tracing's `warn!` with std; nothing without it.
#[cfg(feature = "std")]
macro_rules! log_warn {
    ($($arg:tt)*) => { ::tracing::warn!($($arg)*) };
}
/// tracing's `warn!` with std; nothing without it.
#[cfg(not(feature = "std"))]
macro_rules! log_warn {
    ($($arg:tt)*) => {{
        log_fields_used!($($arg)*);
    }};
}

/// Without std, borrows each variable a log line's fields name (`%error`,
/// `key = value`), so a binding only the log line reads still counts as
/// used. Every other token is passed over.
#[cfg(not(feature = "std"))]
macro_rules! log_fields_used {
    () => {};
    ($key:ident = % $value:ident $($rest:tt)*) => {
        let _ = &$value;
        log_fields_used!($($rest)*);
    };
    ($key:ident = ? $value:ident $($rest:tt)*) => {
        let _ = &$value;
        log_fields_used!($($rest)*);
    };
    ($key:ident = $value:ident $($rest:tt)*) => {
        let _ = &$value;
        log_fields_used!($($rest)*);
    };
    (% $value:ident $($rest:tt)*) => {
        let _ = &$value;
        log_fields_used!($($rest)*);
    };
    (? $value:ident $($rest:tt)*) => {
        let _ = &$value;
        log_fields_used!($($rest)*);
    };
    ($other:tt $($rest:tt)*) => {
        log_fields_used!($($rest)*);
    };
}

/// tracing's `debug!` with std; nothing without it.
#[cfg(feature = "std")]
macro_rules! log_debug {
    ($($arg:tt)*) => { ::tracing::debug!($($arg)*) };
}
/// tracing's `debug!` with std; nothing without it.
#[cfg(not(feature = "std"))]
macro_rules! log_debug {
    ($($arg:tt)*) => {{
        log_fields_used!($($arg)*);
    }};
}
