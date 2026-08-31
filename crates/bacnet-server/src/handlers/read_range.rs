use std::ops::Range;

use super::*;
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};

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
/// By Position uses the list's exact one-based order. By Sequence is available
/// only for `LOG_BUFFER` when the object supplies aligned resident identities.
/// By Time remains explicitly unsupported by this handler.
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
        Some(RangeSpec::ByTime { .. }) => {
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            });
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
