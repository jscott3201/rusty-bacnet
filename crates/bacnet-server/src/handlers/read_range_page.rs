//! Directional, contiguous ReadRange pages after all selection validation.
use super::*;
use crate::server::ReadRangeBudget;

#[cfg(test)]
#[path = "tests/read_range_page_encoding.rs"]
mod tests;

#[derive(Debug)]
pub(crate) enum ReadRangeFailure {
    Service(Error),
    Bytes,
}

fn negative(range: &Option<RangeSpec>) -> bool {
    matches!(range,
        Some(RangeSpec::ByPosition { count, .. }
        | RangeSpec::BySequenceNumber { count, .. }
        | RangeSpec::ByTime { count, .. }) if *count < 0)
}

fn envelope(
    selected: &PreparedReadRange,
    range: Range<usize>,
) -> Result<ReadRangeAck, ReadRangeFailure> {
    let count = u32::try_from(range.len()).map_err(|_| ReadRangeFailure::Bytes)?;
    Ok(ReadRangeAck {
        object_identifier: selected.request.object_identifier,
        property_identifier: selected.request.property_identifier,
        property_array_index: selected.request.property_array_index,
        result_flags: if range.is_empty() {
            (false, false, false)
        } else {
            (
                range.start == 0,
                range.end == selected.items.len(),
                range != selected.selection.range,
            )
        },
        item_count: count,
        item_data: Vec::new(),
        first_sequence_number: selected
            .identities
            .as_ref()
            .filter(|_| count != 0)
            .map(|ids| ids[range.start].sequence_number()),
    })
}

pub(super) fn append_page_with<F>(
    selected: &PreparedReadRange,
    response: &mut BytesMut,
    budget: ReadRangeBudget,
    mut encode_item: F,
) -> Result<(), ReadRangeFailure>
where
    F: FnMut(&mut BytesMut, &PropertyValue) -> Result<(), Error>,
{
    let backwards = negative(&selected.request.range);
    let matching = selected.selection.range.clone();
    let mut range = if backwards {
        matching.end..matching.end
    } else {
        matching.start..matching.start
    };
    let mut ack = envelope(selected, range.clone())?;
    let mut header = BytesMut::new();
    ack.encode(&mut header);
    if header.len() > budget.max_service_ack_bytes {
        return Err(ReadRangeFailure::Bytes);
    }

    // Store each accepted encoding once. Backward traversal reverses chunks, not
    // bytes, at final assembly; prepending/re-encoding the growing page is quadratic.
    let mut chunks = Vec::new();
    let mut accepted_bytes = 0usize;
    for offset in 0..matching.len() {
        if chunks.len() >= budget.max_returned_items || chunks.len() >= u32::MAX as usize {
            break;
        }
        let index = if backwards {
            matching.end - 1 - offset
        } else {
            matching.start + offset
        };
        let candidate = if backwards {
            index..matching.end
        } else {
            matching.start..index + 1
        };
        let next_ack = envelope(selected, candidate.clone())?;
        header.clear();
        next_ack.encode(&mut header);
        if header.len() > budget.max_service_ack_bytes {
            break;
        }
        let mut item = BytesMut::new();
        encode_item(&mut item, &selected.items[index]).map_err(ReadRangeFailure::Service)?;
        let Some(bytes) = accepted_bytes.checked_add(item.len()) else {
            break;
        };
        if bytes > budget.max_service_ack_bytes - header.len() {
            break;
        }
        chunks.push(item);
        accepted_bytes = bytes;
        range = candidate;
        ack = next_ack;
    }
    if !matching.is_empty() && range.is_empty() {
        return Err(ReadRangeFailure::Bytes);
    }
    if backwards {
        chunks.reverse();
    }
    ack.item_data = Vec::with_capacity(accepted_bytes);
    for chunk in chunks {
        ack.item_data.extend_from_slice(&chunk);
    }
    let mut encoded = BytesMut::new();
    ack.encode(&mut encoded);
    debug_assert!(encoded.len() <= budget.max_service_ack_bytes);
    response.extend_from_slice(&encoded);
    Ok(())
}
