//! The Trend Log, Trend Log Multiple and Event Log snapshot slot (`RBLOGS01`),
//! as `FileLogPersistence` (with std) writes each of its two slots.
//!
//! A slot is a 26-octet header (the magic tag, the schema version, the
//! object identifier, the generation and the payload length), the payload,
//! and a CRC-32 of everything before it. Version 2 ends the payload with the
//! written reporting settings; a version 1 slot still loads, with none
//! written. [`decode`] checks everything [`encode`] does, so a slot that
//! loads always saves again.
//!
//! The file backend keeps two slots and writes the one the generation's
//! parity names, so a commit cut off part way leaves the other to load. A
//! backend of its own can do the same: on load, take the newer slot that
//! decodes, pass over one whose error [is
//! damage](super::SnapshotError::is_damage), and stop at one whose error
//! isn't.

use crate::prelude::*;
use bacnet_types::constructed::{BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord};
use bacnet_types::primitives::ObjectIdentifier;

use super::SnapshotError;
use crate::log_persistence::LogSnapshot;
use crate::log_persistence::codec;

/// The magic tag every slot starts with.
pub const MAGIC: [u8; 8] = *codec::MAGIC;
/// The schema version [`encode`] writes.
pub const VERSION: u16 = codec::SCHEMA_VERSION;
/// The oldest schema version [`decode`] still reads.
pub const OLDEST_VERSION: u16 = codec::SCHEMA_VERSION_1;
/// The most octets a slot may take.
pub const MAX_LEN: usize = codec::MAX_SNAPSHOT_BYTES;

/// A log record family a slot can hold: [`BACnetLogRecord`] for a Trend Log,
/// [`BACnetLogMultipleRecord`] for a Trend Log Multiple and
/// [`BACnetEventLogRecord`] for an Event Log. Sealed: these three are all.
pub trait LogRecord: sealed::Sealed {}

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
/// Refuses damage (a slot cut short, a bad magic tag, length or checksum,
/// or contents that don't decode or don't hold together), and a sound slot
/// of another object or of a schema version past [`VERSION`].
pub fn decode<R: LogRecord>(
    octets: &[u8],
    object: ObjectIdentifier,
) -> Result<LogSnapshot<R>, SnapshotError> {
    R::decode(octets, object)
}
