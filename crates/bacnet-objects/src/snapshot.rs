//! The persisted snapshot formats, for storage an application brings
//! (#2047).
//!
//! Each durable object keeps its state through a storage trait, such as
//! [`LogPersistence`] or [`WriteConfigPersistence`], and the hosted build
//! ships a file backend for each. A device without std, or with storage of
//! its own (flash, EEPROM, a database row), implements the trait itself.
//! This module gives it the same octets the file backends write, so state
//! saved by one can be loaded by the other, and a backend needn't invent a
//! format. It needs only `alloc`.
//!
//! Each format has a module with `encode`, which returns the octets to
//! store, and `decode`, which takes them back, plus its magic tag and
//! version as constants. Both refuse with a [`SnapshotError`], whose
//! [`kind`](SnapshotError::kind) says why: octets cut short, a version or
//! kind this build doesn't read, a snapshot of another object, or a value
//! no save writes, treated as damage. `decode` checks everything `encode`
//! does, so whatever loads saves again.
//!
//! A backend that keeps two copies, as the log and Audit Log file backends
//! keep two slots, gets the file backend's choice between them from
//! [`log::newest`] and [`audit_log::newest`].
//!
//! | Module | Snapshot | Tag | Versions read (written) | File backend |
//! | --- | --- | --- | --- | --- |
//! | [`log`] | [`LogSnapshot`] | `RBLOGS01` | 1, 2, 3 (2, 3) | `FileLogPersistence`, one slot of two |
//! | [`audit_log`] | [`AuditLogSnapshot`] | `RBALOG01` | 1, 2, 3 (3) | `FileAuditLogPersistence`, one slot of two |
//! | [`write_config`] | [`WriteConfigSnapshot`] | `RBNWCF01` | kinds 1 to 6 (2 to 6) | `FileWriteConfigPersistence` |
//! | [`notification_class`] | [`NotificationClassSnapshot`] | `RBNNCL01` | 1, 2 (1, 2) | `FileNotificationClassPersistence` |
//! | [`notification_forwarder`] | [`ForwarderSnapshot`] | `RBNFWD01` | 1, 2 (1, 2) | `FileNotificationForwarderPersistence` |
//! | [`tags`] | [`TagsSnapshot`] | `RBNTAG01` | 1, 2 (1, 2) | `FileTagsPersistence` |
//! | [`color`] | [`ColorOutputSnapshot`] | `RBNCLR02` | 1 (`RBNCLR01`), 2, 3 (2, 3) | `FileColorOutputPersistence` |
//! | [`access_rights`] | [`AccessRightsSnapshot`] | `RBNACR01` | 1, 2 (1, 2) | `FileAccessRightsPersistence` |
//!
//! A written Description (#2364) takes the newest version (or kind) in each
//! row it changed: log schema version 3, write configuration kind 5, the
//! colour output's version 3 and the other one-file formats' version 2.
//! Only a snapshot holding one is written that
//! way, so a snapshot without one keeps the octets an earlier release wrote,
//! and a release that predates the version refuses one with it as
//! [`UnknownVersion`](SnapshotErrorKind::UnknownVersion) or
//! [`UnknownKind`](SnapshotErrorKind::UnknownKind). A written event
//! algorithm parameter (#2331) takes write configuration kind 6 the same
//! way.
//!
//! The one-file formats share a header: the eight-octet tag, which names the
//! format and its version, then the four-octet identifier of the object the
//! file belongs to. Their `encode` and `decode` take that object first. A
//! newer version of a format's tag, or a kind this build doesn't know, is
//! [`UnknownVersion`](SnapshotErrorKind::UnknownVersion) or
//! [`UnknownKind`](SnapshotErrorKind::UnknownKind), never damage.
//!
//! # Stability
//!
//! The promise covers the octets: a later release keeps loading what an
//! earlier release wrote. A format may gain versions and kinds, and a
//! backend that stores the octets as they are needs no change for that. A
//! format version stops loading only in a release whose changelog carries a
//! migration note saying so. A snapshot an older release can't read (a
//! newer version or kind) is refused with
//! [`UnknownVersion`](SnapshotErrorKind::UnknownVersion) or
//! [`UnknownKind`](SnapshotErrorKind::UnknownKind) rather than loaded in
//! part.
//!
//! The Rust snapshot types are ordinary API and follow the crate's usual
//! rules: a field added to one is a breaking change to the Rust API, even
//! when the octets stay readable.
//!
//! # A backend of your own
//!
//! A Channel's written configuration kept in a flash sector, here a
//! `Vec<u8>` (empty while erased). A firmware image would erase and program
//! the sector where this one replaces the vector.
//!
//! ```
//! use std::sync::Mutex;
//!
//! use bacnet_objects::snapshot::write_config;
//! use bacnet_objects::write_config::{
//!     ChannelWriteConfig, WriteConfigPersistence, WriteConfigSnapshot,
//! };
//! use bacnet_types::enums::ObjectType;
//! use bacnet_types::error::Error;
//! use bacnet_types::primitives::ObjectIdentifier;
//!
//! struct FlashSector(Mutex<Vec<u8>>);
//!
//! impl WriteConfigPersistence for FlashSector {
//!     fn load(&self, object: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error> {
//!         let sector = self.0.lock().unwrap();
//!         if sector.is_empty() {
//!             return Ok(None);
//!         }
//!         write_config::decode(object, &sector)
//!             .map(Some)
//!             .map_err(|error| Error::Encoding(error.to_string()))
//!     }
//!
//!     fn save(&self, object: ObjectIdentifier, snapshot: &WriteConfigSnapshot) -> Result<(), Error> {
//!         let octets = write_config::encode(object, snapshot)
//!             .map_err(|error| Error::OutOfRange(error.to_string()))?;
//!         *self.0.lock().unwrap() = octets;
//!         Ok(())
//!     }
//! }
//!
//! let flash = FlashSector(Mutex::new(Vec::new()));
//! let channel = ObjectIdentifier::new(ObjectType::CHANNEL, 1)?;
//! assert_eq!(flash.load(channel)?, None);
//!
//! let mut config = ChannelWriteConfig::default();
//! config.channel_number = Some(12);
//! let written = WriteConfigSnapshot::Channel(config);
//! flash.save(channel, &written)?;
//! assert!(flash.0.lock().unwrap().starts_with(&write_config::MAGIC));
//! assert_eq!(flash.load(channel)?, Some(written));
//! # Ok::<(), Error>(())
//! ```
//!
//! [`LogPersistence`]: crate::log_persistence::LogPersistence
//! [`LogSnapshot`]: crate::log_persistence::LogSnapshot
//! [`AuditLogSnapshot`]: crate::audit::AuditLogSnapshot
//! [`WriteConfigPersistence`]: crate::write_config::WriteConfigPersistence
//! [`WriteConfigSnapshot`]: crate::write_config::WriteConfigSnapshot
//! [`NotificationClassSnapshot`]: crate::notification_class::NotificationClassSnapshot
//! [`ForwarderSnapshot`]: crate::notification_forwarder::ForwarderSnapshot
//! [`TagsSnapshot`]: crate::object_profile::TagsSnapshot
//! [`ColorOutputSnapshot`]: crate::color::ColorOutputSnapshot
//! [`AccessRightsSnapshot`]: crate::access_control::AccessRightsSnapshot

pub mod access_rights;
pub mod audit_log;
pub mod color;
mod error;
pub(crate) mod framing;
pub mod log;
pub mod notification_class;
pub mod notification_forwarder;
pub mod tags;
pub mod write_config;

pub use error::{SnapshotError, SnapshotErrorKind};

#[cfg(test)]
mod audit_log_tests;
#[cfg(test)]
mod described_tests;
#[cfg(test)]
mod description_tests;
#[cfg(test)]
pub(crate) mod enrollment_parameters_tests;
#[cfg(test)]
mod one_file_tests;
#[cfg(test)]
mod parameter_list_tests;
#[cfg(test)]
mod parameters_tests;
#[cfg(test)]
mod reporting_tests;
#[cfg(test)]
mod slot_tests;
#[cfg(test)]
mod state_text_tests;
#[cfg(test)]
mod tests;
