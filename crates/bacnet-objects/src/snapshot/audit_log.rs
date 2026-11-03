//! The Audit Log snapshot slot (`RBALOG01`), as `FileAuditLogPersistence`
//! (with std) writes each of its two slots.
//!
//! A slot is a 26-octet header (the magic tag, the schema version, the
//! object identifier, the generation and the payload length), the payload,
//! and a CRC-32 of everything before it. The payload holds the capacity, an
//! Enable octet, Total_Record_Count and the records, each with its sequence
//! number and length; from version 2 the completed receipts follow. Version
//! 3 stores log-status records bit 0 first, as every BACnet bit string is;
//! versions 1 and 2, from releases up to 0.11.0, stored them reversed, and
//! [`decode`] puts them right. [`encode`] writes version 3 only.
//!
//! The file backend keeps two slots and writes the one the generation's
//! parity names. A backend of its own can do the same: hand both slots'
//! octets to [`newest`], which makes the file backend's choice.

use crate::prelude::*;
use bacnet_types::primitives::ObjectIdentifier;

use super::SnapshotError;
use crate::audit::AuditLogSnapshot;
use crate::audit::persistence::codec;

/// The magic tag every slot starts with.
pub const MAGIC: [u8; 8] = *codec::MAGIC;
/// The schema version [`encode`] writes.
pub const VERSION: u16 = codec::SCHEMA_VERSION;
/// The oldest schema version [`decode`] still reads.
pub const OLDEST_VERSION: u16 = codec::SCHEMA_VERSION_V1;
/// The most octets a slot may take.
pub const MAX_LEN: usize = codec::MAX_SNAPSHOT_BYTES;

/// The slot octets for `snapshot`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) a snapshot that
/// doesn't hold together, as [`decode`] would refuse it (a zero generation,
/// more records than its capacity or than the Audit Log keeps, sequence
/// numbers that don't run on to Total_Record_Count, receipts past their
/// caps or repeated), a record past 1 MiB, and a slot past [`MAX_LEN`].
pub fn encode(snapshot: &AuditLogSnapshot) -> Result<Vec<u8>, SnapshotError> {
    codec::encode(snapshot)
}

/// The snapshot one slot's `octets` hold for `object`.
///
/// Refuses damage (a slot cut short or past [`MAX_LEN`], a bad magic tag,
/// length or checksum, or contents that don't decode or don't hold
/// together), and a sound slot of another object or of a schema version
/// this build doesn't read (any but [`OLDEST_VERSION`] to [`VERSION`]).
pub fn decode(object: ObjectIdentifier, octets: &[u8]) -> Result<AuditLogSnapshot, SnapshotError> {
    codec::decode(octets, object)
}

/// The slot to load, as [`newest`] picks it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NewestSlot {
    /// The slot it came from, 0 or 1.
    pub slot: usize,
    /// Its generation: the next commit is one past it, to the other slot.
    pub generation: u64,
    /// The snapshot.
    pub snapshot: AuditLogSnapshot,
    /// Each damaged slot passed over, by slot, for the backend to report.
    pub passed_over: [Option<SnapshotError>; 2],
}

/// Pick the snapshot to load for `object` from a two-slot store's slots,
/// `None` for a slot that holds nothing, as `FileAuditLogPersistence` picks
/// it.
///
/// The slot of the higher generation that decodes wins, and a damaged slot
/// is passed over for the other. `Ok(None)` when neither slot holds
/// anything. Refused:
///
/// - a slot that is a sound snapshot this log must not take (another
///   object's, or an unknown schema version), without falling back to the
///   other;
/// - both slots damaged, or one damaged and the other empty: the error of
///   the last damaged slot (slot 1's when both are). The file backend never
///   reads a slot past the size cap, and reports it only when no slot it
///   read was damaged;
/// - two slots of one generation that encode, in today's schema, to
///   different octets ([`Damaged`](super::SnapshotErrorKind::Damaged)). Two slots
///   that differ only in schema version, holding the same state, are one
///   snapshot.
pub fn newest(
    object: ObjectIdentifier,
    slots: [Option<&[u8]>; 2],
) -> Result<Option<NewestSlot>, SnapshotError> {
    let mut newest: Option<NewestSlot> = None;
    let mut passed_over = [None, None];
    let mut damage = None;
    for (slot, octets) in slots.into_iter().enumerate() {
        let Some(octets) = octets else {
            continue;
        };
        match decode(object, octets) {
            Err(error) if !error.is_damage() => return Err(error),
            Err(error) => {
                passed_over[slot] = Some(error);
                damage = Some(error);
            }
            Ok(snapshot) => match &newest {
                Some(current) if current.generation > snapshot.generation => {}
                Some(current) if current.generation == snapshot.generation => {
                    if encode(&snapshot)? != encode(&current.snapshot)? {
                        return Err(SnapshotError::damaged(
                            "slots contain divergent snapshots at the same generation",
                        )
                        .in_format("AuditLog"));
                    }
                }
                _ => {
                    newest = Some(NewestSlot {
                        slot,
                        generation: snapshot.generation,
                        snapshot,
                        passed_over: [None, None],
                    });
                }
            },
        }
    }
    match (newest, damage) {
        (Some(mut newest), _) => {
            newest.passed_over = passed_over;
            Ok(Some(newest))
        }
        (None, Some(error)) => Err(error),
        (None, None) => Ok(None),
    }
}
