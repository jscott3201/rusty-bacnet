#![no_main]

//! See `encoding_fuzz::constructed` for what this target checks.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| encoding_fuzz::constructed::check(input));
