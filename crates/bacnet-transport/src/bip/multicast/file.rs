//! The persisted B/IP multicast group
//! ([`BipTransport::set_multicast_persist_path`](crate::bip::BipTransport::set_multicast_persist_path),
//! #1954): a group activated through the Network Port object is saved, so
//! a restart keeps it, as Clause 12.56 expects of the values a device needs
//! to operate.
//!
//! It is a file of its own rather than a trailer in the persisted-BDT
//! file: that file tells its one NAT trailer apart by length, with no room
//! for a second, and only a BBMD has one, while a plain member's group needs
//! keeping too. The file holds 11 octets: `GRP1`, the group's IPv4 address
//! (0.0.0.0 when the activation turned multicast off), the multicast TTL,
//! and the group's UDP port (0 for the transport's own). A save replaces it
//! whole through a temporary file and a rename ([`write_atomically`]), on a
//! thread of its own after the activation has released its locks; a save
//! older than one already written is skipped, and a failed one is logged
//! and counted, leaving the activation in place.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bacnet_types::error::Error;
use tracing::{debug, warn};

use super::BipMulticast;
use crate::bbmd::write_atomically;

/// The file's tag.
const TAG: &[u8; 4] = b"GRP1";
/// The file's length.
const LENGTH: usize = 11;

/// What the file holds: the group in effect, or none, and the TTL and port
/// a group written to the Network Port gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PersistedGroup {
    pub(crate) group: Option<Ipv4Addr>,
    pub(crate) ttl: u8,
    pub(crate) port: Option<u16>,
}

impl PersistedGroup {
    fn encode(self) -> [u8; LENGTH] {
        let mut out = [0; LENGTH];
        out[..4].copy_from_slice(TAG);
        out[4..8].copy_from_slice(&self.group.unwrap_or(Ipv4Addr::UNSPECIFIED).octets());
        out[8] = self.ttl;
        out[9..].copy_from_slice(&self.port.unwrap_or(0).to_be_bytes());
        out
    }

    fn decode(data: &[u8]) -> Result<Self, Error> {
        let invalid = |why: &str| Error::Encoding(format!("persisted B/IP multicast group: {why}"));
        let [t0, t1, t2, t3, a, b, c, d, ttl, high, low] =
            <[u8; LENGTH]>::try_from(data).map_err(|_| invalid("wrong length"))?;
        if [t0, t1, t2, t3] != *TAG {
            return Err(invalid("unknown tag"));
        }
        let group = Ipv4Addr::new(a, b, c, d);
        let port = u16::from_be_bytes([high, low]);
        let persisted = Self {
            group: (!group.is_unspecified()).then_some(group),
            ttl,
            port: (port != 0).then_some(port),
        };
        // The group, TTL and port must make a group the setters accept;
        // with none, the TTL still must be one they would.
        let usable = match persisted.group {
            Some(group) => persisted.multicast_with(group).is_ok(),
            None => ttl != 0,
        };
        if !usable {
            return Err(invalid("not a usable group"));
        }
        Ok(persisted)
    }

    /// The group with this TTL and port, at `group`.
    fn multicast_with(self, group: Ipv4Addr) -> Result<BipMulticast, Error> {
        let multicast = BipMulticast::new(group)?.with_ttl(self.ttl)?;
        match self.port {
            Some(port) => multicast.with_port(port),
            None => Ok(multicast),
        }
    }

    /// The group in effect, if any.
    pub(crate) fn multicast(self) -> Option<BipMulticast> {
        self.group.and_then(|group| self.multicast_with(group).ok())
    }
}

/// One transport's persisted-group file.
#[derive(Debug)]
pub(crate) struct GroupFile {
    path: PathBuf,
    /// The newest generation handed out.
    generation: AtomicU64,
    /// Held across a save: the newest generation written.
    written: Mutex<u64>,
    /// Saves that failed.
    failures: AtomicU64,
    /// Saves that finished, whatever came of them.
    finished: AtomicU64,
}

impl GroupFile {
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

    /// How many saves have finished, whatever came of them; for the tests
    /// in multicast/persist_tests.rs, which run where loopback multicast
    /// does.
    #[cfg(all(test, any(target_os = "linux", target_vendor = "apple")))]
    pub(crate) fn finished(&self) -> u64 {
        self.finished.load(Ordering::Acquire)
    }

    /// What the file holds; `None` when there is no file yet.
    pub(crate) fn load(&self) -> Result<Option<PersistedGroup>, Error> {
        match std::fs::read(&self.path) {
            Ok(data) => PersistedGroup::decode(&data).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::Transport(error)),
        }
    }

    /// The generation of a group about to be saved. Take it under the
    /// lock the group changed under, so generations follow the order the
    /// groups went into effect.
    pub(crate) fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Save `group`, of `generation`, off the caller's thread: inside a
    /// Tokio context on a plain thread of its own, which a runtime's
    /// shutdown can't cancel; outside one, here. A save that never runs is
    /// counted as failed.
    pub(crate) fn save_later(self: &Arc<Self>, generation: u64, group: PersistedGroup) {
        let pending = Pending(Some(Arc::clone(self)));
        let save = move || pending.run(generation, group);
        if tokio::runtime::Handle::try_current().is_err() {
            return save();
        }
        // A thread that can't be spawned drops `save` unrun, which counts.
        let _ = std::thread::Builder::new()
            .name("bacnet-group-save".into())
            .spawn(save);
    }

    fn save(&self, generation: u64, group: PersistedGroup) {
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        // A failed save doesn't count as written, so an older save still
        // pending may land after it: the file then holds an older group
        // than the one in use, and the failure was warned and counted.
        if generation > *written {
            match write_atomically(&self.path, &group.encode()) {
                Ok(()) => {
                    *written = generation;
                    debug!(path = %self.path.display(), ?group, "Saved the activated B/IP multicast group");
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
            "Failed to save the activated B/IP multicast group; the transport uses it, but a \
             restart will not"
        );
    }
}

/// A save handed to another thread; dropped unrun, it counts as failed.
struct Pending(Option<Arc<GroupFile>>);

impl Pending {
    fn run(mut self, generation: u64, group: PersistedGroup) {
        if let Some(file) = self.0.take() {
            file.save(generation, group);
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(file) = self.0.take() {
            file.failed(&"the save never ran to completion");
            file.finished.fetch_add(1, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_round_trips_and_a_bad_file_is_refused() {
        let group = PersistedGroup {
            group: Some(Ipv4Addr::new(239, 1, 2, 3)),
            ttl: 4,
            port: Some(47809),
        };
        let encoded = group.encode();
        assert_eq!(&encoded[..4], b"GRP1");
        assert_eq!(PersistedGroup::decode(&encoded).unwrap(), group);
        let off = PersistedGroup {
            group: None,
            ttl: 1,
            port: None,
        };
        assert_eq!(PersistedGroup::decode(&off.encode()).unwrap(), off);
        assert_eq!(off.multicast(), None);
        let mut unicast = encoded;
        unicast[4..8].copy_from_slice(&[10, 0, 0, 1]);
        let mut zero_ttl = encoded;
        zero_ttl[8] = 0;
        let mut tag = encoded;
        tag[3] = b'2';
        for bad in [&unicast[..], &zero_ttl[..], &tag[..], &encoded[..10]] {
            assert!(PersistedGroup::decode(bad).is_err(), "{bad:?}");
        }
    }
}
