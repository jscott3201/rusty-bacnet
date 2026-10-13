//! Runs the router image's scenario natively and checks what it reports.
//! This is where CI verifies the scenario's behaviour: CI only links the
//! bare-metal image, and its emulation build, which checks the same summary
//! on an emulated CPU, runs by hand (docs/ci.md, "Emulation (manual)").

use core::mem::size_of;

use firmware_link::router::{
    expected, run, Actions, BbmdTables, BipPort, Core, Memory, Queues, RouterEngine, RouterStorage,
    Summary,
};

/// The whole summary against the one the emulation build checks too
/// (`firmware_link::router::expected`, which explains each value).
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
    let router = size_of::<RouterEngine<'static>>();
    let core = size_of::<Core<'static>>();
    let port = size_of::<BipPort<'static>>();
    println!(
        "Storage<2, 8, 8, 4> (the router's, in the static): {} octets",
        size_of::<RouterStorage>()
    );
    println!("HeaplessQueues<1, 2, 1497>: {} octets", size_of::<Queues>());
    println!("ExtendedActionBuffers<1>: {} octets", size_of::<Actions>());
    println!("Memory: {} octets", size_of::<Memory>());
    println!("Router (on the stack): {router} octets");
    println!("MasterCore (on the stack): {core} octets");
    println!("BbmdPort with its Node<1> (on the stack): {port} octets");
    println!(
        "Tables<2, 2> (on the stack): {} octets",
        size_of::<BbmdTables>()
    );
    println!("Summary (on the stack): {} octets", size_of::<Summary>());
    // The router and the core borrow their large buffers, so both stay
    // small.
    assert!(router < 128, "the router is {router} octets");
    assert!(core < 1024, "the core is {core} octets");
}
