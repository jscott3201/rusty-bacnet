//! The Trend Log, Trend Log Multiple and Event Log snapshot slot (`RBLOGS01`),
//! as `FileLogPersistence` (with std) writes each of its two slots.
//!
//! A slot is a 26-octet header (the magic tag, the schema version, the
//! object identifier, the generation and the payload length), the payload,
//! and a CRC-32 of everything before it. Version 2 ends the payload with the
//! written reporting settings; a version 1 slot still loads, with none
//! written. Version 3 ([`VERSION_WITH_DESCRIPTION`], #2364) adds a written
//! Description after them, and [`encode`] writes it only for a snapshot
//! holding one, so a slot without one keeps version 2's octets.
//! [`decode`] checks everything [`encode`] does, so a slot that loads
//! always saves again.
//!
//! The file backend keeps two slots and writes the one the generation's
//! parity names, so a commit cut off part way leaves the other to load. A
//! backend of its own can do the same: hand both slots' octets to
//! [`newest`], which makes the file backend's choice, and write the next
//! commit to the slot the new generation's parity names.

use crate::prelude::*;
use bacnet_types::constructed::{BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord};
use bacnet_types::primitives::ObjectIdentifier;

use super::SnapshotError;
use crate::log_persistence::LogSnapshot;
use crate::log_persistence::codec;

/// The magic tag every slot starts with.
pub const MAGIC: [u8; 8] = *codec::MAGIC;
/// The schema version [`encode`] writes for a snapshot without a written
/// Description.
pub const VERSION: u16 = codec::SCHEMA_VERSION;
/// The schema version [`encode`] writes for a snapshot with a written
/// Description (#2364): the newest [`decode`] reads.
pub const VERSION_WITH_DESCRIPTION: u16 = codec::SCHEMA_VERSION_DESCRIBED;
/// The oldest schema version [`decode`] still reads.
pub const OLDEST_VERSION: u16 = codec::SCHEMA_VERSION_1;
/// The most octets a slot may take.
pub const MAX_LEN: usize = codec::MAX_SNAPSHOT_BYTES;

/// A log record family a slot can hold: [`BACnetLogRecord`] for a Trend Log,
/// [`BACnetLogMultipleRecord`] for a Trend Log Multiple and
/// [`BACnetEventLogRecord`] for an Event Log. Sealed: these three are all.
pub trait LogRecord: sealed::Sealed + PartialEq {}

mod sealed {
    use super::*;

    pub trait Sealed: Sized {
        fn encode(snapshot: &LogSnapshot<Self>) -> Result<Vec<u8>, SnapshotError>;
        fn decode(
            octets: &[u8],
            object: ObjectIdentifier,
        ) -> Result<LogSnapshot<Self>, SnapshotError>;
    }
}

macro_rules! log_record {
    ($($record:ty),*) => {$(
        impl sealed::Sealed for $record {
            fn encode(snapshot: &LogSnapshot<Self>) -> Result<Vec<u8>, SnapshotError> {
                codec::encode(snapshot)
            }

            fn decode(
                octets: &[u8],
                object: ObjectIdentifier,
            ) -> Result<LogSnapshot<Self>, SnapshotError> {
                codec::decode(octets, object)
            }
        }

        impl LogRecord for $record {}
    )*};
}

log_record!(
    BACnetLogRecord,
    BACnetEventLogRecord,
    BACnetLogMultipleRecord
);

/// The slot octets for `snapshot`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) a snapshot
/// past [`MAX_LEN`] or holding an element of more than 1 MiB, and one that
/// doesn't hold together, as [`decode`] would refuse it: a zero
/// generation, more records than Buffer_Size, settings its object type
/// lacks, and the like.
pub fn encode<R: LogRecord>(snapshot: &LogSnapshot<R>) -> Result<Vec<u8>, SnapshotError> {
    R::encode(snapshot)
}

/// The snapshot one slot's `octets` hold for `object`.
///
/// Refuses damage (a slot cut short or past [`MAX_LEN`], a bad magic tag,
/// length or checksum, or contents that don't decode or don't hold
/// together), and a sound slot of another object or of a schema version
/// this build doesn't read (any but [`OLDEST_VERSION`] to
/// [`VERSION_WITH_DESCRIPTION`]).
pub fn decode<R: LogRecord>(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<LogSnapshot<R>, SnapshotError> {
    R::decode(octets, object)
}

/// The slot to load, as [`newest`] picks it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NewestSlot<R> {
    /// The slot it came from, 0 or 1.
    pub slot: usize,
    /// Its generation: the next commit is one past it, to the other slot.
    pub generation: u64,
    /// The snapshot.
    pub snapshot: LogSnapshot<R>,
    /// Each damaged slot passed over, by slot, for the backend to report.
    pub passed_over: [Option<SnapshotError>; 2],
}

/// Pick the snapshot to load for `object` from a two-slot store's slots,
/// `None` for a slot that holds nothing, as `FileLogPersistence` picks it.
///
/// The slot of the higher generation that decodes wins, and a damaged slot
/// is passed over for the other. `Ok(None)` when neither slot holds
/// anything. Refused:
///
/// - a slot that decodes to a sound snapshot this log must not take
///   (another object's, or an unknown schema version), without falling
///   back to the other;
/// - both slots damaged, or one damaged and the other empty: the damage;
/// - two slots of one generation that hold different snapshots
///   ([`Damaged`](super::SnapshotErrorKind::Damaged)).
pub fn newest<R: LogRecord>(
    object: ObjectIdentifier,
    slots: [Option<&[u8]>; 2],
) -> Result<Option<NewestSlot<R>>, SnapshotError> {
    let mut newest: Option<NewestSlot<R>> = None;
    let mut passed_over = [None, None];
    let mut damage = None;
    for (slot, octets) in slots.into_iter().enumerate() {
        let Some(octets) = octets else {
            continue;
        };
        match decode::<R>(object, octets) {
            Err(error) if !error.is_damage() => return Err(error),
            Err(error) => {
                passed_over[slot] = Some(error);
                damage = Some(error);
            }
            Ok(snapshot) => match &newest {
                Some(current) if current.generation > snapshot.generation => {}
                Some(current) if current.generation == snapshot.generation => {
                    if current.snapshot != snapshot {
                        return Err(SnapshotError::damaged(
                            "slots hold different snapshots of one generation",
                        )
                        .in_format(codec::WHAT));
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
