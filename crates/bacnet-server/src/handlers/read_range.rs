use std::ops::Range;

use super::*;
use bacnet_objects::log_buffer::LogRecordIdentity;
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use bacnet_types::primitives::{Date, Time};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SignedRangeSelection {
    pub(super) range: Range<usize>,
    pub(super) result_flags: (bool, bool, bool),
}

impl SignedRangeSelection {
    fn empty() -> Self {
        Self {
            range: 0..0,
            result_flags: (false, false, false),
        }
    }

    fn from_range(total: usize, range: Range<usize>) -> Self {
        if range.is_empty() {
            return Self::empty();
        }
        Self {
            result_flags: (range.start == 0, range.end == total, false),
            range,
        }
    }
}

/// Select a signed ReadRange window around an exact zero-based resident ordinal.
///
/// A positive count starts at `reference`; a negative count ends there. Missing
/// references, zero counts, and empty collections select an empty window.
pub(super) fn select_signed_range(
    total: usize,
    reference: Option<usize>,
    count: i32,
) -> SignedRangeSelection {
    let Some(reference) = reference.filter(|reference| *reference < total) else {
        return SignedRangeSelection::empty();
    };
    if count == 0 {
        return SignedRangeSelection::empty();
    }

    let range = if count > 0 {
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        reference..reference.saturating_add(count).min(total)
    } else {
        let count = usize::try_from(count.unsigned_abs()).unwrap_or(usize::MAX);
        let end = reference + 1;
        end.saturating_sub(count)..end
    };
    SignedRangeSelection::from_range(total, range)
}

fn list_item_not_numbered() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: ErrorCode::LIST_ITEM_NOT_NUMBERED.to_raw() as u32,
    }
}

fn list_item_not_timestamped() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: ErrorCode::LIST_ITEM_NOT_TIMESTAMPED.to_raw() as u32,
    }
}

type CivilDateTime = (u16, u8, u8, u8, u8, u8, u8);

fn civil_datetime(date: Date, time: Time) -> Option<CivilDateTime> {
    let year = date.actual_year()?;
    if !(1..=12).contains(&date.month)
        || !(1..=31).contains(&date.day)
        || !(1..=7).contains(&date.day_of_week)
        || !(0..=23).contains(&time.hour)
        || !(0..=59).contains(&time.minute)
        || !(0..=59).contains(&time.second)
        || !(0..=99).contains(&time.hundredths)
    {
        return None;
    }
    Some((
        year,
        date.month,
        date.day,
        time.hour,
        time.minute,
        time.second,
        time.hundredths,
    ))
}

/// Select a signed ReadRange window around the resident timestamp anchor.
///
/// Identity validation is all-or-nothing. The endpoint scan follows resident
/// order, while timestamp comparison ignores day-of-week after validating it.
pub(super) fn select_time_range(
    item_count: usize,
    identities: Option<&[LogRecordIdentity]>,
    reference_time: (Date, Time),
    count: i32,
) -> Result<(SignedRangeSelection, Option<u32>), Error> {
    let identities = identities.ok_or_else(list_item_not_timestamped)?;
    if identities.len() != item_count {
        return Err(list_item_not_timestamped());
    }
    let reference =
        civil_datetime(reference_time.0, reference_time.1).ok_or_else(list_item_not_timestamped)?;
    let timestamps = identities
        .iter()
        .map(|identity| civil_datetime(identity.date(), identity.time()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(list_item_not_timestamped)?;
    let anchor = if count > 0 {
        timestamps
            .iter()
            .position(|timestamp| *timestamp > reference)
    } else {
        timestamps
            .iter()
            .rposition(|timestamp| *timestamp < reference)
    };
    let selection = select_signed_range(item_count, anchor, count);
    let first_sequence_number = identities
        .get(selection.range.start)
        .filter(|_| !selection.range.is_empty())
        .map(LogRecordIdentity::sequence_number);
    Ok((selection, first_sequence_number))
}

pub(super) fn append_read_range_ack_with<F>(
    request: &ReadRangeRequest,
    items: &[PropertyValue],
    selection: &SignedRangeSelection,
    first_sequence_number: Option<u32>,
    response: &mut BytesMut,
    mut encode_item: F,
) -> Result<(), Error>
where
    F: FnMut(&mut BytesMut, &PropertyValue) -> Result<(), Error>,
{
    let selected = &items[selection.range.clone()];
    let mut item_data = BytesMut::new();
    for item in selected {
        encode_item(&mut item_data, item)?;
    }

    let ack = ReadRangeAck {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        result_flags: selection.result_flags,
        item_count: selected.len() as u32,
        item_data: item_data.to_vec(),
        first_sequence_number,
    };
    let mut encoded_ack = BytesMut::new();
    ack.encode(&mut encoded_ack);
    response.extend_from_slice(&encoded_ack);
    Ok(())
}

/// Handle a ReadRange request.
///
/// By Position uses the list's exact one-based order. By Sequence and By Time
/// use aligned resident identities supplied for `LOG_BUFFER` by the object.
pub fn handle_read_range(
    db: &ObjectDatabase,
    service_data: &[u8],
    response: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadRangeRequest::decode(service_data)?;
    let object = db.get(&request.object_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;
    if request.property_array_index.is_some()
        && !object.is_array_property(request.property_identifier)
    {
        return Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
        });
    }
    let value = object.read_property(request.property_identifier, request.property_array_index)?;
    let items = match value {
        PropertyValue::List(items) => items,
        _ => {
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::PROPERTY_IS_NOT_A_LIST.to_raw() as u32,
            });
        }
    };

    let (selection, first_sequence_number) = match &request.range {
        None => (
            SignedRangeSelection::from_range(items.len(), 0..items.len()),
            None,
        ),
        Some(RangeSpec::ByPosition {
            reference_index,
            count,
        }) => {
            let reference = reference_index
                .checked_sub(1)
                .and_then(|reference| usize::try_from(reference).ok());
            (select_signed_range(items.len(), reference, *count), None)
        }
        Some(RangeSpec::BySequenceNumber {
            reference_seq,
            count,
        }) => {
            if request.property_identifier != PropertyIdentifier::LOG_BUFFER {
                return Err(list_item_not_numbered());
            }
            let identities = object
                .log_record_identities_internal()
                .ok_or_else(list_item_not_numbered)?;
            if identities.len() != items.len() {
                return Err(list_item_not_numbered());
            }
            let reference = identities
                .iter()
                .position(|identity| identity.sequence_number() == *reference_seq);
            let selection = select_signed_range(items.len(), reference, *count);
            let first_sequence_number = (!selection.range.is_empty())
                .then(|| identities[selection.range.start].sequence_number());
            (selection, first_sequence_number)
        }
        Some(RangeSpec::ByTime {
            reference_time,
            count,
        }) => {
            if request.property_identifier != PropertyIdentifier::LOG_BUFFER {
                return Err(list_item_not_timestamped());
            }
            let identities = object.log_record_identities_internal();
            select_time_range(items.len(), identities.as_deref(), *reference_time, *count)?
        }
    };

    append_read_range_ack_with(
        &request,
        &items,
        &selection,
        first_sequence_number,
        response,
        encode_property_value,
    )
}
