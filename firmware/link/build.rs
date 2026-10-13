//! Hands the linker the image's memory layout on a bare-metal target: the
//! generic `memory.x`, which the link test's images use, or in the emulation
//! build the layout of the board its one board feature names, from `boards/`.
//! The emulation build also gets the board's QEMU machine name and the target
//! triple, which its images print.
//!
//! A host build gets nothing: its binary is a stub that links like any other.

use std::env;
use std::fs;
use std::path::PathBuf;

/// The emulation build's boards: each one's feature as cargo's environment
/// spells it (`board-sifive-e` is `CARGO_FEATURE_BOARD_SIFIVE_E`), its QEMU
/// machine, which also names its layout in `boards/`, and the architecture
/// its CPU runs.
const BOARDS: [(&str, &str, &str); 4] = [
    ("MICROBIT", "microbit", "arm"),
    ("LM3S6965EVB", "lm3s6965evb", "arm"),
    ("NETDUINOPLUS2", "netduinoplus2", "arm"),
    ("SIFIVE_E", "sifive_e", "riscv32"),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=boards");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    let layout = match board() {
        Some(machine) => {
            let target = env::var("TARGET").expect("cargo sets TARGET");
            println!("cargo:rustc-env=FIRMWARE_LINK_BOARD={machine}");
            println!("cargo:rustc-env=FIRMWARE_LINK_TARGET={target}");
            format!("boards/{machine}.x")
        }
        None => "memory.x".to_owned(),
    };
    // The runtime's link.x includes memory.x from the linker's search path.
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::copy(&layout, out.join("memory.x"))
        .unwrap_or_else(|error| panic!("copy {layout} into OUT_DIR: {error}"));
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
}

/// The QEMU machine the board features pick: none without the `emulate`
/// feature, and exactly one, whose CPU runs the target's architecture, with
/// it.
fn board() -> Option<&'static str> {
    let picked: Vec<_> = BOARDS
        .iter()
        .filter(|(feature, ..)| env::var_os(format!("CARGO_FEATURE_BOARD_{feature}")).is_some())
        .collect();
    let emulate = env::var_os("CARGO_FEATURE_EMULATE").is_some();
    match picked[..] {
        [] if !emulate => None,
        [&(_, machine, arch)] => {
            let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
            assert!(
                target_arch == arch,
                "board {machine} runs {arch} code, and this build is for {target_arch}"
            );
            Some(machine)
        }
        _ => panic!(
            "the emulation build needs exactly one board feature: board-microbit, \
             board-lm3s6965evb, board-netduinoplus2 or board-sifive-e"
        ),
    }
}
