//! The firmware link test's apdu image (#1635).
//!
//! On a bare-metal target this is a `no_main` image with a global allocator:
//! `embedded-alloc`'s linked-list first fit over an arena of
//! [`ARENA`](firmware_link::apdu::ARENA) octets in a static. Its reset entry
//! hands the allocator the arena, then runs [`firmware_link::apdu::run`] once
//! on the hand-written datagrams. Those and the summary pass through
//! [`core::hint::black_box`], so none of the work can be optimized away, and
//! then it loops. CI never runs it: building it proves that
//! `bacnet-encoding` and `bacnet-services` link `no_std` with `alloc`, with
//! no symbol only a hosted target provides, and
//! `scripts/ci/check-firmware-link.sh` checks its sizes. On a host the
//! binary is a stub; `tests/apdu.rs` runs the scenario there.
//!
//! The emulation build (the `emulate` feature) swaps in the reset entry of
//! `emulated`, which runs under QEMU by hand: it runs the same scenario, and
//! the `emulate` module checks the summary against
//! `firmware_link::apdu::expected()` and reports through semihosting.
//!
//! The image, not a library, chooses what only a firmware may:
//!
//! - **The allocator**, which locks its heap with a critical section.
//! - **The critical section**, for a part with one core: on Cortex-M the one
//!   below, which masks interrupts; on RISC-V `riscv`'s single-hart one,
//!   which does the same.
//! - **The atomics backend** where the target has no compare-and-swap
//!   (ARMv6-M, RV32IMC): `bytes` takes its atomics from `portable-atomic`,
//!   which takes the same critical section (see the crate's manifest).
//!
//! The only `unsafe` code is what the allocator, the critical section and
//! the runtimes need: the arena's hand-over, the Cortex-M critical section,
//! and the code the runtimes' entry macros generate; and in the emulation
//! build, the `emulate` module's.

#![cfg_attr(target_os = "none", no_std, no_main)]

/// The global allocator, on bare-metal targets; a host build uses std's.
#[cfg(target_os = "none")]
mod heap {
    use embedded_alloc::LlffHeap;

    #[global_allocator]
    static HEAP: LlffHeap = LlffHeap::empty();

    /// Hand the allocator its arena: a static of
    /// [`ARENA`](firmware_link::apdu::ARENA) octets that `init!` declares.
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
            embedded_alloc::init!(HEAP, firmware_link::apdu::ARENA);
        }
    }
}

/// Cortex-M (ARMv6-M and ARMv7-M), on cortex-m-rt.
#[cfg(all(target_os = "none", target_arch = "arm", not(feature = "emulate")))]
mod image {
    use core::hint::black_box;

    use firmware_link::apdu::{run, DATAGRAMS};

    /// The reset entry.
    #[cortex_m_rt::entry]
    fn main() -> ! {
        super::heap::init();
        black_box(run(black_box(&DATAGRAMS)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// The critical section on Cortex-M, which the allocator and, on ARMv6-M,
/// `portable-atomic` lock with. It masks interrupts with PRIMASK and puts
/// back the mask it found, so sections nest. On a part with one core that
/// stops every interrupt handler but NMI's and HardFault's; a multi-core
/// part would need more.
///
/// It is what `cortex-m`'s `critical-section-single-core` feature provides,
/// but that crate depends on the unmaintained `bare-metal`
/// (RUSTSEC-2026-0110), which Cargo Deny refuses.
#[cfg(all(target_os = "none", target_arch = "arm"))]
#[allow(unsafe_code)]
mod single_core {
    use core::arch::asm;
    use core::sync::atomic::{compiler_fence, Ordering};

    use critical_section::RawRestoreState;

    struct SingleCore;

    critical_section::set_impl!(SingleCore);

    // SAFETY: on one core, no interrupt handler runs while PRIMASK masks
    // interrupts, so a section excludes every other. PRIMASK leaves NMI and
    // HardFault unmasked, as with cortex-m's implementation: their handlers
    // must not enter a critical section or touch what one guards, and this
    // image's never do (cortex-m-rt's defaults only loop). `release` unmasks
    // only when its `acquire` found interrupts unmasked, so an inner
    // section's release leaves the outer section masked. The compiler fences
    // keep the section's memory accesses inside it, which on one core is the
    // ordering the trait's contract asks for.
    unsafe impl critical_section::Impl for SingleCore {
        unsafe fn acquire() -> RawRestoreState {
            let primask: u32;
            // SAFETY: reads PRIMASK, then masks interrupts (CPSID I); both
            // instructions exist on ARMv6-M and ARMv7-M, and neither touches
            // memory or the stack.
            unsafe {
                asm!(
                    "mrs {}, PRIMASK",
                    "cpsid i",
                    out(reg) primask,
                    options(nomem, nostack, preserves_flags),
                );
            }
            compiler_fence(Ordering::SeqCst);
            // Bit 0 clear: interrupts were unmasked, so release unmasks them.
            primask & 1 == 0
        }

        unsafe fn release(was_unmasked: RawRestoreState) {
            if was_unmasked {
                compiler_fence(Ordering::SeqCst);
                // SAFETY: unmasks interrupts (CPSIE I), as they were before
                // the matching acquire; touches no memory or stack.
                unsafe { asm!("cpsie i", options(nomem, nostack, preserves_flags)) };
            }
        }
    }
}

/// RISC-V (RV32IMC, the ESP32-C3 class), on riscv-rt.
#[cfg(all(target_os = "none", target_arch = "riscv32", not(feature = "emulate")))]
mod image {
    use core::hint::black_box;

    use firmware_link::apdu::{run, DATAGRAMS};

    /// Links riscv, and with it the single-hart critical section its
    /// `critical-section-single-hart` feature defines. riscv-rt links the
    /// crate already; this keeps the image from relying on that.
    use riscv as _;

    /// The reset entry.
    #[riscv_rt::entry]
    fn main() -> ! {
        super::heap::init();
        black_box(run(black_box(&DATAGRAMS)));
        loop {
            core::hint::spin_loop();
        }
    }
}

// The emulation build's harness, documented in its file.
#[cfg(all(target_os = "none", feature = "emulate"))]
mod emulate;

/// The emulation build's reset entry, on either runtime: `image`'s, with the
/// summary going to the harness instead of `black_box`. The datagrams still
/// pass through `black_box`, so the scenario can't be worked out at compile
/// time.
#[cfg(all(target_os = "none", feature = "emulate"))]
mod emulated {
    use core::hint::black_box;

    use firmware_link::apdu::{expected, run, DATAGRAMS};

    /// Links riscv's single-hart critical section, as in `image`.
    #[cfg(target_arch = "riscv32")]
    use riscv as _;

    /// On Cortex-M, as `image::main`.
    #[cfg(target_arch = "arm")]
    #[cortex_m_rt::entry]
    fn main() -> ! {
        super::heap::init();
        crate::emulate::finish(&run(black_box(&DATAGRAMS)), expected)
    }

    /// On RISC-V, as `image::main`.
    #[cfg(target_arch = "riscv32")]
    #[riscv_rt::entry]
    fn main() -> ! {
        super::heap::init();
        crate::emulate::finish(&run(black_box(&DATAGRAMS)), expected)
    }
}

/// A host has nothing to flash: `tests/apdu.rs` runs the scenario.
#[cfg(not(target_os = "none"))]
fn main() {}
