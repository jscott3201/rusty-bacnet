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
//! without `getrandom` does (`ENOSYS`), and its callers read `/dev/urandom`
//! instead: everything stays random, as in production. Only these binaries
//! define the symbol; `bacnet-counts` isn't published and nothing depends on
//! it, so no library, CLI or wheel of the stack carries it.
//!
//! The definition stands in for the C library's in the whole process, and
//! the linker exports it, so a run-time lookup by name finds it too. Seeded,
//! these get the same bytes in every run: `std`'s hash-map keys (every
//! `HashMap` and `HashSet`, and the server's digest of oversized requests);
//! Tokio's random number generator, which seeds from a `RandomState`, so
//! its random choices (which `select!` branch it polls first, which worker
//! it steals from) repeat, though the multi-thread runtime's interleaving
//! still doesn't; and the `getrandom` crate, which finds this function by
//! name: the stack's random B/IPv6 and SC addresses, SC reconnect jitter and
//! network-number answer delays, none of which the counted B/IP round trips
//! reach. The client's invoke IDs count up and draw nothing. The harness
//! links no TLS or `rand` crate.
//!
//! The lookup is a `std` internal, so a toolchain could stop using it. Two
//! checks then stop the count rather than let it run on random keys: a run
//! whose `getrandom` `std` never called reports `hash_keys=random`, and
//! `scripts/perf/perf_counts.py` refuses a seeded run that does; a run that
//! drew its keys some other way reports a `hash_check` (one hash of a fixed
//! value under the main thread's first keys) that differs from the other
//! runs', and the script refuses that too.

use std::hash::{BuildHasher, RandomState};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// The environment variable that fixes the hash-map keys: a decimal `u64`
/// seed. `scripts/perf/perf_counts.py` sets it for every counted run.
pub const HASH_SEED_VAR: &str = "BACNET_COUNTS_HASH_SEED";

static SEED: AtomicU64 = AtomicU64::new(0);
static SEEDED: AtomicBool = AtomicBool::new(false);
/// Requests for bytes answered from the seed (the `getrandom` crate's
/// zero-length probe doesn't count): none means `std` never asked this
/// binary's `getrandom`, so the keys weren't fixed.
static SERVED: AtomicU64 = AtomicU64::new(0);
/// The main thread's first keys, as one hash of [`CHECK_INPUT`].
static CHECK: AtomicU64 = AtomicU64::new(0);
const CHECK_INPUT: u64 = 2591;

/// Reads [`HASH_SEED_VAR`]. Call it first in `main`, before anything builds
/// a hash map: a thread's keys are drawn once. Exits with status 2 when the
/// variable holds something other than a decimal `u64`. Where `std` doesn't
/// draw its keys from `getrandom` (not Linux), it says the seed is ignored,
/// and the keys stay random.
pub fn fix_from_env() {
    let Some(value) = std::env::var_os(HASH_SEED_VAR) else {
        return;
    };
    match value.to_str().map(str::parse::<u64>) {
        Some(Ok(seed)) if HOOKED => {
            SEED.store(seed, Relaxed);
            SEEDED.store(true, Relaxed);
            // Draws this thread's keys now, so `report_fields` can tell
            // whether `std` asked this binary's `getrandom`, even in a
            // binary that builds no map, and what keys it got. The same
            // step in every build.
            CHECK.store(RandomState::new().hash_one(CHECK_INPUT), Relaxed);
        }
        Some(Ok(_)) => eprintln!(
            "{HASH_SEED_VAR} is ignored on this target: its std doesn't draw hash-map keys from getrandom, \
             so they stay random"
        ),
        _ => {
            eprintln!("{HASH_SEED_VAR} must be a decimal u64, not {value:?}");
            std::process::exit(2)
        }
    }
}

/// The `RESULT` line's fields for the keys: `hash_keys=fixed` and the
/// `hash_check` when they came from the seed, `hash_keys=random` otherwise.
pub fn report_fields() -> String {
    if SEEDED.load(Relaxed) && SERVED.load(Relaxed) > 0 {
        format!("hash_keys=fixed hash_check={:016x}", CHECK.load(Relaxed))
    } else {
        "hash_keys=random".to_owned()
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
        if len > 0 {
            SERVED.fetch_add(1, Relaxed);
        }
        // A request is at most `isize::MAX` bytes, as for `read`.
        len as isize
    }
}
