//! A Notification Class's written Recipient_List and Description
//! (`RBNNCL01`, and `RBNNCL02` with a Description), as
//! `FileNotificationClassPersistence` (with std) writes its file.
//!
//! After the magic tag and the four-octet object identifier, one presence
//! octet: 0 for no written list, and nothing after it, or 1 followed by the
//! list's destinations, each in its BACnet encoding, filling the rest.
//!
//! A file holding a written Description (#2364) takes version 2's tag,
//! [`MAGIC_WITH_DESCRIPTION`]: the Description, its length in four
//! big-endian octets and then at most 16 KiB of UTF-8, comes between the
//! header and that body. [`encode`] writes version 1 for a snapshot without
//! one, so its octets don't change, and a build that reads only version 1
//! refuses version 2 as an unknown version.

use crate::prelude::*;
use bacnet_encoding::constructed::{decode_destination, encode_destination_list};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::SnapshotError;
use super::framing;
use crate::notification_class::{MAX_RECIPIENT_LIST_DESTINATIONS, NotificationClassSnapshot};

/// The format's tag, version 1: a snapshot without a written Description.
pub const MAGIC: [u8; 8] = *b"RBNNCL01";
/// Version 2's tag, for a snapshot with a written Description (#2364).
pub const MAGIC_WITH_DESCRIPTION: [u8; 8] = *b"RBNNCL02";
/// The most octets a file may take: a full Recipient_List of the longest
/// destinations is 1,504 octets, and a Description at most 16 KiB, so
/// anything much larger is not a save.
pub const MAX_LEN: usize = 64 * 1024;

/// The file octets for `snapshot`, saved for the class `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) more
/// destinations than [`MAX_RECIPIENT_LIST_DESTINATIONS`], a destination
/// that doesn't encode, a Description past 16 KiB, and octets past
/// [`MAX_LEN`]: what [`decode`] would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &NotificationClassSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_file(object, snapshot).map_err(|error| error.in_format("Notification Class snapshot"))
}

/// The snapshot a file's `octets` hold for the class `object`.
///
/// Refuses octets past [`MAX_LEN`], without either version's tag, saved for
/// another object, with a presence octet other than 0 or 1 (or octets after
/// a 0), more destinations than [`MAX_RECIPIENT_LIST_DESTINATIONS`], or one
/// that does not decode, and a Description cut short, past 16 KiB or not
/// UTF-8.
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
    let octets = framing::seal_described(
        (&MAGIC, &MAGIC_WITH_DESCRIPTION),
        object,
        snapshot.description.as_deref(),
        &body,
    )?;
    if octets.len() > MAX_LEN {
        return Err(SnapshotError::out_of_range("is too large to save"));
    }
    Ok(octets)
}

fn decode_file(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<NotificationClassSnapshot, SnapshotError> {
    let (description, body) =
        framing::open_described(octets, (&MAGIC, &MAGIC_WITH_DESCRIPTION), MAX_LEN, object)?;
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
    Ok(NotificationClassSnapshot {
        recipient_list,
        description,
    })
}
