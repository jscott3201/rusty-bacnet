//! The storage of the Device a builder builds (#2730): what a peer writes to
//! it, kept in a file the builder is given, so it survives a restart.
//!
//! The builders take a path ([`ServerBuilder::device_storage_path`]), read
//! only when they build the Device from the identity, and a start checks
//! none is left: a database the application hands over brings its Device's
//! storage itself ([`DeviceObject::with_tags_persistence`]). The file is a
//! [`FileTagsPersistence`], whose errors name its path.
//!
//! [`DeviceObject::with_tags_persistence`]: bacnet_objects::device::DeviceObject::with_tags_persistence

use std::path::PathBuf;

use bacnet_objects::DeviceIdentity;
use bacnet_objects::object_profile::FileTagsPersistence;

use super::*;

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Keep what peers write to the Device this builder builds in the file
    /// at `path` (#2730): a written Description and, with
    /// [`clock_zone_writable`](Self::clock_zone_writable) on, a written
    /// UTC_Offset and Daylight_Savings_Status. Each such write is saved
    /// before it is answered, and the next server built on the same path
    /// serves it again. None set by default: the writes are answered and
    /// served until the server stops, then lost. The built Device has no
    /// Tags row, so a Tags write is refused either way.
    ///
    /// A relative `path` is resolved against the current directory here,
    /// so a later change of directory doesn't move the saves. The builder
    /// reads it only when it builds the Device, from its
    /// [`device`](Self::device) identity: a [`database`](Self::database)
    /// given as well is refused at build, since that database's Device
    /// brings its own storage. A missing file, or a missing directory,
    /// starts the Device with nothing kept; the first save creates them.
    /// A file that can't be read, is damaged, belongs to another Device or
    /// was written by a newer release fails the build, naming the path and
    /// the reason, and is left as it is. A write whose save fails is
    /// refused with DEVICE / OPERATIONAL_PROBLEM, the old value still
    /// served, and the save's error is logged as a warning naming the path.
    /// One server at a time may use a path.
    pub fn device_storage_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.device_storage_path = Some(absolute(path.into()));
        self
    }
}

impl BipServerBuilder {
    /// Keep what peers write to the built Device in the file at `path`; see
    /// [`ServerBuilder::device_storage_path`].
    pub fn device_storage_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.device_storage_path = Some(absolute(path.into()));
        self
    }
}

#[cfg(feature = "sc-tls")]
impl super::ScServerBuilder {
    /// Keep what peers write to the built Device in the file at `path`; see
    /// [`ServerBuilder::device_storage_path`].
    pub fn device_storage_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.device_storage_path = Some(absolute(path.into()));
        self
    }
}

/// `path` against the current directory, if it is relative; as given when
/// it can't be (an empty path, which the build refuses, or no current
/// directory).
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

/// The stock database `identity` declares, its Device kept at the path
/// `config` names, which this takes: what the builders build when they are
/// given no database. A start refuses a path left in its configuration
/// ([`refuse_unused_path`]).
pub(crate) fn stock_database(
    config: &mut ServerConfig,
    identity: &DeviceIdentity,
) -> Result<ObjectDatabase, Error> {
    match config.device_storage_path.take() {
        Some(path) => bacnet_objects::device_identity::build_database_with_tags_persistence(
            identity,
            Arc::new(FileTagsPersistence::new(path)?),
            Vec::new(),
        ),
        None => identity.build_database(),
    }
}

/// Refuse a Device storage path a builder didn't take: one set beside
/// `database`, the one a builder was given or the one a start was, whose
/// Device brings its own storage.
pub(crate) fn refuse_unused_path(config: &ServerConfig, database: &str) -> Result<(), Error> {
    match &config.device_storage_path {
        Some(path) => Err(Error::InvalidConfig(bacnet_types::error::Refusal::text(
            format!(
                "device_storage_path {} is for the Device a builder builds from its identity; \
                 {database} brings its Device's own storage (DeviceObject::with_tags_persistence)",
                path.display()
            ),
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "device_storage_tests.rs"]
mod tests;
