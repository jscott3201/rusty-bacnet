//! Fixed hash-map keys for a counted run (#2591).
//!
//! `std`'s `RandomState` takes each thread's keys from the C library's
//! `getrandom` the first time that thread builds a map, then steps them by
//! one per map. The server's `ObjectDatabase` keeps its objects in such a
//! map, so how many slots a lookup probes, and so the instructions a
//! ReadPropertyMultiple or WriteProperty costs, differed from one process to
//! the next: about one process in twelve drew keys under which a looked-up
//! identifier shares its probe group with another, and paid 30 to 290
//! instructions per request more.
//!
//! `std` reaches `getrandom` through a weak symbol so that a measurement can
//! put its own in place, and this harness's binaries define one. With
//! [`HASH_SEED_VAR`] set, it fills every request with the same bytes, drawn
//! from the seed, so every thread starts from the same keys in every run and
//! two builds of one commit probe alike. Without it, it answers as a kernel
//! without `getrandom` does, and `std` reads `/dev/urandom` instead: the keys
//! are random, as in production. Only these binaries define the symbol: no
//! library of the stack changes, and production builds keep `std`'s random
//! keys. Whatever else `std` seeds from those keys is fixed too, such as
//! Tokio's random number generator, which seeds from a `RandomState`. The
//! `getrandom` crate (the stack's random addresses and delays) looks the C
//! function up by name at run time, which finds the C library's own, so its
//! bytes stay random.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// The environment variable that fixes the hash-map keys: a decimal `u64`
/// seed. `scripts/perf/perf_counts.py` sets it for every counted run.
pub const HASH_SEED_VAR: &str = "BACNET_COUNTS_HASH_SEED";

static SEED: AtomicU64 = AtomicU64::new(0);
static SEEDED: AtomicBool = AtomicBool::new(false);
/// Requests answered from the seed: none means `std` never asked this
/// binary's `getrandom`, so the keys weren't fixed.
static SERVED: AtomicU64 = AtomicU64::new(0);

/// Reads [`HASH_SEED_VAR`]. Call it first in `main`, before anything builds
/// a hash map: a thread's keys are drawn once. Exits with status 2 when the
/// variable holds something other than a decimal `u64`.
pub fn fix_from_env() {
    let Some(value) = std::env::var_os(HASH_SEED_VAR) else {
        return;
    };
    match value.to_str().map(str::parse::<u64>) {
        Some(Ok(seed)) if HOOKED => {
            SEED.store(seed, Relaxed);
            SEEDED.store(true, Relaxed);
            // Draws this thread's keys now, so `state` can tell whether
            // `std` asked this binary's `getrandom`, even in a binary that
            // builds no map. The same step in every build.
            drop(std::hash::RandomState::new());
        }
        Some(Ok(_)) => {}
        _ => {
            eprintln!("{HASH_SEED_VAR} must be a decimal u64, not {value:?}");
            std::process::exit(2)
        }
    }
}

/// `fixed` when the keys came from the seed, `random` otherwise: the value
/// of the `RESULT` line's `hash_keys` field.
pub fn state() -> &'static str {
    if SEEDED.load(Relaxed) && SERVED.load(Relaxed) > 0 {
        "fixed"
    } else {
        "random"
    }
}

/// Whether this target's `std` takes its keys from `getrandom`: Linux with
/// glibc or musl, the counting image's target.
const HOOKED: bool = cfg!(all(
    target_os = "linux",
    any(target_env = "gnu", target_env = "musl")
));

#[cfg(all(target_os = "linux", any(target_env = "gnu", target_env = "musl")))]
mod hook {
    use super::{SEED, SEEDED, SERVED};
    use core::ffi::{c_int, c_uint, c_void};
    use std::sync::atomic::Ordering::Relaxed;

    unsafe extern "C" {
        fn __errno_location() -> *mut c_int;
    }

    /// Linux's "function not implemented", which makes `std` fall back to
    /// `/dev/urandom`.
    const ENOSYS: c_int = 38;

    /// The C library's `getrandom(buf, len, flags)`, in its place for this
    /// process. Seeded, it writes `len` bytes of a SplitMix64 stream that
    /// starts again from the seed on every call, so the bytes don't depend
    /// on which thread asks first; `flags` doesn't matter, as nothing here
    /// waits for entropy.
    ///
    /// # Safety
    ///
    /// `buf` must be valid for writes of `len` bytes, as the C function
    /// requires.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn getrandom(buf: *mut c_void, len: usize, _flags: c_uint) -> isize {
        if !SEEDED.load(Relaxed) {
            // SAFETY: the C library's thread-local errno.
            unsafe { *__errno_location() = ENOSYS };
            return -1;
        }
        let out = buf.cast::<u8>();
        let mut state = SEED.load(Relaxed);
        let mut written = 0;
        while written < len {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            for byte in (z ^ (z >> 31)).to_le_bytes() {
                if written == len {
                    break;
                }
                // SAFETY: `written < len`, and the caller gave `len` bytes.
                unsafe { out.add(written).write(byte) };
                written += 1;
            }
        }
        SERVED.fetch_add(1, Relaxed);
        // A request is at most `isize::MAX` bytes, as for `read`.
        len as isize
    }
}
