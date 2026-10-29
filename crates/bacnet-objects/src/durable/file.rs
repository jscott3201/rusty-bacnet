//! The file plumbing the Notification Forwarder, Notification Class and
//! Access Rights backends share: one object's state in one file, replaced
//! whole on each save.
//!
//! Each file starts with an eight-octet magic tag naming its format and
//! version, then the four-octet object identifier it belongs to. What
//! follows is the format's own body.

use crate::prelude::*;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::sync_parent_dir;

/// The magic tag and the object identifier.
const HEADER_LEN: usize = 8 + 4;

/// One object's file: where it is, the format it holds, and the most octets
/// a file of that format may take.
#[derive(Clone, Debug)]
pub(crate) struct ObjectFile {
    path: PathBuf,
    magic: &'static [u8; 8],
    max_bytes: u64,
    /// The object type the file belongs to, for errors.
    kind: &'static str,
}

impl ObjectFile {
    /// A file at `path` holding `magic`-tagged state of a `kind` object, at
    /// most `max_bytes` long. Refuses an empty path.
    pub(crate) fn new(
        path: &Path,
        magic: &'static [u8; 8],
        max_bytes: u64,
        kind: &'static str,
    ) -> Result<Self, Error> {
        if path.as_os_str().is_empty() {
            return Err(Error::OutOfRange(format!(
                "{kind} persistence path must not be empty"
            )));
        }
        Ok(Self {
            path: path.to_path_buf(),
            magic,
            max_bytes,
            kind,
        })
    }

    /// The file's path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The error for a file that holds something this format cannot load.
    pub(crate) fn corrupt(&self, reason: &str) -> Error {
        Error::Encoding(format!("{} file {reason}", self.kind))
    }

    /// The body saved for `oid`, after the header, or `None` when there is
    /// no file. Refuses a file past the size cap before decoding any of it,
    /// one without this format's header, and one saved for another object.
    pub(crate) fn load(&self, oid: ObjectIdentifier) -> Result<Option<Vec<u8>>, Error> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(self.max_bytes + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > self.max_bytes {
            return Err(self.corrupt("is too large"));
        }
        if bytes.len() < HEADER_LEN || &bytes[..self.magic.len()] != self.magic {
            return Err(self.corrupt("has no valid header"));
        }
        if bytes[self.magic.len()..HEADER_LEN] != oid.encode() {
            return Err(self.corrupt("belongs to another object"));
        }
        bytes.drain(..HEADER_LEN);
        Ok(Some(bytes))
    }

    /// Durably replace the file with `body` saved for `oid`.
    ///
    /// A sibling `.tmp` file is written and synchronized, renamed over the
    /// old one, and then the directory is synchronized, so a failed save
    /// leaves the previous file in place and a completed one survives a power
    /// loss. Once the rename succeeds the save has landed: a directory sync
    /// that fails is logged, not returned (see [`sync_parent_dir`]).
    pub(crate) fn save(&self, oid: ObjectIdentifier, body: &[u8]) -> Result<(), Error> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + body.len());
        bytes.extend_from_slice(self.magic);
        bytes.extend_from_slice(&oid.encode());
        bytes.extend_from_slice(body);
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

    /// Decode `bytes` as one element after another with `decode`, refusing
    /// more than `cap` of them or one that does not decode.
    pub(crate) fn decode_capped<T>(
        &self,
        bytes: &[u8],
        cap: usize,
        decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
    ) -> Result<Vec<T>, Error> {
        let mut list = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            if list.len() == cap {
                return Err(self.corrupt("holds more entries than the cap"));
            }
            let (element, next) = decode(bytes, offset)
                .map_err(|_| self.corrupt("holds an entry that does not decode"))?;
            list.push(element);
            offset = next;
        }
        Ok(list)
    }

    fn temporary_path(&self) -> PathBuf {
        let mut temporary = self.path.as_os_str().to_os_string();
        temporary.push(".tmp");
        PathBuf::from(temporary)
    }
}
