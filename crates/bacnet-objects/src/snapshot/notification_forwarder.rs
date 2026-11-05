//! A Notification Forwarder's two lists and written Description
//! (`RBNFWD01`, and `RBNFWD02` with a Description), as
//! `FileNotificationForwarderPersistence` (with std) writes its file.
//!
//! After the magic tag and the four-octet object identifier: one presence
//! octet, 1 when a written Recipient_List follows and 0 when not; the
//! Recipient_List's length in four big-endian octets (0 with no list); its
//! destinations, each in its BACnet encoding; then Subscribed_Recipients,
//! each entry in its BACnet encoding, filling the rest. Each entry
//! carries the whole minutes it had left when it was saved.
//!
//! A file holding a written Description (#2364) takes version 2's tag,
//! [`MAGIC_WITH_DESCRIPTION`]: the Description, its length in four
//! big-endian octets and then at most 16 KiB of UTF-8, comes between the
//! header and that body. [`encode`] writes version 1 for a snapshot without
//! one, so its octets don't change, and a build that reads only version 1
//! refuses version 2 as an unknown version.

use crate::prelude::*;
use bacnet_encoding::constructed::{
    decode_destination, decode_event_notification_subscription, encode_destination_list,
    encode_event_notification_subscription_list,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::SnapshotError;
use super::framing;
use crate::notification_class::MAX_RECIPIENT_LIST_DESTINATIONS;
use crate::notification_forwarder::ForwarderSnapshot;
use crate::subscribed_recipients::MAX_SUBSCRIBED_RECIPIENTS;

/// The format's tag, version 1: a snapshot without a written Description.
pub const MAGIC: [u8; 8] = *b"RBNFWD01";
/// Version 2's tag, for a snapshot with a written Description (#2364).
pub const MAGIC_WITH_DESCRIPTION: [u8; 8] = *b"RBNFWD02";
/// The most octets a file may take: a full Recipient_List of the longest
/// destinations is 1,504 octets, a full Subscribed_Recipients 1,184 and a
/// Description at most 16 KiB, so anything much larger is not a save.
pub const MAX_LEN: usize = 64 * 1024;

/// The presence octet and the Recipient_List's length.
const BODY_HEADER_LEN: usize = 1 + 4;

/// The file octets for `snapshot`, saved for the forwarder `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) a list
/// longer than a forwarder keeps ([`MAX_RECIPIENT_LIST_DESTINATIONS`],
/// [`MAX_SUBSCRIBED_RECIPIENTS`]), an entry that doesn't encode, a
/// Description past 16 KiB, and octets past [`MAX_LEN`]: what [`decode`]
/// would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &ForwarderSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_file(object, snapshot)
        .map_err(|error| error.in_format("Notification Forwarder snapshot"))
}

/// The snapshot a file's `octets` hold for the forwarder `object`.
///
/// Refuses octets past [`MAX_LEN`], without either version's tag, saved for
/// another object, cut short, with a presence octet other than 0 or 1 (or a
/// list behind a 0), a list longer than the forwarder keeps, an entry that
/// does not decode, or a Description past 16 KiB or not UTF-8.
pub fn decode(object: ObjectIdentifier, octets: &[u8]) -> Result<ForwarderSnapshot, SnapshotError> {
    decode_file(object, octets).map_err(|error| error.in_format("Notification Forwarder file"))
}

fn encode_file(
    object: ObjectIdentifier,
    snapshot: &ForwarderSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    let unencodable = SnapshotError::out_of_range("holds an entry that doesn't encode");
    let too_long = SnapshotError::out_of_range("holds more entries than a forwarder keeps");
    let mut recipient_list = BytesMut::new();
    if let Some(list) = &snapshot.recipient_list {
        if list.len() > MAX_RECIPIENT_LIST_DESTINATIONS {
            return Err(too_long);
        }
        encode_destination_list(&mut recipient_list, list).map_err(|_| unencodable)?;
    }
    if snapshot.subscribed_recipients.len() > MAX_SUBSCRIBED_RECIPIENTS {
        return Err(too_long);
    }
    let mut body = BytesMut::with_capacity(BODY_HEADER_LEN + recipient_list.len() + 1024);
    body.extend_from_slice(&[u8::from(snapshot.recipient_list.is_some())]);
    let length = u32::try_from(recipient_list.len())
        .map_err(|_| SnapshotError::out_of_range("is too large to save"))?;
    body.extend_from_slice(&length.to_be_bytes());
    body.extend_from_slice(&recipient_list);
    encode_event_notification_subscription_list(&mut body, &snapshot.subscribed_recipients)
        .map_err(|_| unencodable)?;
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
) -> Result<ForwarderSnapshot, SnapshotError> {
    let (description, body) =
        framing::open_described(octets, (&MAGIC, &MAGIC_WITH_DESCRIPTION), MAX_LEN, object)?;
    if body.len() < BODY_HEADER_LEN {
        return Err(SnapshotError::truncated("has no valid header"));
    }
    let length = u32::from_be_bytes(body[1..BODY_HEADER_LEN].try_into().expect("four"));
    let split = usize::try_from(length)
        .ok()
        .and_then(|length| BODY_HEADER_LEN.checked_add(length))
        .filter(|split| *split <= body.len())
        .ok_or(SnapshotError::truncated(
            "has a Recipient_List past its end",
        ))?;
    let recipient_list = &body[BODY_HEADER_LEN..split];
    let recipient_list = match body[0] {
        0 if recipient_list.is_empty() => None,
        1 => Some(framing::decode_capped(
            recipient_list,
            MAX_RECIPIENT_LIST_DESTINATIONS,
            decode_destination,
        )?),
        _ => return Err(SnapshotError::damaged("has no valid header")),
    };
    Ok(ForwarderSnapshot {
        recipient_list,
        subscribed_recipients: framing::decode_capped(
            &body[split..],
            MAX_SUBSCRIBED_RECIPIENTS,
            decode_event_notification_subscription,
        )?,
        description,
    })
}
