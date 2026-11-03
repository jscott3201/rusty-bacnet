//! The file plumbing the one-file backends share (Notification Class,
//! Notification Forwarder, Tags, colour, Access Rights and written
//! configuration): one object's state in one file, replaced whole on each
//! save. The octets are the format's, from its codec in [`crate::snapshot`].

use crate::prelude::*;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use bacnet_types::error::Error;

use super::sync_parent_dir;

/// Test support, not a stable API: the octets a file backend keeps in
/// memory instead of on disk, so a fuzz target restores without the file
/// system (#2233). Clones share the octets.
///
/// Hand it to a backend's hidden `in_memory` constructor. A two-slot backend
/// (the logs' and the Audit Log's) keeps its slots in entries 0 and 1, a
/// one-file backend its file in entry 0; `None` is a missing file. Each
/// backend reads and writes the entries where it would read and write its
/// files, with the same size cap, and runs the same decode and encode on
/// them; only the file system calls are left out (open, read, write,
/// synchronize, rename).
#[cfg(feature = "test-support")]
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct MemoryFiles(alloc::sync::Arc<std::sync::Mutex<[Option<Vec<u8>>; 2]>>);

#[cfg(feature = "test-support")]
impl MemoryFiles {
    /// Storage holding `files`.
    pub fn new(files: [Option<Vec<u8>>; 2]) -> Self {
        Self(alloc::sync::Arc::new(std::sync::Mutex::new(files)))
    }

    /// What the entries hold now.
    pub fn files(&self) -> [Option<Vec<u8>>; 2] {
        self.lock().clone()
    }

    /// Entry `index`'s octets, at most `cap` of them, as a capped read of a
    /// file takes.
    pub(crate) fn read(&self, index: usize, cap: usize) -> Option<Vec<u8>> {
        self.lock()[index]
            .as_ref()
            .map(|octets| octets[..octets.len().min(cap)].to_vec())
    }

    /// Replace entry `index` with `octets`.
    pub(crate) fn write(&self, index: usize, octets: Vec<u8>) {
        self.lock()[index] = Some(octets);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, [Option<Vec<u8>>; 2]> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// One object's file: where it is, and the most octets a file of its format
/// may take. The format's codec (in [`crate::snapshot`]) reads and writes
/// the octets; this only moves them.
#[derive(Clone, Debug)]
pub(crate) struct ObjectFile {
    path: PathBuf,
    max_bytes: u64,
    /// Test support: keep the file here instead of at `path`.
    #[cfg(feature = "test-support")]
    memory: Option<MemoryFiles>,
}

impl ObjectFile {
    /// A file at `path` of at most `max_bytes`, that errors call `what` (an
    /// object type, then "file"). Refuses an empty path.
    pub(crate) fn new(path: &Path, max_bytes: u64, what: &'static str) -> Result<Self, Error> {
        if path.as_os_str().is_empty() {
            let kind = what.strip_suffix(" file").unwrap_or(what);
            return Err(Error::OutOfRange(format!(
                "{kind} persistence path must not be empty"
            )));
        }
        Ok(Self {
            path: path.to_path_buf(),
            max_bytes,
            #[cfg(feature = "test-support")]
            memory: None,
        })
    }

    /// Test support: keep the file in `memory`'s entry 0 instead of on disk.
    #[cfg(feature = "test-support")]
    pub(crate) fn keep_in(&mut self, memory: MemoryFiles) {
        self.memory = Some(memory);
    }

    /// The file's path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The file's octets, header and all, up to one past the size cap, or
    /// `None` when there is no file.
    pub(crate) fn read(&self) -> Result<Option<Vec<u8>>, Error> {
        #[cfg(feature = "test-support")]
        if let Some(memory) = &self.memory {
            return Ok(memory.read(0, (self.max_bytes + 1) as usize));
        }
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(self.max_bytes + 1).read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }

    /// Durably replace the file with `bytes`, header and all.
    ///
    /// A sibling `.tmp` file is written and synchronized, renamed over the
    /// old one, and then the directory is synchronized, so a failed save
    /// leaves the previous file in place and a completed one survives a power
    /// loss. Once the rename succeeds the save has landed: a directory sync
    /// that fails is logged, not returned (see [`sync_parent_dir`]).
    pub(crate) fn save_octets(&self, bytes: Vec<u8>) -> Result<(), Error> {
        #[cfg(feature = "test-support")]
        if let Some(memory) = &self.memory {
            memory.write(0, bytes);
            return Ok(());
        }
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let temporary = self.temporary_path();
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &self.path)?;
        // The new state is in place, so the save has landed whatever the
        // directory sync finds.
        sync_parent_dir(&self.path);
        Ok(())
    }

    fn temporary_path(&self) -> PathBuf {
        let mut temporary = self.path.as_os_str().to_os_string();
        temporary.push(".tmp");
        PathBuf::from(temporary)
    }
}
