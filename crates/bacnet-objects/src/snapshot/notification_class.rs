//! A Notification Class's written Recipient_List (`RBNNCL01`), as
//! `FileNotificationClassPersistence` (with std) writes its file.
//!
//! After the magic tag and the four-octet object identifier, one presence
//! octet: 0 for no written list, and nothing after it, or 1 followed by the
//! list's destinations, each in its BACnet encoding, filling the rest.

use crate::prelude::*;
use bacnet_encoding::constructed::{decode_destination, encode_destination_list};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::SnapshotError;
use super::framing::{self, HEADER_LEN};
use crate::notification_class::{MAX_RECIPIENT_LIST_DESTINATIONS, NotificationClassSnapshot};

/// The format's tag.
pub const MAGIC: [u8; 8] = *b"RBNNCL01";
/// The most octets a file may take: a full Recipient_List of the longest
/// destinations is 1,504 octets, so anything much larger is not a save.
pub const MAX_LEN: usize = 64 * 1024;

/// The file octets for `snapshot`, saved for the class `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) more
/// destinations than [`MAX_RECIPIENT_LIST_DESTINATIONS`], a destination
/// that doesn't encode, and octets past [`MAX_LEN`]: what [`decode`] would
/// refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &NotificationClassSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_file(object, snapshot).map_err(|error| error.in_format("Notification Class snapshot"))
}

/// The snapshot a file's `octets` hold for the class `object`.
///
/// Refuses octets past [`MAX_LEN`], without this format's tag, saved for
/// another object, with a presence octet other than 0 or 1 (or octets after
/// a 0), more destinations than [`MAX_RECIPIENT_LIST_DESTINATIONS`], or one
/// that does not decode.
pub fn decode(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<NotificationClassSnapshot, SnapshotError> {
    decode_file(object, octets).map_err(|error| error.in_format("Notification Class file"))
}

fn encode_file(
    object: ObjectIdentifier,
    snapshot: &NotificationClassSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    let mut body = BytesMut::new();
    body.extend_from_slice(&[u8::from(snapshot.recipient_list.is_some())]);
    if let Some(list) = &snapshot.recipient_list {
        if list.len() > MAX_RECIPIENT_LIST_DESTINATIONS {
            return Err(SnapshotError::out_of_range(
                "holds more destinations than a class keeps",
            ));
        }
        encode_destination_list(&mut body, list)
            .map_err(|_| SnapshotError::out_of_range("holds a destination that doesn't encode"))?;
    }
    if HEADER_LEN + body.len() > MAX_LEN {
        return Err(SnapshotError::out_of_range("is too large to save"));
    }
    Ok(framing::seal(&MAGIC, object, &body))
}

fn decode_file(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<NotificationClassSnapshot, SnapshotError> {
    let (_, body) = framing::open(octets, &MAGIC, None, MAX_LEN, object)?;
    let recipient_list = match body.split_first() {
        Some((0, [])) => None,
        Some((1, list)) => Some(framing::decode_capped(
            list,
            MAX_RECIPIENT_LIST_DESTINATIONS,
            decode_destination,
        )?),
        None => return Err(SnapshotError::truncated("has no valid header")),
        _ => return Err(SnapshotError::damaged("has no valid header")),
    };
    Ok(NotificationClassSnapshot { recipient_list })
}
