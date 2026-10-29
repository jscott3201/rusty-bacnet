//! Keeping what peers write to an Access Rights object across a restart
//! (#1392).
//!
//! Clause 12.34 says nothing about restarts, so this is the stack's own
//! choice, made because peers provision access rules over the network
//! (#1330, #1332): a head end writes the rules once and expects the
//! controller to keep them. As for the Notification Class
//! ([`NotificationClassPersistence`](crate::notification_class::NotificationClassPersistence)),
//! the storage belongs to the application. The object loads it once, when it
//! is built, and saves on its own writer thread each time a write changes
//! Positive_Access_Rules, Negative_Access_Rules, Enable or Accompaniment
//! (#1393). The bundled server waits for that save with the database guard
//! dropped; a write nobody staged waits where it is ([`crate::durable`] lists
//! those paths).

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

#[cfg(feature = "std")]
use bacnet_encoding::constructed::tagged::{
    decode_ctx_boolean, expect_closing, expect_opening, next_is_context, next_is_opening,
};
#[cfg(feature = "std")]
use bacnet_encoding::constructed::{
    decode_access_rule, decode_device_object_reference, encode_access_rule,
    encode_device_object_reference,
};
#[cfg(feature = "std")]
use bacnet_encoding::{primitives, tags};
use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectReference};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
#[cfg(feature = "std")]
use bytes::BytesMut;

#[cfg(feature = "std")]
use super::MAX_ACCESS_RULES;
#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;

/// What an Access Rights object keeps across a restart. Each member is
/// `None` until a write sets it: what the application configures applies at
/// each start until then.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessRightsSnapshot {
    /// Positive_Access_Rules, in array order, once a write has set them.
    pub positive_access_rules: Option<Vec<BACnetAccessRule>>,
    /// Negative_Access_Rules, in array order, once a write has set them.
    pub negative_access_rules: Option<Vec<BACnetAccessRule>>,
    /// Enable, once a write has set it.
    pub enable: Option<bool>,
    /// Accompaniment, once a write has set it (#1393).
    pub accompaniment: Option<BACnetDeviceObjectReference>,
}

/// Application-owned storage for one Access Rights object's rule arrays,
/// Enable and Accompaniment.
///
/// The object calls it from its writer thread, one call at a time. A written
/// value lands in storage a moment before the object serves it: see [storage
/// leads the served state](crate::durable#storage-leads-the-served-state).
pub trait AccessRightsPersistence: Send + Sync {
    /// What was last saved for `rights`, or `None` when nothing was saved.
    fn load(&self, rights: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error>;

    /// Durably replace what is saved for `rights` with `snapshot`.
    fn save(&self, rights: ObjectIdentifier, snapshot: &AccessRightsSnapshot) -> Result<(), Error>;
}

/// The format's tag. A Notification Class's `RBNNCL01` and a forwarder's
/// `RBNFWD01` files are siblings with the same header; a new layout gets a
/// new tag. Accompaniment (#1393) is an optional member after the others,
/// so a file written before it existed is one with that member absent.
#[cfg(feature = "std")]
const MAGIC: &[u8; 8] = b"RBNACR01";
/// A rule this object accepts encodes in at most 40 octets, so two full
/// arrays of [`MAX_ACCESS_RULES`] take about 80 KiB, and Accompaniment at
/// most 12 octets more. Anything much larger is not a file this backend
/// wrote.
#[cfg(feature = "std")]
pub(super) const MAX_FILE_BYTES: u64 = 128 * 1024;
/// The context tags of the four members, in file order.
#[cfg(feature = "std")]
const POSITIVE_TAG: u8 = 0;
#[cfg(feature = "std")]
const NEGATIVE_TAG: u8 = 1;
#[cfg(feature = "std")]
const ENABLE_TAG: u8 = 2;
#[cfg(feature = "std")]
const ACCOMPANIMENT_TAG: u8 = 3;
/// The label decoding errors carry.
#[cfg(feature = "std")]
const WHAT: &str = "Access Rights file";

/// A file holding one Access Rights object's written rule arrays, Enable and
/// Accompaniment, replaced whole on each save, the same way as
/// [`FileNotificationClassPersistence`]: a failed save leaves the previous
/// state in place, and a completed one survives a power loss.
///
/// After the magic tag `RBNACR01` and the object's identifier, the file
/// holds BACnet encodings of up to four members, in this order, each only
/// once a write has set it:
///
/// - context tag 0, opening, then each Positive_Access_Rules rule in its
///   Clause 21 `BACnetAccessRule` encoding, then context tag 0, closing;
/// - the same for Negative_Access_Rules under context tag 1;
/// - Enable as a context-tagged BOOLEAN with tag 2;
/// - context tag 3, opening, then Accompaniment in its Clause 21
///   `BACnetDeviceObjectReference` encoding, then context tag 3, closing.
///
/// Loading refuses a file past 128 KiB, an array of more than
/// [`MAX_ACCESS_RULES`] rules, members out of order or repeated, a rule or
/// reference that does not decode, and octets after the last member. It
/// does not check the rules or the reference themselves; the object does,
/// as it loads them. It does not coordinate between processes.
///
/// [`FileNotificationClassPersistence`]: crate::notification_class::FileNotificationClassPersistence
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileAccessRightsPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileAccessRightsPersistence {
    /// Keep the rule arrays, Enable and Accompaniment in the file at `path`.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), MAGIC, MAX_FILE_BYTES, "Access Rights")?,
        })
    }

    /// The file the rule arrays, Enable and Accompaniment are kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }

    fn undecodable(&self) -> Error {
        self.file.corrupt("holds an entry that does not decode")
    }

    /// The rule array framed by context tag `tag` at `*pos`, if one is there,
    /// moving `*pos` past it.
    fn decode_array(
        &self,
        body: &[u8],
        pos: &mut usize,
        tag: u8,
    ) -> Result<Option<Vec<BACnetAccessRule>>, Error> {
        if !next_is_opening(body, *pos, tag).map_err(|_| self.undecodable())? {
            return Ok(None);
        }
        let mut at = expect_opening(body, *pos, tag, WHAT).map_err(|_| self.undecodable())?;
        let mut rules = Vec::new();
        loop {
            // Running out of octets here means the frame never closed.
            let (next, after) = tags::decode_tag(body, at).map_err(|_| self.undecodable())?;
            if next.is_closing_tag(tag) {
                *pos = after;
                return Ok(Some(rules));
            }
            if rules.len() == MAX_ACCESS_RULES {
                return Err(self.file.corrupt("holds more entries than the cap"));
            }
            let (rule, end) = decode_access_rule(body, at).map_err(|_| self.undecodable())?;
            rules.push(rule);
            at = end;
        }
    }

    /// The Accompaniment framed by context tag 3 at `*pos`, if one is there,
    /// moving `*pos` past it.
    fn decode_accompaniment(
        &self,
        body: &[u8],
        pos: &mut usize,
    ) -> Result<Option<BACnetDeviceObjectReference>, Error> {
        if !next_is_opening(body, *pos, ACCOMPANIMENT_TAG).map_err(|_| self.undecodable())? {
            return Ok(None);
        }
        let decoded = expect_opening(body, *pos, ACCOMPANIMENT_TAG, WHAT)
            .and_then(|at| decode_device_object_reference(body, at))
            .and_then(|(reference, at)| {
                expect_closing(body, at, ACCOMPANIMENT_TAG, WHAT).map(|end| (reference, end))
            });
        let (reference, end) = decoded.map_err(|_| self.undecodable())?;
        *pos = end;
        Ok(Some(reference))
    }
}

#[cfg(feature = "std")]
impl AccessRightsPersistence for FileAccessRightsPersistence {
    fn load(&self, rights: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error> {
        let Some(body) = self.file.load(rights)? else {
            return Ok(None);
        };
        let mut pos = 0;
        let positive_access_rules = self.decode_array(&body, &mut pos, POSITIVE_TAG)?;
        let negative_access_rules = self.decode_array(&body, &mut pos, NEGATIVE_TAG)?;
        let enable = if next_is_context(&body, pos, ENABLE_TAG).map_err(|_| self.undecodable())? {
            let (enable, end) =
                decode_ctx_boolean(&body, pos, ENABLE_TAG, WHAT).map_err(|_| self.undecodable())?;
            pos = end;
            Some(enable)
        } else {
            None
        };
        let accompaniment = self.decode_accompaniment(&body, &mut pos)?;
        if pos != body.len() {
            return Err(self.undecodable());
        }
        Ok(Some(AccessRightsSnapshot {
            positive_access_rules,
            negative_access_rules,
            enable,
            accompaniment,
        }))
    }

    fn save(&self, rights: ObjectIdentifier, snapshot: &AccessRightsSnapshot) -> Result<(), Error> {
        let mut body = BytesMut::new();
        for (tag, rules) in [
            (POSITIVE_TAG, &snapshot.positive_access_rules),
            (NEGATIVE_TAG, &snapshot.negative_access_rules),
        ] {
            if let Some(rules) = rules {
                tags::encode_opening_tag(&mut body, tag);
                for rule in rules {
                    encode_access_rule(&mut body, rule);
                }
                tags::encode_closing_tag(&mut body, tag);
            }
        }
        if let Some(enable) = snapshot.enable {
            primitives::encode_ctx_boolean(&mut body, ENABLE_TAG, enable);
        }
        if let Some(reference) = &snapshot.accompaniment {
            tags::encode_opening_tag(&mut body, ACCOMPANIMENT_TAG);
            encode_device_object_reference(&mut body, reference);
            tags::encode_closing_tag(&mut body, ACCOMPANIMENT_TAG);
        }
        self.file.save(rights, &body)
    }
}
