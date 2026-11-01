#![no_main]

//! See `objects_fuzz` for what this target checks.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    objects_fuzz::check(objects_fuzz::Target::Log, input);
});
