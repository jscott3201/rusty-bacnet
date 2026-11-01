#![no_main]

//! See `encoding_fuzz::service` for what this target checks.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| encoding_fuzz::service::check(input));
