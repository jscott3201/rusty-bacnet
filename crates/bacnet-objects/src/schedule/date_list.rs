//! The Calendar object's Date_List (Clause 12.9): a BACnetLIST of
//! `BACnetCalendarEntry` (Clause 21), each entry under its own CHOICE context
//! tag (#996).
//!
//! A read is a `List` holding one `ApplicationData` per entry, so the services
//! put the concatenated entries on the wire and ReadRange can address each
//! entry by position. A write arrives in the same shape (the service decoders
//! split a payload at tag boundaries, one element per entry) or, from a local
//! write of one entry or a pre-encoded list, as one `ApplicationData`.
//! AddListElement and RemoveListElement edit that list and write it back
//! through the same path.
//!
//! Error pairings follow Clause 15.9.1.3, which separates a value of the wrong
//! datatype from a malformed encoding and from a value out of range: an
//! element that isn't a calendar entry at all (an application-tagged Date, the
//! form this property used to carry, or a context tag other than `[0]`, `[1]`
//! or `[2]`) is INVALID_DATA_TYPE; an entry tag whose content doesn't decode is
//! INVALID_DATA_ENCODING; an entry holding an octet outside its Clause 21
//! range ([`BACnetCalendarEntry::is_valid`]: month 1-14, week-of-month 1-9,
//! weekday 1-7, day 1-34, and date-range ends that are specific dates or
//! wholly unspecified) is VALUE_OUT_OF_RANGE (#1029); and more than
//! [`MAX_DATE_LIST_ENTRIES`] entries is NO_SPACE_TO_WRITE_PROPERTY. Entries
//! are checked in order, so the first offending entry decides the error.

use crate::prelude::*;
use bacnet_encoding::constructed::{decode_calendar_entry, encode_calendar_entry};
use bacnet_encoding::tags::{self, TagClass};
use bacnet_types::constructed::BACnetCalendarEntry;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use crate::common;

/// Resource cap on Date_List entries, the same bound `MAX_ALARM_VALUES` puts
/// on Alarm_Values.
pub(crate) const MAX_DATE_LIST_ENTRIES: usize = 1024;

/// The error for one entry more than [`MAX_DATE_LIST_ENTRIES`].
pub(super) fn no_space_error() -> Error {
    Error::protocol(ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)
}

/// The property value of a Date_List: one encoded entry per list element.
pub(super) fn read(entries: &[BACnetCalendarEntry]) -> PropertyValue {
    PropertyValue::List(
        entries
            .iter()
            .map(|entry| {
                let mut encoded = BytesMut::new();
                encode_calendar_entry(&mut encoded, entry);
                PropertyValue::ApplicationData(encoded.to_vec())
            })
            .collect(),
    )
}

/// Decode and validate a whole-list Date_List write. A refused entry is
/// named by its position among the entries decoded so far, however the write
/// groups them into elements (#1048).
pub(super) fn decode_write(value: PropertyValue) -> Result<Vec<BACnetCalendarEntry>, Error> {
    let elements = match value {
        PropertyValue::List(elements) => elements,
        element @ PropertyValue::ApplicationData(_) => vec![element],
        _ => return Err(common::invalid_data_type_error()),
    };
    let mut entries = Vec::new();
    for element in elements {
        // The entry being decoded is the next one: index `entries.len()`.
        let PropertyValue::ApplicationData(bytes) = element else {
            let error = common::invalid_data_type_error();
            return Err(common::at_list_element(error, entries.len()));
        };
        let mut offset = 0;
        while offset < bytes.len() {
            if entries.len() == MAX_DATE_LIST_ENTRIES {
                return Err(common::at_list_element(no_space_error(), entries.len()));
            }
            let (entry, end) = decode_entry(&bytes, offset)
                .map_err(|error| common::at_list_element(error, entries.len()))?;
            if !entry.is_valid() {
                let error = common::value_out_of_range_error();
                return Err(common::at_list_element(error, entries.len()));
            }
            entries.push(entry);
            offset = end;
        }
    }
    Ok(entries)
}

/// Decode one entry, telling a foreign datatype from a malformed entry.
fn decode_entry(bytes: &[u8], offset: usize) -> Result<(BACnetCalendarEntry, usize), Error> {
    match tags::decode_tag(bytes, offset) {
        Ok((tag, _)) if tag.class == TagClass::Context && tag.number <= 2 => {}
        Ok(_) => return Err(common::invalid_data_type_error()),
        Err(_) => return Err(common::invalid_data_encoding_error()),
    }
    decode_calendar_entry(bytes, offset).map_err(|_| common::invalid_data_encoding_error())
}
