//! Runs the mstp-extended image's scenario natively and checks what it
//! reports. This is where CI verifies the scenario's behaviour: CI only
//! links the bare-metal image, and its emulation build, which checks the
//! same summary on an emulated CPU, runs by hand (docs/ci.md, "Emulation
//! (manual)").

use core::mem::size_of;

use firmware_link::mstp_extended::{run, Actions, Core, Memory, Queues, EXPECTED};

/// The whole summary against the one the emulation build checks too
/// (`firmware_link::mstp_extended::EXPECTED`, which explains each value).
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
    let core = size_of::<Core<'static>>();
    println!("MasterCore<&mut Queues, 1497> (the image's, on the stack): {core} octets");
    println!("HeaplessQueues<1, 1, 1497>: {} octets", size_of::<Queues>());
    println!("ExtendedActionBuffers<1>: {} octets", size_of::<Actions>());
    println!("Memory: {} octets", size_of::<Memory>());
    // The core borrows its large buffers, so it stays small with extended
    // frames on.
    assert!(core < 1024, "the core is {core} octets");
}
