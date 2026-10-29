//! Keeping a forwarder's Recipient_List and Subscribed_Recipients across a
//! restart (Clauses 12.51.8 and 12.51.9).
//!
//! Like [`AuditLogPersistence`](crate::audit::AuditLogPersistence), the
//! storage belongs to the application. The forwarder loads it once, when it is
//! built, and saves both lists whenever either changes. Saves run on the
//! forwarder's own writer thread. The bundled server waits for them with the
//! database guard dropped; a write nobody staged waits where it is
//! ([`crate::durable`] lists those paths). Each saved Subscribed_Recipients
//! entry carries the whole minutes it had left at the save.

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

#[cfg(feature = "std")]
use bacnet_encoding::constructed::{
    decode_destination, decode_event_notification_subscription, encode_destination_list,
    encode_event_notification_subscription_list,
};
use bacnet_types::constructed::{BACnetDestination, BACnetEventNotificationSubscription};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
#[cfg(feature = "std")]
use bytes::BytesMut;

#[cfg(feature = "std")]
use super::MAX_RECIPIENT_LIST_DESTINATIONS;
#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;
#[cfg(feature = "std")]
use crate::subscribed_recipients::MAX_SUBSCRIBED_RECIPIENTS;

/// What a forwarder keeps across a restart: both of its lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForwarderSnapshot {
    /// Recipient_List, in list order, once a write has set it. `None` until
    /// then: the destinations the application configures apply at each
    /// start.
    pub recipient_list: Option<Vec<BACnetDestination>>,
    /// The live Subscribed_Recipients entries, in list order, each with the
    /// whole minutes it had left when saved.
    pub subscribed_recipients: Vec<BACnetEventNotificationSubscription>,
}

/// Application-owned storage for one Notification Forwarder's lists.
///
/// The forwarder calls it from its writer thread, one call at a time. A
/// written list lands in storage a moment before the forwarder serves it: see
/// [storage leads the served
/// state](crate::durable#storage-leads-the-served-state).
pub trait NotificationForwarderPersistence: Send + Sync {
    /// The lists last saved for `forwarder`, or `None` when nothing was
    /// saved.
    fn load(&self, forwarder: ObjectIdentifier) -> Result<Option<ForwarderSnapshot>, Error>;

    /// Durably replace what is saved for `forwarder` with `snapshot`.
    fn save(&self, forwarder: ObjectIdentifier, snapshot: &ForwarderSnapshot) -> Result<(), Error>;
}

#[cfg(feature = "std")]
const MAGIC: &[u8; 8] = b"RBNFWD01";
/// After the shared header (the magic tag and the object identifier): one
/// octet, 1 when the file holds a Recipient_List and 0 when not, then the
/// Recipient_List's length.
#[cfg(feature = "std")]
const BODY_HEADER_LEN: usize = 1 + 4;
/// A full Recipient_List of the longest destinations is 1,504 octets and a
/// full Subscribed_Recipients 1,184; anything much larger is not a file this
/// backend wrote.
#[cfg(feature = "std")]
pub(super) const MAX_FILE_BYTES: u64 = 64 * 1024;

/// A file holding one forwarder's lists, replaced whole on each save.
///
/// A save writes a sibling `.tmp` file, synchronizes it, renames it over the
/// old one and then synchronizes the directory, so a failed save leaves the
/// previous lists in place and a completed one survives a power loss. Once
/// the rename succeeds the save has landed: a directory sync that fails is
/// logged, not returned. (On Windows the directory is not synchronized: see
/// the [`durable`](crate::durable) module.) The file holds a magic tag, the
/// forwarder's object identifier, whether a Recipient_List follows, the
/// encoded Recipient_List with its length, and the encoded
/// Subscribed_Recipients. It does not coordinate between processes.
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileNotificationForwarderPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileNotificationForwarderPersistence {
    /// Keep the lists in the file at `path`.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(
                path.as_ref(),
                MAGIC,
                MAX_FILE_BYTES,
                "Notification Forwarder",
            )?,
        })
    }

    /// The file the lists are kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl NotificationForwarderPersistence for FileNotificationForwarderPersistence {
    fn load(&self, forwarder: ObjectIdentifier) -> Result<Option<ForwarderSnapshot>, Error> {
        let Some(body) = self.file.load(forwarder)? else {
            return Ok(None);
        };
        if body.len() < BODY_HEADER_LEN {
            return Err(self.file.corrupt("has no valid header"));
        }
        let length = u32::from_be_bytes(body[1..BODY_HEADER_LEN].try_into().unwrap());
        let split = usize::try_from(length)
            .ok()
            .and_then(|length| BODY_HEADER_LEN.checked_add(length))
            .filter(|split| *split <= body.len())
            .ok_or_else(|| self.file.corrupt("has a Recipient_List past its end"))?;
        let recipient_list = &body[BODY_HEADER_LEN..split];
        let recipient_list = match body[0] {
            0 if recipient_list.is_empty() => None,
            1 => Some(self.file.decode_capped(
                recipient_list,
                MAX_RECIPIENT_LIST_DESTINATIONS,
                decode_destination,
            )?),
            _ => return Err(self.file.corrupt("has no valid header")),
        };
        Ok(Some(ForwarderSnapshot {
            recipient_list,
            subscribed_recipients: self.file.decode_capped(
                &body[split..],
                MAX_SUBSCRIBED_RECIPIENTS,
                decode_event_notification_subscription,
            )?,
        }))
    }

    fn save(&self, forwarder: ObjectIdentifier, snapshot: &ForwarderSnapshot) -> Result<(), Error> {
        let mut recipient_list = BytesMut::new();
        if let Some(list) = &snapshot.recipient_list {
            encode_destination_list(&mut recipient_list, list)?;
        }
        let mut body = BytesMut::with_capacity(BODY_HEADER_LEN + recipient_list.len() + 1024);
        body.extend_from_slice(&[u8::from(snapshot.recipient_list.is_some())]);
        let length = u32::try_from(recipient_list.len())
            .map_err(|_| Error::OutOfRange("Recipient_List too long to save".into()))?;
        body.extend_from_slice(&length.to_be_bytes());
        body.extend_from_slice(&recipient_list);
        encode_event_notification_subscription_list(&mut body, &snapshot.subscribed_recipients)?;
        self.file.save(forwarder, &body)
    }
}
