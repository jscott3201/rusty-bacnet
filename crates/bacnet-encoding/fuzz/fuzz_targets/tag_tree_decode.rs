#![no_main]

//! See `encoding_fuzz::tag_tree` for what this target checks.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| encoding_fuzz::tag_tree::check(input));
