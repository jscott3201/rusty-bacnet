//! The emulation build's harness, which each image includes with the
//! `emulate` feature (docs/ci.md, "Emulation (manual)").
//!
//! `scripts/emu/emulate-firmware.sh` builds the images with that feature and
//! one board's, and runs each on the board's QEMU machine with semihosting
//! on. An image's reset entry runs its scenario as the link test's image
//! does, then hands the summary to [`finish`], which:
//!
//! 1. measures how deep the stack went. The runtime paints the stack at
//!    reset: cortex-m-rt with its `paint-stack` feature, riscv-rt through
//!    the `__pre_init` below. A word the program writes all but surely loses
//!    the paint, so the lowest word without it is the high-water mark. A
//!    frame's words that nothing ever writes keep their paint, so the mark
//!    is what the frames wrote, at most what they reserved;
//! 2. compares the summary with the one the host test asserts;
//! 3. prints the summary and a verdict through semihosting, and exits with a
//!    status QEMU passes on as its own: [`PASS`], [`MISMATCH`],
//!    [`STACK_EXHAUSTED`], [`FAULT`] or [`PANIC`].
//!
//! The panic handler and the fault handlers report through semihosting too,
//! so a run ends with a status instead of spinning until the runner's
//! timeout.
//!
//! The `unsafe` code here is what the measurement and the handlers need:
//! reading the stack's words, the RISC-V stack paint and register reads in
//! assembly, and the handlers' exported names.

use core::fmt::Debug;
use core::ptr::addr_of;

use semihosting::process::exit;
use semihosting::{eprintln, println};

/// The image, from cargo.
const IMAGE: &str = env!("CARGO_BIN_NAME");
/// The board's QEMU machine, from build.rs.
const BOARD: &str = env!("FIRMWARE_LINK_BOARD");
/// The target triple, from build.rs.
const TARGET: &str = env!("FIRMWARE_LINK_TARGET");

/// The summary is as expected, with stack paint to spare.
const PASS: i32 = 0;
/// The summary differs from the expected one.
const MISMATCH: i32 = 1;
/// No paint is left on the stack: it may have run into the statics.
const STACK_EXHAUSTED: i32 = 2;
/// The CPU faulted: a HardFault on Cortex-M, an exception on RISC-V.
const FAULT: i32 = 3;
/// The image panicked: the status a Rust program on a host exits with
/// after a panic.
const PANIC: i32 = 101;

/// The word the stack is painted with, cortex-m-rt's `STACK_PAINT_VALUE`.
#[cfg(target_arch = "arm")]
const PAINT: u32 = cortex_m_rt::STACK_PAINT_VALUE;
/// The same word on RISC-V, which `__pre_init` paints the stack with.
#[cfg(target_arch = "riscv32")]
const PAINT: u32 = 0xCCCC_CCCC;

/// Measure the stack, compare `summary` with `expected()`, report both
/// through semihosting, and exit with the status that says how it went.
///
/// `expected` is built only after the measurement, so building it can't
/// count toward the stack's high-water mark. It is usually a summary too;
/// the node image's is its own type, since the summary holds core types it
/// can't build.
pub fn finish<S: Debug + PartialEq<E>, E: Debug>(summary: &S, expected: impl FnOnce() -> E) -> ! {
    let stack = Stack::measure();
    let expected = expected();
    let status = if *summary != expected {
        println!("expected: {expected:#?}");
        println!("actual: {summary:#?}");
        MISMATCH
    } else {
        println!("summary: {summary:?}");
        if stack.peak < stack.size {
            PASS
        } else {
            STACK_EXHAUSTED
        }
    };
    let (verdict, outcome) = match status {
        PASS => ("PASS", "summary as expected"),
        MISMATCH => ("FAIL", "summary differs (expected and actual above)"),
        _ => ("FAIL", "summary as expected, but no stack paint is left"),
    };
    println!(
        "{verdict} {IMAGE} on {BOARD} ({TARGET}): {outcome}; stack peak {} of {} octets",
        stack.peak, stack.size
    );
    exit(status)
}

/// How deep the stack went, from the paint left on it.
struct Stack {
    /// Octets from the top of the stack down to the lowest word that lost
    /// its paint.
    peak: usize,
    /// Octets between the top of the stack and its bottom, the end of the
    /// statics.
    size: usize,
}

impl Stack {
    /// Scan the stack from its bottom up for the first word without paint.
    fn measure() -> Self {
        // The stack's bounds, from the runtime's link.x: it grows down from
        // `_stack_start`, the top of RAM, toward its bottom just above the
        // statics, cortex-m-rt's `_stack_end` or riscv-rt's `__estack`. Only
        // their addresses are used: each is a word boundary, not a word of
        // data.
        extern "C" {
            #[cfg_attr(target_arch = "arm", link_name = "_stack_end")]
            #[cfg_attr(target_arch = "riscv32", link_name = "__estack")]
            static STACK_BOTTOM: u32;
            #[link_name = "_stack_start"]
            static STACK_TOP: u32;
        }
        let bottom = addr_of!(STACK_BOTTOM);
        let top = addr_of!(STACK_TOP);
        let mut word = bottom;
        // SAFETY: every word from the bottom up to the top is the stack's
        // own RAM, mapped and readable, and both bounds are word-aligned:
        // cortex-m-rt's link.x asserts it, and riscv-rt's aligns the end of
        // the heap, `__estack`, to 4. The scan stops at the first word
        // without paint, so it reads only words below every live frame,
        // which no object holds, and that one word. Reads write nothing,
        // and being volatile, none is dropped or merged.
        #[allow(unsafe_code)]
        while word < top && unsafe { word.read_volatile() } == PAINT {
            word = word.wrapping_add(1);
        }
        Self {
            peak: top.addr() - word.addr(),
            size: top.addr() - bottom.addr(),
        }
    }
}

/// Report a panic and exit with [`PANIC`]: what Rust's test harness does,
/// with the panic's location and message.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    eprintln!("FAIL {IMAGE} on {BOARD} ({TARGET}): {info}");
    exit(PANIC)
}

/// Report a HardFault and exit with [`FAULT`]. On these parts every fault
/// escalates to it: the image enables no configurable fault handler.
#[cfg(target_arch = "arm")]
#[allow(unsafe_code)]
#[cortex_m_rt::exception]
unsafe fn HardFault(frame: &cortex_m_rt::ExceptionFrame) -> ! {
    eprintln!(
        "FAIL {IMAGE} on {BOARD} ({TARGET}): HardFault at pc {:#010x}",
        frame.pc()
    );
    exit(FAULT)
}

/// Report an exception and exit with [`FAULT`]. riscv-rt calls it for any
/// exception without a handler of its own, and this image has none.
#[cfg(target_arch = "riscv32")]
#[allow(unsafe_code)]
#[export_name = "ExceptionHandler"]
extern "C" fn exception(_frame: &riscv_rt::TrapFrame) -> ! {
    let (cause, pc): (usize, usize);
    // SAFETY: reads two machine-mode CSRs, the trap's cause and the address
    // of the instruction that trapped. The image runs in machine mode, so
    // both reads are allowed; they touch no memory or stack.
    unsafe {
        core::arch::asm!(
            "csrr {cause}, mcause",
            "csrr {pc}, mepc",
            cause = out(reg) cause,
            pc = out(reg) pc,
            options(nomem, nostack, preserves_flags),
        );
    }
    eprintln!("FAIL {IMAGE} on {BOARD} ({TARGET}): exception {cause} at pc {pc:#010x}");
    exit(FAULT)
}

/// Paints the stack before RAM is set up, as cortex-m-rt's `paint-stack`
/// does on Cortex-M: riscv-rt's `pre-init` feature calls `__pre_init` from
/// its startup code, with `sp` at the top of the stack and nothing on it
/// yet.
#[cfg(target_arch = "riscv32")]
#[allow(unsafe_code)]
mod pre_init {
    // SAFETY: it writes only the stack's words, `__estack` up to
    // `_stack_start`, which nothing holds yet and which are word-aligned, as
    // riscv-rt's link.x places them; and it uses only the temporaries t0-t2,
    // as riscv-rt asks of `__pre_init`: its startup code keeps `main`'s
    // arguments in s0-s2. It returns to that code through `ra`, untouched.
    core::arch::global_asm!(
        ".section .init.pre_init, \"ax\"",
        ".global __pre_init",
        "__pre_init:",
        "    la t0, __estack",
        "    la t1, _stack_start",
        "    li t2, {paint}",
        "1:  bgeu t0, t1, 2f",
        "    sw t2, 0(t0)",
        "    addi t0, t0, 4",
        "    j 1b",
        "2:  ret",
        paint = const super::PAINT,
    );
}
