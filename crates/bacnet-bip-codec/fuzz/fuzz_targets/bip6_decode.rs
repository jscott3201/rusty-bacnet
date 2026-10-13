#![no_main]

use bacnet_bip_codec::ipv6::{decode, Bvll6Error, Bvll6Frame};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    if let Ok(frame) = Bvll6Frame::decode(input) {
        let _ = frame.message();
    }
    // An accepted datagram re-encodes to itself, unless its result code is
    // one Annex U doesn't define.
    if let Ok(message) = decode(input) {
        let mut wire = vec![0u8; input.len()];
        match message.encode_into(&mut wire) {
            Ok(length) => assert_eq!(&wire[..length], input),
            Err(error) => assert!(matches!(error, Bvll6Error::UndefinedResultCode { .. })),
        }
    }
});
