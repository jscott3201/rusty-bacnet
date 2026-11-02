//! The counting harness's shared parts: a counting allocator that each
//! binary installs as its global allocator, and the B/IP loopback setup the
//! round-trip workloads use.
//!
//! Each binary runs one workload a fixed number of times between two
//! untimed value oracles and prints one `RESULT` line to stderr with the
//! allocator calls and requested bytes of the counted loop. The instruction
//! counts come from running the binary under callgrind at two loop lengths
//! (`scripts/perf/perf_counts.py`); their difference divided by the
//! difference in length is the cost of one iteration, free of setup,
//! oracles and shutdown.
#![deny(missing_docs)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub mod setup;

static CALLS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

/// A global allocator that counts allocation calls (`alloc`, `alloc_zeroed`
/// and `realloc`) and the bytes they request, then defers to the system
/// allocator. Frees aren't counted. The counters are process-wide, so they
/// include every thread's allocations.
pub struct CountingAlloc;

// SAFETY: every method defers to `System` with the caller's arguments
// unchanged; the counters are plain atomics.
//
// `#[inline(never)]` keeps each method a function of its own under fat LTO,
// so scripts/perf can find and subtract the counting cost by name; inlined
// into the allocator shims, it would vanish into them (#2314).
unsafe impl GlobalAlloc for CountingAlloc {
    #[inline(never)]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        CALLS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    #[inline(never)]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        CALLS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    #[inline(never)]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller upholds `GlobalAlloc::dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }

    #[inline(never)]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        CALLS.fetch_add(1, Relaxed);
        BYTES.fetch_add(new_size as u64, Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Allocation calls and requested bytes so far, process-wide.
#[derive(Clone, Copy, Debug)]
pub struct AllocSnapshot {
    /// Allocation calls.
    pub calls: u64,
    /// Bytes those calls requested.
    pub bytes: u64,
}

impl AllocSnapshot {
    /// The counters now.
    pub fn now() -> Self {
        Self {
            calls: CALLS.load(Relaxed),
            bytes: BYTES.load(Relaxed),
        }
    }

    /// The counts since `earlier`.
    pub fn since(self, earlier: Self) -> Self {
        Self {
            calls: self.calls - earlier.calls,
            bytes: self.bytes - earlier.bytes,
        }
    }
}

/// Number of untimed iterations run before the counted loop.
pub const WARMUP: u64 = 200;

/// Prints the line `scripts/perf/perf_counts.py` reads: the workload, the
/// loop length and the counted loop's allocator totals.
pub fn report(workload: &str, runtime: &str, n: u64, counted: AllocSnapshot) {
    eprintln!(
        "RESULT workload={workload} runtime={runtime} n={n} warmup={WARMUP} allocs={} bytes={} \
         allocs_per_iter={:.6} bytes_per_iter={:.6}",
        counted.calls,
        counted.bytes,
        counted.calls as f64 / n as f64,
        counted.bytes as f64 / n as f64,
    );
}

/// Reads a loop length argument; exits with a usage message if it is
/// missing or not a positive number.
pub fn parse_n(arg: Option<String>, usage: &str) -> u64 {
    match arg.as_deref().map(str::parse::<u64>) {
        Some(Ok(n)) if n > 0 => n,
        _ => usage_exit(usage),
    }
}

/// Prints `usage` and exits with status 2.
pub fn usage_exit(usage: &str) -> ! {
    eprintln!("usage: {usage}");
    std::process::exit(2)
}
