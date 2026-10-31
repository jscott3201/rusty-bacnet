//! The critical section the allocator images (apdu, objects) take on
//! Cortex-M; each image includes it, as only a firmware may choose one. Its
//! documentation is on the module declaration in each image.

use core::arch::asm;
use core::sync::atomic::{Ordering, compiler_fence};

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
