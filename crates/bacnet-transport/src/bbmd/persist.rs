//! The persisted-BDT file a BBMD starts from
//! ([`BipTransport::set_bdt_persist_path`](crate::bip::BipTransport::set_bdt_persist_path)),
//! and its writer: a BDT activated through the Network Port object is saved
//! there, so a restart keeps it (#1826).
//!
//! The file holds the BDT rows as a Write-BDT payload carries them, 10
//! octets each, as the loader reads it. Since #940 a save adds an 11-octet
//! trailer after them with the NAT setting the Network Port activated:
//! `NAT1`, then 1 when NAT traversal is on and 0 when it is off, then the
//! global address (IPv4 and UDP port, all zero when none). Rows are a
//! multiple of 10 octets, so the trailer is told apart by the length, and a
//! file without one, from before #940, loads as it always did. A write goes to a temporary file
//! beside it, is synced, and then renamed over it, so a reader or a crash
//! sees the old table or the new one, never part of one; on Unix the
//! directory is synced too, so the rename itself survives a crash.
//!
//! An activation runs under the server's database lock and the BBMD lock,
//! so the file I/O happens after both are released, on a thread of its own
//! (a plain thread, which a Tokio runtime's shutdown doesn't cancel):
//! the activation takes a generation for its table under the BBMD lock
//! ([`BdtFile::next_generation`]), and a save older than one already
//! written is skipped, so a slow save of an older table never overwrites a
//! newer one. A failed write is logged and counted, and leaves the
//! activation in place: the BBMD runs on the new table either way, and only
//! a restart would lose it.

use std::io::Write as _;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bytes::BytesMut;
use tracing::{debug, warn};

use bacnet_types::error::Error;

use super::{encode_bdt_entries, BbmdState, BdtEntry};

/// The NAT setting saved with the BDT: whether NAT traversal is on, and
/// the global address, kept while it is off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PersistedNat {
    pub(crate) enabled: bool,
    pub(crate) global: Option<SocketAddrV4>,
}

/// The trailer's tag.
const NAT_TAG: &[u8; 4] = b"NAT1";
/// The trailer's length.
const NAT_TRAILER: usize = 11;

impl PersistedNat {
    fn encode(self, out: &mut BytesMut) {
        out.extend_from_slice(NAT_TAG);
        out.extend_from_slice(&[u8::from(self.enabled)]);
        let global = self
            .global
            .unwrap_or(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
        out.extend_from_slice(&global.ip().octets());
        out.extend_from_slice(&global.port().to_be_bytes());
    }

    fn decode(trailer: &[u8]) -> Option<Self> {
        let [t0, t1, t2, t3, enabled, a, b, c, d, high, low] =
            <[u8; NAT_TRAILER]>::try_from(trailer).ok()?;
        if [t0, t1, t2, t3] != *NAT_TAG || enabled > 1 {
            return None;
        }
        let global = SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), u16::from_be_bytes([high, low]));
        Some(Self {
            enabled: enabled == 1,
            global: (!global.ip().is_unspecified() || global.port() != 0).then_some(global),
        })
    }
}

/// A persisted-BDT file's rows and, when it has the trailer, its NAT
/// setting.
pub(crate) fn decode_file(data: &[u8]) -> Result<(Vec<BdtEntry>, Option<PersistedNat>), Error> {
    if data.len() % 10 == NAT_TRAILER % 10 && data.len() >= NAT_TRAILER {
        let (rows, trailer) = data.split_at(data.len() - NAT_TRAILER);
        let nat = PersistedNat::decode(trailer).ok_or_else(|| {
            Error::Encoding("persisted BDT: unknown trailer after the rows".into())
        })?;
        return Ok((BbmdState::decode_bdt(rows)?, Some(nat)));
    }
    Ok((BbmdState::decode_bdt(data)?, None))
}

/// One BBMD's persisted-BDT file.
#[derive(Debug)]
pub(crate) struct BdtFile {
    path: PathBuf,
    /// The newest generation handed out ([`Self::next_generation`]).
    generation: AtomicU64,
    /// Held across a save: the newest generation a save was made for.
    written: Mutex<u64>,
    /// Saves that failed.
    failures: AtomicU64,
    /// Saves that finished, written, failed or skipped.
    finished: AtomicU64,
}

impl BdtFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            generation: AtomicU64::new(0),
            written: Mutex::new(0),
            failures: AtomicU64::new(0),
            finished: AtomicU64::new(0),
        }
    }

    /// Where the file is.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// How many saves have failed.
    pub(crate) fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }

    /// How many saves have finished, whatever came of them.
    #[cfg(test)]
    pub(crate) fn finished(&self) -> u64 {
        self.finished.load(Ordering::Acquire)
    }

    /// The generation of a table about to be saved. Take it under the BBMD
    /// lock, with the table, so generations follow the order the tables
    /// went into effect.
    pub(crate) fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Save `rows`, of `generation`, off the caller's thread. Inside a
    /// Tokio context that is a plain thread of its own, which the runtime's
    /// shutdown can't cancel, so a WARMSTART followed at once by a stop
    /// still saves. Outside one (a caller with no runtime) the save runs
    /// here, under whatever locks the caller holds. A save that never runs
    /// or panics is counted as failed ([`PendingSave`]).
    pub(crate) fn save_later(
        self: &Arc<Self>,
        generation: u64,
        rows: Vec<BdtEntry>,
        nat: Option<PersistedNat>,
    ) {
        let pending = self.pending_save();
        let save = move || pending.run_with(generation, &rows, nat);
        if tokio::runtime::Handle::try_current().is_err() {
            return save();
        }
        // A thread that can't be spawned drops `save` unrun, which counts.
        let _ = std::thread::Builder::new()
            .name("bacnet-bdt-save".into())
            .spawn(save);
    }

    /// A save about to be handed off: it counts as failed unless it runs to
    /// the end.
    pub(crate) fn pending_save(self: &Arc<Self>) -> PendingSave {
        PendingSave {
            file: Some(Arc::clone(self)),
        }
    }

    /// Write `rows`, of `generation`, to the file, replacing it whole,
    /// unless a newer generation was already written; a failure is logged
    /// and counted, and doesn't count as written, so an older save may
    /// still land.
    #[cfg(test)]
    pub(crate) fn save(&self, generation: u64, rows: &[BdtEntry]) {
        self.save_with(generation, rows, None);
    }

    /// Write `rows`, of `generation`, with the NAT setting in the trailer
    /// when given, as described on `save`.
    pub(crate) fn save_with(&self, generation: u64, rows: &[BdtEntry], nat: Option<PersistedNat>) {
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if generation <= *written {
            debug!(generation, newest = *written, "Skipped saving an older BDT");
        } else {
            match self.write(rows, nat) {
                Ok(()) => {
                    *written = generation;
                    debug!(
                        path = %self.path.display(),
                        entries = rows.len(),
                        "Saved the activated BDT"
                    );
                }
                Err(error) => self.failed(&error),
            }
        }
        drop(written);
        self.finished.fetch_add(1, Ordering::Release);
    }

    fn failed(&self, why: &dyn std::fmt::Display) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        warn!(
            path = %self.path.display(),
            error = %why,
            "Failed to save the activated BDT; the BBMD uses it, but a restart will not"
        );
    }

    fn write(&self, rows: &[BdtEntry], nat: Option<PersistedNat>) -> std::io::Result<()> {
        static TEMPORARY: AtomicU64 = AtomicU64::new(0);
        let mut octets = BytesMut::new();
        encode_bdt_entries(rows, &mut octets);
        if let Some(nat) = nat {
            nat.encode(&mut octets);
        }
        let name = self.path.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "the path names no file")
        })?;
        // Unique to this process and save, so two writers of one path never
        // share a temporary file.
        let mut temporary = name.to_owned();
        temporary.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = self.path.with_file_name(temporary);
        let written = (|| {
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(&octets)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &self.path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written?;
        sync_directory(&self.path);
        Ok(())
    }
}

/// A save handed to another thread. Dropped without having run, because
/// the thread never started or the save panicked, it counts as a failed
/// save, so no activated table is lost silently.
pub(crate) struct PendingSave {
    file: Option<Arc<BdtFile>>,
}

impl PendingSave {
    /// Run the save; it then no longer counts as lost.
    #[cfg(test)]
    pub(crate) fn run(self, generation: u64, rows: &[BdtEntry]) {
        self.run_with(generation, rows, None);
    }

    /// Run the save, with the NAT setting in the trailer when given.
    pub(crate) fn run_with(
        mut self,
        generation: u64,
        rows: &[BdtEntry],
        nat: Option<PersistedNat>,
    ) {
        if let Some(file) = &self.file {
            file.save_with(generation, rows, nat);
        }
        self.file = None;
    }
}

impl Drop for PendingSave {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            file.failed(&"the save never ran to completion");
            file.finished.fetch_add(1, Ordering::Release);
        }
    }
}

/// Sync the directory holding `path`, so a rename into it survives a crash;
/// best effort.
#[cfg(unix)]
fn sync_directory(path: &Path) {
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if let Err(error) = std::fs::File::open(directory).and_then(|dir| dir.sync_all()) {
        debug!(%error, "Could not sync the persisted BDT's directory");
    }
}

/// Other platforms offer no portable directory sync; skipped there.
#[cfg(not(unix))]
fn sync_directory(_path: &Path) {}
