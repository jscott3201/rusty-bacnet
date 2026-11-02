#![no_main]

//! See `objects_fuzz` for what this target checks: `object_file_restore` from real files.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    objects_fuzz::check_files(objects_fuzz::Target::ObjectFile, input);
});
