//! What a CreateObject initial value may set on a new Trend Log that
//! WriteProperty can't change afterwards (#2208).
//!
//! Buffer_Size is read-only over the network, since resizing a log that
//! holds records would have to drop or move them. A new log holds none, so
//! its size can be chosen at creation. Object_Name is read-only too, and a
//! client names the log it creates.

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::TrendLogObject;
use crate::common;
use crate::log_buffer::LogRecordBuffer;

/// The properties [`initialize`] sets.
pub(super) const CREATION_ONLY: &[PropertyIdentifier] = &[
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::BUFFER_SIZE,
];

/// The Buffer_Size a Trend Log built by CreateObject starts with, until a
/// Buffer_Size initial value sets it; the Python binding's default too.
pub const CREATED_BUFFER_SIZE: u32 = 100;

/// The largest Buffer_Size a CreateObject initial value may give a Trend
/// Log. Records are kept in memory, so a client can't ask for an unbounded
/// buffer; an application builds a larger log itself.
pub const MAX_CREATED_BUFFER_SIZE: u32 = 10_000;

/// Apply an Object_Name initial value to `log`, a non-empty
/// CharacterString, or a Buffer_Size one, a log that holds no record
/// yet: an Unsigned up to [`MAX_CREATED_BUFFER_SIZE`], zero included (a log
/// that only counts). A larger value is VALUE_OUT_OF_RANGE and any other
/// datatype INVALID_DATA_TYPE. A log that has taken records is
/// WRITE_ACCESS_DENIED, though the server only asks this of a new one. On a
/// refusal the log is unchanged.
pub(super) fn initialize(
    log: &mut TrendLogObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    if property == PropertyIdentifier::OBJECT_NAME {
        return common::initialize_object_name(&mut log.name, &value);
    }
    if property != PropertyIdentifier::BUFFER_SIZE {
        return Err(common::write_access_denied_error());
    }
    let PropertyValue::Unsigned(size) = value else {
        return Err(common::invalid_data_type_error());
    };
    let size = u32::try_from(size)
        .ok()
        .filter(|size| *size <= MAX_CREATED_BUFFER_SIZE)
        .ok_or_else(common::value_out_of_range_error)?;
    if log.log_buffer.total_record_count() != 0 || !log.records().is_empty() {
        return Err(common::write_access_denied_error());
    }
    log.buffer_size = size;
    log.log_buffer = LogRecordBuffer::new(size);
    Ok(())
}
