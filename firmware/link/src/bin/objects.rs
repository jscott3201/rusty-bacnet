//! The firmware link test's objects image (#2178).
//!
//! On a bare-metal target this is a `no_main` image with a global allocator:
//! `embedded-alloc`'s linked-list first fit over an arena of
//! [`ARENA`](firmware_link::objects::ARENA) octets in a static. Its reset
//! entry hands the allocator the arena, then runs
//! [`firmware_link::objects::run`] once on the hand-written requests. Those
//! and the summary pass through [`core::hint::black_box`], so none of the
//! work can be optimized away, and then it loops. CI never runs it: building
//! it proves that `bacnet-device-core` and `bacnet-objects` link `no_std`
//! with `alloc`, with no symbol only a hosted target provides, and
//! `scripts/ci/check-firmware-link.sh` checks its sizes. On a host the
//! binary is a stub; `tests/objects.rs` runs the scenario there.
//!
//! Both crates need atomic compare-and-swap, so the image builds for
//! thumbv7m, thumbv7em and riscv32imac, not for ARMv6-M or RV32IMC. It has
//! no emulation build yet.
//!
//! The image, not a library, chooses what only a firmware may: the
//! allocator, and the critical section it locks its heap with, as the apdu
//! image does (see `src/bin/apdu.rs`). The only `unsafe` code is the
//! arena's hand-over, the Cortex-M critical section and the code the
//! runtimes' entry macros generate.

#![cfg_attr(target_os = "none", no_std, no_main)]

/// The global allocator, on bare-metal targets; a host build uses std's.
#[cfg(target_os = "none")]
mod heap {
    use embedded_alloc::LlffHeap;

    #[global_allocator]
    static HEAP: LlffHeap = LlffHeap::empty();

    /// Hand the allocator its arena: a static of
    /// [`ARENA`](firmware_link::objects::ARENA) octets that `init!` declares.
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
            embedded_alloc::init!(HEAP, firmware_link::objects::ARENA);
        }
    }
}

/// Cortex-M (ARMv7-M), on cortex-m-rt.
#[cfg(all(target_os = "none", target_arch = "arm"))]
mod image {
    use core::hint::black_box;

    use firmware_link::objects::{APDUS, run};

    /// The reset entry.
    #[cortex_m_rt::entry]
    fn main() -> ! {
        super::heap::init();
        black_box(run(black_box(&APDUS)));
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

    use firmware_link::objects::{APDUS, run};

    /// Links riscv, and with it the single-hart critical section its
    /// `critical-section-single-hart` feature defines.
    use riscv as _;

    /// The reset entry.
    #[riscv_rt::entry]
    fn main() -> ! {
        super::heap::init();
        black_box(run(black_box(&APDUS)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// A host has nothing to flash: `tests/objects.rs` runs the scenario.
#[cfg(not(target_os = "none"))]
fn main() {}
