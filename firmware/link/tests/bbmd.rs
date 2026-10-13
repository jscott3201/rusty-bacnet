//! Runs the BBMD image's scenario natively and checks what it reports.
//! This is where CI verifies the scenario's behaviour: CI only links the
//! bare-metal image, and its emulation build, which checks the same summary
//! on an emulated CPU, runs by hand (docs/ci.md, "Emulation (manual)").

use core::mem::size_of;

use firmware_link::bbmd::{run, BbmdTables, Memory, Summary, EXPECTED};

/// The whole summary against the one the emulation build checks too
/// (`firmware_link::bbmd::EXPECTED`, which explains each value).
#[test]
fn scenario_summary() {
    let mut memory = Memory::new();
    assert_eq!(run(&mut memory), EXPECTED);
}

/// Prints the sizes behind the image's RAM use, for the host's pointer
/// width; check-firmware-link.sh prints the bare-metal ones from the image.
/// nextest shows the output (.config/nextest.toml).
#[test]
fn ram_sizes() {
    let tables = size_of::<BbmdTables>();
    println!("Tables<4, 4> (the image's, on the stack): {tables} octets");
    println!("Summary: {} octets", size_of::<Summary>());
    println!("Memory: {} octets", size_of::<Memory>());
    assert!(tables < 512, "the tables are {tables} octets");
}
