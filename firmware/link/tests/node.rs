//! Runs the node image's scenario natively and checks what it reports.
//! This is where CI verifies the scenario's behaviour: CI only links the
//! bare-metal image, and its emulation build, which checks the same summary
//! on an emulated CPU, runs by hand (docs/ci.md, "Emulation (manual)").

use core::mem::size_of;

use firmware_link::node::{expected, run, ForeignNode, Memory, Summary};

/// The whole summary against the one the emulation build checks too
/// (`firmware_link::node::expected`, which explains each value).
#[test]
fn scenario_summary() {
    let mut memory = Memory::new();
    assert_eq!(run(&mut memory), expected());
}

/// Prints the sizes behind the image's RAM use, for the host's pointer
/// width; check-firmware-link.sh prints the bare-metal ones from the image.
/// nextest shows the output (.config/nextest.toml).
#[test]
fn ram_sizes() {
    let node = size_of::<ForeignNode>();
    println!("Node<2> (the image's, on the stack): {node} octets");
    println!("Summary: {} octets", size_of::<Summary>());
    println!("Memory: {} octets", size_of::<Memory>());
    assert!(node < 1024, "the node is {node} octets");
}
