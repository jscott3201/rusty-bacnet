#![no_main]

use bacnet_mstp_codec::decode_frame;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = decode_frame(input);
});
