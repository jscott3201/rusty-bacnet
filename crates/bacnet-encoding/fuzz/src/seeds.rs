//! Replays every seed input in `seeds/<target>/` through its target's check.

use std::fs;
use std::path::Path;

/// Runs `check` on each file in `seeds/<target>/`; returns how many.
fn replay(target: &str, check: fn(&[u8])) -> usize {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("seeds")
        .join(target);
    let mut replayed = 0;
    for entry in fs::read_dir(&directory).expect("the seeds directory") {
        let path = entry.expect("a directory entry").path();
        let input = fs::read(&path).expect("a seed input");
        check(&input);
        replayed += 1;
    }
    replayed
}

#[test]
fn apdu_seeds_pass() {
    assert!(replay("apdu_decode", crate::apdu::check) > 0);
}

#[test]
fn value_seeds_pass() {
    assert!(replay("value_decode", crate::value::check) > 0);
}

#[test]
fn constructed_seeds_pass() {
    assert!(replay("constructed_decode", crate::constructed::check) > 0);
}

#[test]
fn service_seeds_pass() {
    assert!(replay("service_decode", crate::service::check) > 0);
}
