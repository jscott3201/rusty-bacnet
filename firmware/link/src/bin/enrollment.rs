//! The firmware link test's enrollment image (#2319).
//!
//! The reporting image (`src/bin/reporting.rs`) with `bacnet-device-core`'s
//! Event Enrollment pass on: on a bare-metal target, a `no_main` image
//! whose reset entry hands its allocator an arena of
//! [`ARENA`](firmware_link::enrollment::ARENA) octets, then runs
//! [`firmware_link::enrollment::run`] once: the reporting image's scenario,
//! then a device whose Event Enrollment reports a transition, its
//! acknowledgement, its return and a fault through the `Reporter`. The
//! requests and the summary pass through [`core::hint::black_box`], so none
//! of the work can be optimized away, and then it loops. CI never runs it:
//! building it proves that the pass links `no_std` with `alloc`, and
//! `scripts/ci/check-firmware-link.sh` checks its sizes. On a host the
//! binary is a stub; `tests/enrollment.rs` runs the scenario there.
//!
//! It builds for the reporting image's targets, thumbv7m, thumbv7em and
//! riscv32imac, in its 512 KiB layout, and has no emulation build yet. The
//! allocator, the critical section and the only `unsafe` code are the
//! objects image's.

#![cfg_attr(target_os = "none", no_std, no_main)]

/// The global allocator, on bare-metal targets; a host build uses std's.
#[cfg(target_os = "none")]
mod heap {
    use embedded_alloc::LlffHeap;

    #[global_allocator]
    static HEAP: LlffHeap = LlffHeap::empty();

    /// Hand the allocator its arena: a static of
    /// [`ARENA`](firmware_link::enrollment::ARENA) octets that `init!` declares.
    /// Call it once, before anything allocates.
    pub fn init() {
        // SAFETY: `init!` declares the arena as a static that nothing else
        // names, so the allocator owns those octets for the rest of the
        // program, and the size it passes is the static's. The allocator
        // must be initialized once, before its first allocation: the reset
        // entry calls this first, before the scenario, and runs once and
        // never returns.
        #[allow(unsafe_code)]
        unsafe {
            embedded_alloc::init!(HEAP, firmware_link::enrollment::ARENA);
        }
    }
}

/// Cortex-M (ARMv7-M), on cortex-m-rt.
#[cfg(all(target_os = "none", target_arch = "arm"))]
mod image {
    use core::hint::black_box;

    use firmware_link::enrollment::run;
    use firmware_link::objects::APDUS;
    use firmware_link::reporting::REQUESTS;

    /// The reset entry.
    #[cortex_m_rt::entry]
    fn main() -> ! {
        super::heap::init();
        black_box(run(black_box(&APDUS), black_box(&REQUESTS)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// The critical section on Cortex-M, which the allocator locks with: the
/// apdu image's, which masks interrupts with PRIMASK and puts back the mask
/// it found. See `src/bin/apdu.rs` for its terms.
#[cfg(all(target_os = "none", target_arch = "arm"))]
#[allow(unsafe_code)]
#[path = "support/single_core.rs"]
mod single_core;

/// RISC-V (RV32IMAC), on riscv-rt.
#[cfg(all(target_os = "none", target_arch = "riscv32"))]
mod image {
    use core::hint::black_box;

    use firmware_link::enrollment::run;
    use firmware_link::objects::APDUS;
    use firmware_link::reporting::REQUESTS;

    /// Links riscv, and with it the single-hart critical section its
    /// `critical-section-single-hart` feature defines.
    use riscv as _;

    /// The reset entry.
    #[riscv_rt::entry]
    fn main() -> ! {
        super::heap::init();
        black_box(run(black_box(&APDUS), black_box(&REQUESTS)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// A host has nothing to flash: `tests/enrollment.rs` runs the scenario.
#[cfg(not(target_os = "none"))]
fn main() {}
