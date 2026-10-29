//! Runs the device image's scenario natively and checks what it reports.
//! This is where CI verifies the scenario's behaviour: CI only links the
//! bare-metal image, and its emulation build, which checks the same summary
//! on an emulated CPU, runs by hand (docs/ci.md, "Emulation (manual)").

use core::mem::size_of;

use firmware_link::device::app::{ANSWER_LENGTH, Answer, Scratch, answer};
use firmware_link::device::{
    Core, GLOBAL_READ_PROPERTY, Memory, REQUESTS, Storage, Summary, expected, run,
};

/// The whole summary, against the one the emulation build checks too
/// (`firmware_link::device::expected`, which explains each value).
#[test]
fn scenario_summary() {
    let mut memory = Memory::new();
    assert_eq!(run(&mut memory), expected());
}

/// The application alone: each answer fits the buffer the image gives it,
/// and what it doesn't serve gets no answer.
#[test]
fn the_application_answers_within_its_buffers() {
    let mut scratch = Scratch::new();
    let mut out = [0; ANSWER_LENGTH];
    for request in REQUESTS {
        assert!(
            matches!(answer(request, &mut scratch, &mut out), Answer::Reply(n) if n <= ANSWER_LENGTH),
            "{request:02X?}"
        );
    }
    for ignored in [
        // Message type 0 for the network layer (Who-Is-Router-To-Network).
        &[0x01, 0x80, 0x00][..],
        // A ReadProperty for DNET 9, which a device that isn't a router
        // drops.
        &[
            0x01, 0x24, 0x00, 0x09, 0x01, 0x05, 0xFF, 0x00, 0x03, 0x01, 0x0C, 0x0C, 0x02, 0x00,
            0x04, 0xD2, 0x19, 0x4D,
        ],
        // An I-Am, an unconfirmed service the device doesn't serve.
        &[0x01, 0x00, 0x10, 0x00, 0xC4, 0x02, 0x00, 0x00, 0x01],
        // A ComplexACK, which answers nothing the device asked.
        &[0x01, 0x00, 0x30, 0x01, 0x0C],
        // A ReadProperty sent to every network: confirmed requests don't
        // travel by broadcast.
        &GLOBAL_READ_PROPERTY,
        // A Who-Is whose low limit (5) is above its high one (1).
        &[0x01, 0x00, 0x10, 0x08, 0x09, 0x05, 0x19, 0x01],
        // Protocol version 2.
        &[0x02, 0x00, 0x10, 0x08],
        // An empty APDU.
        &[0x01, 0x00],
    ] {
        assert_eq!(
            answer(ignored, &mut scratch, &mut out),
            Answer::Nothing,
            "{ignored:02X?}"
        );
    }
    // A Who-Is to every network is answered as a local one is.
    let global = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
    assert!(matches!(
        answer(&global, &mut scratch, &mut out),
        Answer::Broadcast(20)
    ));
    // An answer that doesn't fit the output is dropped, not cut short.
    assert_eq!(
        answer(REQUESTS[0], &mut scratch, &mut out[..10]),
        Answer::Nothing
    );
}

/// Prints the sizes behind the image's RAM use, for the host's pointer
/// width; check-firmware-link.sh prints the bare-metal ones from the image.
/// nextest shows the output (.config/nextest.toml).
#[test]
fn ram_sizes() {
    let core = size_of::<Core<'static>>();
    println!("MasterCore<&mut FixedQueues<1, 2>> (the image's, on the stack): {core} octets");
    println!("StandardStorage<1, 2, 1>: {} octets", size_of::<Storage>());
    println!("Scratch: {} octets", size_of::<Scratch>());
    println!("Summary: {} octets", size_of::<Summary>());
    println!("Memory: {} octets", size_of::<Memory>());
    assert!(core < 1024, "the core is {core} octets");
}
