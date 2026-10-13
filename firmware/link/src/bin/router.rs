//! The firmware link test's router image (#1679).
//!
//! On a bare-metal target this is a `no_main` image without a global
//! allocator. Its reset entry runs [`firmware_link::router::run`] once,
//! with the storage the router, the MS/TP core and the BBMD port borrow in
//! a static; the router, the core and the port are built on the stack. Its
//! input and its summary pass through [`core::hint::black_box`], so none of
//! the work can be optimized away, and then it loops. CI never runs it:
//! building it proves that `bacnet-router-core` with its B/IP and MS/TP
//! glue, over `bacnet-bip-core`'s BBMD and `bacnet-mstp-core` with extended
//! frames, links without `std` or an allocator, and
//! `scripts/ci/check-firmware-link.sh` checks its sizes. On a host the
//! binary is a stub; `tests/router.rs` runs the scenario there.
//!
//! The emulation build (the `emulate` feature) swaps in the reset entry of
//! `emulated`, which runs under QEMU by hand: it runs the same scenario, and
//! the `emulate` module checks the summary against
//! [`firmware_link::router::expected`] and reports through semihosting.
//!
//! The only `unsafe` code is what the runtimes need: the code their entry
//! macros generate, and on RISC-V one block that borrows the static, which
//! cortex-m-rt's macro does safely on Cortex-M; and in the emulation build,
//! the `emulate` module's.

#![cfg_attr(target_os = "none", no_std, no_main)]

/// Cortex-M (ARMv6-M and ARMv7-M), on cortex-m-rt.
#[cfg(all(target_os = "none", target_arch = "arm", not(feature = "emulate")))]
mod image {
    use core::hint::black_box;

    use firmware_link::router::{run, Memory};

    /// The reset entry. cortex-m-rt's macro turns the `static mut` below
    /// into a `&'static mut`, which is sound because the entry runs once.
    #[cortex_m_rt::entry]
    fn main() -> ! {
        static mut MEMORY: Memory = Memory::new();
        black_box(run(black_box(MEMORY)));
        loop {
            core::hint::spin_loop();
        }
    }
}

/// RISC-V (RV32IMC, the ESP32-C3 class), on riscv-rt.
#[cfg(all(target_os = "none", target_arch = "riscv32", not(feature = "emulate")))]
mod image {
    use core::hint::black_box;
    use core::ptr::addr_of_mut;

    use firmware_link::router::{run, Memory};

    static mut MEMORY: Memory = Memory::new();

    /// The reset entry. riscv-rt's macro has no counterpart to cortex-m-rt's
    /// `static mut` handling, so the static is borrowed by hand.
    #[riscv_rt::entry]
    fn main() -> ! {
        // SAFETY: this `&mut` is the only reference to the static for the
        // rest of the program. The image targets single-hart parts (which is
        // why riscv-rt's `single-hart` feature, which only drops its
        // multi-hart gate, is on), so one hart exists, its startup code
        // calls `main` once, and `main` never returns. No interrupt handler
        // can run either: the startup code writes 0 to `mie`, `mstatus.MIE`
        // is clear at reset, and nothing here enables either. Nothing else
        // names the static.
        #[allow(unsafe_code)]
        let memory = unsafe { &mut *addr_of_mut!(MEMORY) };
        black_box(run(black_box(memory)));
        loop {
            core::hint::spin_loop();
        }
    }
}

// The emulation build's harness, documented in its file.
#[cfg(all(target_os = "none", feature = "emulate"))]
mod emulate;

/// The emulation build's reset entry, on either runtime: `image`'s, with the
/// summary going to the harness instead of `black_box`. The input still
/// passes through `black_box`, so the scenario can't be worked out at
/// compile time.
#[cfg(all(target_os = "none", feature = "emulate"))]
mod emulated {
    use core::hint::black_box;

    use firmware_link::router::{expected, run, Memory};

    /// On Cortex-M, as `image::main`.
    #[cfg(target_arch = "arm")]
    #[cortex_m_rt::entry]
    fn main() -> ! {
        static mut MEMORY: Memory = Memory::new();
        crate::emulate::finish(&run(black_box(MEMORY)), expected)
    }

    #[cfg(target_arch = "riscv32")]
    static mut MEMORY: Memory = Memory::new();

    /// On RISC-V, as `image::main`.
    #[cfg(target_arch = "riscv32")]
    #[riscv_rt::entry]
    fn main() -> ! {
        // SAFETY: as in `image::main`: the only reference to the static, on
        // a single hart, from a `main` that the startup code calls once and
        // that never returns. No interrupt is enabled, and the exception
        // handler in `emulate` never names the static.
        #[allow(unsafe_code)]
        let memory = unsafe { &mut *core::ptr::addr_of_mut!(MEMORY) };
        crate::emulate::finish(&run(black_box(memory)), expected)
    }
}

/// A host has nothing to flash: `tests/router.rs` runs the scenario.
#[cfg(not(target_os = "none"))]
fn main() {}
