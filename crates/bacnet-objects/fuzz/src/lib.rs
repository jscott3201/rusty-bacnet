//! What the object crate's fuzz targets check: a restore from any octets,
//! through the file persistence backends (#2180).
//!
//! Each target writes its input into a format's files and runs that
//! format's restore check, `restore::Format::check`, shared with the
//! object crate's own corruption test (`tests/support/restore.rs`): no
//! panic; a loaded snapshot names its object and saves and loads back the
//! same; a load that fails fails the object's build; a refused build leaves
//! the files as they were; a log or Audit Log built from a snapshot serves
//! its counts. libFuzzer stands in for the rest: an allocation past its
//! limit (2,048 megabytes by default) or a stack overflow fails the target.
//!
//! | Target | Formats |
//! | --- | --- |
//! | `log_restore` | the Trend Log, Trend Log Multiple and Event Log snapshots (`log_persistence/codec.rs`) |
//! | `audit_restore` | the Audit Log snapshot, schema versions 1 to 3 |
//! | `object_file_restore` | the one-file formats: Notification Forwarder, Notification Class, Tags, the two colour objects' output, Access Rights |
//!
//! An input is a selector octet, which picks a format of the target's
//! (modulo their count), then the files. For a two-slot format: a flags
//! octet (bit 0 and bit 1: slot 0 and slot 1 exist; bit 2 and bit 3: seal
//! slot 0 and slot 1), a big-endian 16-bit length of slot 0, then slot 0's
//! octets and slot 1's. Sealing a slot of at least 30 octets writes its
//! length field and its CRC-32 for what it holds, so mutations reach the
//! payload decoders past the checksum. For a one-file format, the rest of
//! the input is the file, which exists unless the input ends at the
//! selector.

#[path = "../../tests/support/restore.rs"]
pub mod restore;

#[cfg(test)]
mod seeds;

use std::path::PathBuf;
use std::time::Duration;

use restore::{FORMATS, Format, Scratch, write_files};
pub use restore::{crc32, seal};

/// A fuzz target: which formats its selector picks from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The three log snapshots.
    Log,
    /// The Audit Log snapshot.
    Audit,
    /// The one-file formats.
    ObjectFile,
}

impl Target {
    /// Its formats, in selector order.
    pub fn formats(self) -> Vec<&'static Format> {
        FORMATS
            .iter()
            .filter(|format| match self {
                Self::Log => format.two_slot && format.name != "audit_log",
                Self::Audit => format.name == "audit_log",
                Self::ObjectFile => !format.two_slot,
            })
            .collect()
    }
}

/// The files `rest` holds for `format`, as the module docs lay them out.
fn files(format: &Format, rest: &[u8]) -> Vec<Option<Vec<u8>>> {
    if !format.two_slot {
        return vec![(!rest.is_empty()).then(|| rest.to_vec())];
    }
    let (flags, rest) = rest
        .split_first()
        .map_or((0, &[][..]), |(&flags, rest)| (flags, rest));
    let (split, rest) = match rest {
        [high, low, rest @ ..] => (usize::from(u16::from_be_bytes([*high, *low])), rest),
        _ => (0, rest),
    };
    let (first, second) = rest.split_at(split.min(rest.len()));
    [first, second]
        .into_iter()
        .enumerate()
        .map(|(slot, octets)| {
            (flags & (1 << slot) != 0).then(|| {
                let mut octets = octets.to_vec();
                if flags & (4 << slot) != 0 {
                    seal(&mut octets);
                }
                octets
            })
        })
        .collect()
}

/// A base path for `format` in this thread's scratch directory (the seed
/// tests run side by side). A test thread's directory goes when the thread
/// ends; libFuzzer exits without running that, so a new directory first
/// removes those earlier runs left more than an hour ago.
fn base(format: &Format) -> PathBuf {
    thread_local! {
        static SCRATCH: Scratch = {
            remove_stale_scratch();
            Scratch::new(SCRATCH_LABEL)
        };
    }
    SCRATCH.with(|scratch| scratch.base(format.name))
}

const SCRATCH_LABEL: &str = "objects-fuzz";

/// Remove scratch directories of earlier runs untouched for an hour. A
/// run still going rewrites its files on every input, so it stays.
fn remove_stale_scratch() {
    let prefix = format!("rusty-bacnet-{SCRATCH_LABEL}-");
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(3600));
        if stale && entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Restore `target`'s format picked by `input` from the files it holds.
pub fn check(target: Target, input: &[u8]) -> restore::Outcome {
    let formats = target.formats();
    let (selector, rest) = input
        .split_first()
        .map_or((0, &[][..]), |(&s, rest)| (s, rest));
    let format = formats[usize::from(selector) % formats.len()];
    let base = base(format);
    write_files(&format.files(&base), &files(format, rest));
    (format.check)(&base)
}
