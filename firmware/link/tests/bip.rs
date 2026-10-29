//! Runs the B/IP image's scenario natively and checks what it reports.
//! This is where CI verifies the scenario's behaviour: CI only links the
//! bare-metal image, and its emulation build, which checks the same summary
//! on an emulated CPU, runs by hand (docs/ci.md, "Emulation (manual)").

use firmware_link::bip::{EXPECTED, Memory, run};

/// The whole summary, its B/IPv6 part included, against the one the
/// emulation build checks too (`firmware_link::bip::EXPECTED`, which
/// explains each value).
#[test]
fn scenario_summary() {
    let mut memory = Memory::new();
    assert_eq!(run(&mut memory), EXPECTED);
}
