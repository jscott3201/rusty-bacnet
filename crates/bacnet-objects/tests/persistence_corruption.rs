//! Truncation and single-bit damage to a sample of every saved format
//! (#2183, #2180).
//!
//! Each format's sample is written through its file backend, then each
//! saved file is cut off at every length and has bits flipped one at a
//! time (every bit of a slot, one bit of each octet of a one-file format),
//! and the restore check in `support/restore.rs` runs on the result: no
//! panic, a refused restore leaves the files as they were, and whatever
//! loads saves and loads back the same. A two-slot format's slots end with
//! a CRC-32, which catches every single-bit flip and every cut, so there no
//! damaged slot may load at all.

#![cfg(feature = "std")]

#[path = "support/restore.rs"]
mod restore;

use restore::{FORMATS, Format, Outcome, Scratch, read_files, write_files};

/// Run the check over every truncation and bit flip of each file `format`
/// saved, returning how many damaged inputs still loaded.
fn damage(format: &Format) -> usize {
    let scratch = Scratch::new(&format!("corruption-{}", format.name));
    let base = scratch.base("state");
    (format.sample)(&base);
    let files = format.files(&base);
    let saved = read_files(&files);
    assert_eq!(
        (format.check)(&base),
        Outcome {
            loaded: true,
            built: true
        },
        "{}: the sample doesn't restore",
        format.name
    );
    let mut loaded = 0;
    let mut run = |variant: Vec<Option<Vec<u8>>>, what: &str| {
        write_files(&files, &variant);
        let outcome = (format.check)(&base);
        assert!(
            !(format.two_slot && outcome.loaded),
            "{}: a slot {what} loaded",
            format.name
        );
        loaded += usize::from(outcome.loaded);
    };
    for (index, octets) in saved.iter().enumerate() {
        let Some(octets) = octets else { continue };
        let with = |damaged: Vec<u8>| {
            let mut variant = saved.clone();
            variant[index] = Some(damaged);
            variant
        };
        for length in 0..octets.len() {
            run(
                with(octets[..length].to_vec()),
                &format!("cut to {length} octets"),
            );
        }
        for bit in 0..octets.len() * 8 {
            // A one-file format that loads damage pays a synchronized save
            // for the round trip, so it takes one flip per octet, a
            // different bit in each; a slot takes all eight.
            if !format.two_slot && bit % 8 != (bit / 8) % 8 {
                continue;
            }
            let mut damaged = octets.clone();
            damaged[bit / 8] ^= 1 << (bit % 8);
            run(with(damaged), &format!("with bit {bit} flipped"));
        }
    }
    loaded
}

#[test]
fn damaged_files_never_panic_and_refused_restores_write_nothing() {
    for format in FORMATS {
        let loaded = damage(format);
        // The one-file formats have no checksum: some flips land on a value
        // that is still valid (a time, a flag, a kelvin), and a cut at an
        // entry boundary leaves a shorter list.
        if !format.two_slot {
            assert!(loaded > 0, "{}: no damaged input loaded", format.name);
        }
    }
}

/// The samples cover every format the crate saves: one per file backend
/// (the Color backend twice, once for each object it serves).
#[test]
fn every_format_has_a_sample() {
    let names: Vec<&str> = FORMATS.iter().map(|format| format.name).collect();
    assert_eq!(
        names,
        [
            "trend_log",
            "trend_log_multiple",
            "event_log",
            "audit_log",
            "notification_forwarder",
            "notification_class",
            "tags",
            "color",
            "color_temperature",
            "access_rights",
        ]
    );
}
