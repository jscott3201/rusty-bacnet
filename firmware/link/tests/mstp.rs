//! Runs the MS/TP image's scenario natively and checks what it reports.
//! This is where CI verifies the scenario's behaviour: CI only links the
//! bare-metal image, and its emulation build, which checks the same summary
//! on an emulated CPU, runs by hand (docs/ci.md, "Emulation (manual)").

use core::mem::size_of;

use bacnet_mstp_codec::{StreamDecoder, MAX_STANDARD_FRAME_LENGTH};
use bacnet_mstp_core::{ActionBuffers, HeaplessQueues, MasterCore};
use firmware_link::mstp::{run, Actions, Core, Memory, Queues, EXPECTED};

/// The whole summary, against the one the emulation build checks too
/// (`firmware_link::mstp::EXPECTED`, which explains each value).
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
    let owning = size_of::<MasterCore<'static, Queues>>();
    let queues = size_of::<Queues>();
    let actions = size_of::<Actions>();
    let default_actions = size_of::<ActionBuffers>();
    let host = size_of::<MasterCore<'static, HeaplessQueues<4, 2>>>();
    let decoder = size_of::<StreamDecoder<'static>>();
    println!("MasterCore<&mut HeaplessQueues<4, 2>> (the image's): {core} octets");
    println!("MasterCore<HeaplessQueues<4, 2>>: {owning} octets, its queues {queues}");
    println!(
        "ActionBuffers<1> (the image's): {actions} octets; ActionBuffers<8>: {default_actions}"
    );
    println!("MasterCore<HeaplessQueues<4, 2>> with ActionBuffers<8>: {host} + {default_actions}");
    println!(
        "StreamDecoder: {decoder} octets, {} with {MAX_STANDARD_FRAME_LENGTH} octets of storage",
        decoder + MAX_STANDARD_FRAME_LENGTH
    );
    println!("Memory: {} octets", size_of::<Memory>());
    // The core holds its decoder, but borrows the decoder's storage, its
    // queues and its action buffers: it stays small whatever their size.
    assert!(core > decoder);
    assert!(core < 1024, "the core is {core} octets");
    assert_eq!(owning, host);
    assert!(owning > queues + decoder);
}
