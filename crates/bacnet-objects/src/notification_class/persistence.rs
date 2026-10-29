//! Keeping a Notification Class's written Recipient_List across a restart
//! (Clause 12.21.8, #1315).
//!
//! As for the Notification Forwarder
//! ([`NotificationForwarderPersistence`](crate::notification_forwarder::NotificationForwarderPersistence)),
//! the storage belongs to the application. The class loads it once, when it
//! is built, and saves the list on its own writer thread each time a write
//! changes it. The bundled server waits for that save with the database
//! guard dropped; a write nobody staged waits where it is ([`crate::durable`]
//! lists those paths).

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

#[cfg(feature = "std")]
use bacnet_encoding::constructed::{decode_destination, encode_destination_list};
use bacnet_types::constructed::BACnetDestination;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
#[cfg(feature = "std")]
use bytes::BytesMut;

#[cfg(feature = "std")]
use super::MAX_RECIPIENT_LIST_DESTINATIONS;
#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;

/// What a Notification Class keeps across a restart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotificationClassSnapshot {
    /// Recipient_List, in list order, once a write has set it. `None` until
    /// then: the destinations the application configures apply at each
    /// start.
    pub recipient_list: Option<Vec<BACnetDestination>>,
}

/// Application-owned storage for one Notification Class's Recipient_List.
///
/// The class calls it from its writer thread, one call at a time. A written
/// list lands in storage a moment before the class serves it: see [storage
/// leads the served state](crate::durable#storage-leads-the-served-state).
pub trait NotificationClassPersistence: Send + Sync {
    /// What was last saved for `class`, or `None` when nothing was saved.
    fn load(&self, class: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error>;

    /// Durably replace what is saved for `class` with `snapshot`.
    fn save(
        &self,
        class: ObjectIdentifier,
        snapshot: &NotificationClassSnapshot,
    ) -> Result<(), Error>;
}

/// The format's tag. The Notification Forwarder's file (`RBNFWD01`) is a
/// sibling: the same header, then both of a forwarder's lists.
#[cfg(feature = "std")]
const MAGIC: &[u8; 8] = b"RBNNCL01";
/// The Notification Forwarder's cap: a full Recipient_List of the longest
/// destinations is 1,504 octets, so anything much larger is not a file this
/// backend wrote.
#[cfg(feature = "std")]
pub(super) const MAX_FILE_BYTES: u64 = 64 * 1024;

/// A file holding one Notification Class's Recipient_List, replaced whole on
/// each save, the same way as [`FileNotificationForwarderPersistence`]: a
/// failed save leaves the previous list in place, and a completed one
/// survives a power loss.
///
/// The file holds a magic tag, the class's object identifier, one octet that
/// is 1 when a Recipient_List follows and 0 when not, and then the encoded
/// Recipient_List. It does not coordinate between processes.
///
/// [`FileNotificationForwarderPersistence`]: crate::notification_forwarder::FileNotificationForwarderPersistence
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileNotificationClassPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileNotificationClassPersistence {
    /// Keep the Recipient_List in the file at `path`.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), MAGIC, MAX_FILE_BYTES, "Notification Class")?,
        })
    }

    /// The file the Recipient_List is kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl NotificationClassPersistence for FileNotificationClassPersistence {
    fn load(&self, class: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error> {
        let Some(body) = self.file.load(class)? else {
            return Ok(None);
        };
        let recipient_list = match body.split_first() {
            Some((0, [])) => None,
            Some((1, list)) => Some(self.file.decode_capped(
                list,
                MAX_RECIPIENT_LIST_DESTINATIONS,
                decode_destination,
            )?),
            _ => return Err(self.file.corrupt("has no valid header")),
        };
        Ok(Some(NotificationClassSnapshot { recipient_list }))
    }

    fn save(
        &self,
        class: ObjectIdentifier,
        snapshot: &NotificationClassSnapshot,
    ) -> Result<(), Error> {
        let mut body = BytesMut::new();
        body.extend_from_slice(&[u8::from(snapshot.recipient_list.is_some())]);
        if let Some(list) = &snapshot.recipient_list {
            encode_destination_list(&mut body, list)?;
        }
        self.file.save(class, &body)
    }
}
