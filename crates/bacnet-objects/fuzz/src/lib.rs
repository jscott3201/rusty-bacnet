//! What the object crate's fuzz targets check: a restore from any octets,
//! through the persistence backends (#2180).
//!
//! Each target hands its input to a format's backend as that format's files
//! and runs the format's restore check, `restore::Format::check`, shared
//! with the object crate's own corruption test (`tests/support/restore.rs`):
//! no panic; a loaded snapshot names its object and saves and loads back the
//! same; a load that fails fails the object's build; a refused build leaves
//! the storage as it was; a log or Audit Log built from a snapshot serves
//! its counts. libFuzzer stands in for the rest: an allocation past its
//! limit (2,048 megabytes by default) or a stack overflow fails the target.
//! The targets build without a sanitizer (`scripts/ci/fuzz.sh`, #2233), and
//! libFuzzer's per-allocation limit needs one, so [`Capped`], this crate's
//! global allocator, keeps it.
//!
//! The main targets run the file backends in their in-memory mode
//! (`bacnet-objects`' hidden `test-support` feature, #2233): the same load,
//! slot selection, decode, encode and save code, with the octets in memory,
//! so a run makes no file system call. Each has a `*_file` twin that writes
//! the input into real files and restores from them, much slower, for the
//! half of the backends the in-memory mode leaves out: opening, reading and
//! writing real files, and a refused restore leaving them on disk as they
//! were. The twin also runs the in-memory check on the same
//! input and fails unless both come to the same outcome.
//!
//! | Target | Formats |
//! | --- | --- |
//! | `log_restore`, `log_restore_file` | the Trend Log, Trend Log Multiple and Event Log snapshots, schema versions 1 and 2 (`log_persistence/codec.rs`) |
//! | `audit_restore`, `audit_restore_file` | the Audit Log snapshot, schema versions 1 to 3 |
//! | `object_file_restore`, `object_file_restore_file` | the one-file formats: Notification Forwarder, Notification Class, Tags, the two colour objects' output, Access Rights, the Timer's and Channel's write configuration, and an Analog Input's and an Alert Enrollment's written reporting settings |
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

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::time::Duration;

use bacnet_objects::access_control::FileAccessRightsPersistence;
use bacnet_objects::audit::FileAuditLogPersistence;
use bacnet_objects::color::FileColorOutputPersistence;
use bacnet_objects::durable::MemoryFiles;
use bacnet_objects::log_persistence::FileLogPersistence;
use bacnet_objects::notification_class::FileNotificationClassPersistence;
use bacnet_objects::notification_forwarder::FileNotificationForwarderPersistence;
use bacnet_objects::object_profile::FileTagsPersistence;
use bacnet_objects::write_config::FileWriteConfigPersistence;
use restore::{FORMATS, Files, Format, Medium, Outcome, Scratch, write_files};
pub use restore::{crc32, seal};

/// libFuzzer's default limit on one allocation (`-malloc_limit_mb`, which
/// defaults to `-rss_limit_mb`, 2,048).
const ALLOCATION_LIMIT: usize = 2048 << 20;

/// The system allocator, aborting on a request of [`ALLOCATION_LIMIT`] or
/// more, as libFuzzer does.
/// libFuzzer enforces `-malloc_limit_mb` through a sanitizer's malloc
/// hooks, which a build without a sanitizer lacks; without this, a crafted
/// length could reserve gigabytes it never touches and pass unnoticed. The
/// abort is a crash, so libFuzzer saves the input.
pub struct Capped;

impl Capped {
    fn check(size: usize) {
        if size >= ALLOCATION_LIMIT {
            use std::io::Write;
            // No formatting: nothing here may allocate.
            let _ = std::io::stderr()
                .write_all(b"objects-fuzz: an allocation reached the 2,048 MB limit\n");
            std::process::abort();
        }
    }
}

// SAFETY: every call goes to `System` unchanged once the size is checked.
unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Self::check(layout.size());
        // SAFETY: the caller's contract for `alloc`, passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        Self::check(layout.size());
        // SAFETY: as for `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System` through this allocator.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        Self::check(new_size);
        // SAFETY: as for `dealloc`, with the caller's `realloc` contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Capped = Capped;

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
        self.iter().collect()
    }

    /// Its formats, in selector order, without collecting them: [`parse`]
    /// runs on every input.
    fn iter(self) -> impl Iterator<Item = &'static Format> + Clone {
        FORMATS.iter().filter(move |format| match self {
            Self::Log => format.two_slot && format.name != "audit_log",
            Self::Audit => format.name == "audit_log",
            Self::ObjectFile => !format.two_slot,
        })
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

/// A format's files in memory: the backends' in-memory mode, entry 0 (and
/// for a two-slot format entry 1) holding them.
struct Memory {
    files: MemoryFiles,
    count: usize,
}

impl Memory {
    fn new(files: Vec<Option<Vec<u8>>>) -> Self {
        let count = files.len();
        let mut entries = files.into_iter();
        Self {
            files: MemoryFiles::new([entries.next().flatten(), entries.next().flatten()]),
            count,
        }
    }
}

impl Medium for Memory {
    fn log(&self) -> FileLogPersistence {
        FileLogPersistence::in_memory(self.files.clone())
    }
    fn audit(&self) -> FileAuditLogPersistence {
        FileAuditLogPersistence::in_memory(self.files.clone())
    }
    fn forwarder(&self) -> FileNotificationForwarderPersistence {
        FileNotificationForwarderPersistence::in_memory(self.files.clone())
    }
    fn class(&self) -> FileNotificationClassPersistence {
        FileNotificationClassPersistence::in_memory(self.files.clone())
    }
    fn tags(&self) -> FileTagsPersistence {
        FileTagsPersistence::in_memory(self.files.clone())
    }
    fn color(&self) -> FileColorOutputPersistence {
        FileColorOutputPersistence::in_memory(self.files.clone())
    }
    fn rights(&self) -> FileAccessRightsPersistence {
        FileAccessRightsPersistence::in_memory(self.files.clone())
    }
    fn write_config(&self) -> FileWriteConfigPersistence {
        FileWriteConfigPersistence::in_memory(self.files.clone())
    }
    fn fresh(&self) -> Self {
        Self::new(vec![None; self.count])
    }
    fn contents(&self) -> Vec<Option<Vec<u8>>> {
        // One copy of the octets, not two: the check takes this on every
        // input.
        self.files.files().into_iter().take(self.count).collect()
    }
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

/// `target`'s format that `input` picks, and the files it holds.
fn parse(target: Target, input: &[u8]) -> (&'static Format, Vec<Option<Vec<u8>>>) {
    let mut formats = target.iter();
    let (selector, rest) = input
        .split_first()
        .map_or((0, &[][..]), |(&s, rest)| (s, rest));
    let count = formats.clone().count();
    let format = formats
        .nth(usize::from(selector) % count)
        .expect("a selector modulo the format count");
    (format, files(format, rest))
}

/// Restore `target`'s format picked by `input` from the files it holds,
/// kept in memory.
pub fn check(target: Target, input: &[u8]) -> Outcome {
    let (format, files) = parse(target, input);
    format.check(&Memory::new(files))
}

/// The same restore from real files, in this thread's scratch directory,
/// which must come to the same outcome as [`check`] and leave the storage
/// holding the same octets. Nothing the check builds reads a clock (the
/// objects get none), so both runs write the same octets.
pub fn check_files(target: Target, input: &[u8]) -> Outcome {
    let (format, files) = parse(target, input);
    let base = base(format);
    write_files(&format.files(&base), &files);
    let on_disk = Files::new(format, &base);
    let outcome = format.check(&on_disk);
    let memory = Memory::new(files);
    assert_eq!(
        outcome,
        format.check(&memory),
        "{}: the files and memory restore differently",
        format.name
    );
    assert!(
        on_disk.contents() == memory.contents(),
        "{}: the files and memory hold different octets after the restore",
        format.name
    );
    outcome
}
