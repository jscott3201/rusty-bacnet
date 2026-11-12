//! Truncation and single-bit damage to a sample of every saved format
//! (#2183, #2180).
//!
//! Each format's sample is written through its file backend, then each
//! saved file is cut off at every length and has bits flipped one at a
//! time (every bit of a slot, one bit of each octet of a one-file format),
//! and the restore check in `support/restore.rs` runs on the result: no
//! panic, a refused restore leaves the files as they were, and whatever
//! loads saves and loads back the same. A two-slot format's slot declares
//! its length and ends with a CRC-32: the length check catches every cut
//! and the CRC every single-bit flip, so no damaged slot may load. A second
//! pass then re-seals each damaged slot (its length field and CRC made to
//! match, one flip per octet) so the damage reaches the payload decoders,
//! where a load is allowed but the same check applies.

// std: every format here is read and written through its file backend.
#![cfg(feature = "std")]

#[path = "support/restore.rs"]
mod restore;

use restore::{FORMATS, Format, Outcome, Scratch, read_files, seal, write_files};

/// Run the check over every truncation and bit flip of each file `format`
/// saved, returning how many damaged inputs still loaded.
fn damage(format: &Format) -> usize {
    let scratch = Scratch::new(&format!("corruption-{}", format.name));
    let base = scratch.base("state");
    (format.sample)(&base);
    let files = format.files(&base);
    let saved = read_files(&files);
    assert_eq!(
        format.check_files(&base),
        Outcome {
            loaded: true,
            built: true
        },
        "{}: the sample doesn't restore",
        format.name
    );
    let mut loaded = 0;
    // Each damaged file, with whether it was re-sealed.
    let mut run = |variant: Vec<Option<Vec<u8>>>, sealed: bool, what: &str| {
        write_files(&files, &variant);
        let outcome = format.check_files(&base);
        assert!(
            sealed || !(format.two_slot && outcome.loaded),
            "{}: a slot {what} loaded",
            format.name
        );
        loaded += usize::from(outcome.loaded);
    };
    // A two-slot format runs the damage once as it is and once re-sealed.
    let passes: &[bool] = if format.two_slot {
        &[false, true]
    } else {
        &[false]
    };
    for (index, octets) in saved.iter().enumerate() {
        let Some(octets) = octets else { continue };
        for &sealed in passes {
            let with = |mut damaged: Vec<u8>| {
                if sealed {
                    seal(&mut damaged);
                }
                let mut variant = saved.clone();
                variant[index] = Some(damaged);
                variant
            };
            for length in 0..octets.len() {
                let what = format!("cut to {length} octets");
                run(with(octets[..length].to_vec()), sealed, &what);
            }
            for bit in 0..octets.len() * 8 {
                // A damaged input that loads pays a synchronized save for
                // the round trip, so where loads happen (a one-file format,
                // a re-sealed slot) it takes one flip per octet, a different
                // bit in each; a slot as damaged takes all eight.
                if (sealed || !format.two_slot) && bit % 8 != (bit / 8) % 8 {
                    continue;
                }
                let mut damaged = octets.clone();
                damaged[bit / 8] ^= 1 << (bit % 8);
                run(with(damaged), sealed, &format!("with bit {bit} flipped"));
            }
        }
    }
    loaded
}

#[test]
fn damaged_files_never_panic_and_refused_restores_write_nothing() {
    for format in FORMATS {
        let loaded = damage(format);
        // Some damage must load: in the one-file formats, which have no
        // checksum, and in a re-sealed slot, a flip that lands on a value
        // that is still valid (a time, a flag, a count) or a cut at an entry
        // boundary that leaves a shorter list.
        assert!(loaded > 0, "{}: no damaged input loaded", format.name);
    }
}

/// The samples cover every format the crate saves: one per file backend
/// (the Color backend twice, once for each object type it serves, and the
/// write configuration backend sixteen times: a Timer's, a Channel's, two
/// intrinsic reporters', an Analog Input's and an Alert Enrollment's, which
/// takes fewer Notify_Types, kind 5 around the Timer's, the Channel's
/// and the Analog Input's, #2364, an Analog Input's written algorithm
/// parameters, kind 6, plain and under kind 5, a Multi-state Input's,
/// with the list row, the same two ways, an Access Door's, with its three
/// list rows, the same two ways, an Access Zone's, and an Event
/// Enrollment's, with its Event_Parameters and Fault_Parameters, plain and
/// under kind 5, #2331), and a written
/// Description in the log's, the forwarder's, the class's, Access Rights',
/// the Tags and the colour output's formats (#2364), and a Load Control's
/// shed request (#2479).
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
            "timer_write_config",
            "channel_write_config",
            "reporting_write_config",
            "alert_reporting_write_config",
            "described_timer_write_config",
            "described_channel_write_config",
            "described_reporting_write_config",
            "described_trend_log",
            "described_notification_forwarder",
            "described_notification_class",
            "described_access_rights",
            "described_tags",
            "described_color",
            "described_color_temperature",
            "parameters_reporting_write_config",
            "described_parameters_reporting_write_config",
            "alarm_values_reporting_write_config",
            "described_alarm_values_reporting_write_config",
            "door_lists_reporting_write_config",
            "described_door_lists_reporting_write_config",
            "zone_alarm_values_reporting_write_config",
            "enrollment_parameters_reporting_write_config",
            "described_enrollment_parameters_reporting_write_config",
            "state_text_reporting_write_config",
            "described_state_text_reporting_write_config",
            "load_control",
        ]
    );
}
