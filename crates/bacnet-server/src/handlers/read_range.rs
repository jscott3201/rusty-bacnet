use std::ops::Range;

use super::group_present_value::GroupMembers;
use super::*;
use bacnet_objects::log_buffer::{LogRecordIdentity, TimestampKey};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use bacnet_types::primitives::{Date, Time};

#[path = "read_range_logs.rs"]
mod logs;
pub(crate) use logs::{LogIdentities, RangeItems};
#[path = "read_range_items.rs"]
mod items;
#[path = "read_range_page.rs"]
mod page;
pub(crate) use page::ReadRangeFailure;

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

/// Select a signed ReadRange window around the resident timestamp anchor.
///
/// Timestamp validation is all-or-nothing. The anchor follows resident
/// order, while timestamp comparison ignores day-of-week after validating it
/// ([`TimestampKey`]); [`LogIdentities::time_anchor`] bisects a log whose
/// timestamps run in order and visits each record of any other.
pub(super) fn select_time_range(
    item_count: usize,
    identities: Option<&LogIdentities<'_>>,
    reference_time: (Date, Time),
    count: i32,
) -> Result<(SignedRangeSelection, Option<u64>), Error> {
    let identities = identities.ok_or_else(list_item_not_timestamped)?;
    if identities.len() != item_count {
        return Err(list_item_not_timestamped());
    }
    let reference = TimestampKey::of(reference_time.0, reference_time.1)
        .ok_or_else(list_item_not_timestamped)?;
    let anchor = identities.time_anchor(reference, count)?;
    let selection = select_signed_range(item_count, anchor, count);
    let first_sequence_number =
        (!selection.range.is_empty()).then(|| identities.sequence_number(selection.range.start));
    Ok((selection, first_sequence_number))
}

pub(super) fn append_read_range_ack_with<F>(
    request: &ReadRangeRequest,
    items: &RangeItems<'_>,
    selection: &SignedRangeSelection,
    first_sequence_number: Option<u64>,
    response: &mut BytesMut,
    mut encode_item: F,
) -> Result<(), Error>
where
    F: FnMut(&mut BytesMut, &PropertyValue) -> Result<(), Error>,
{
    let mut item_data = BytesMut::new();
    for index in selection.range.clone() {
        items.encode_with(index, &mut item_data, &mut encode_item)?;
    }

    let ack = ReadRangeAck {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        result_flags: selection.result_flags,
        item_count: selection.range.len() as u32,
        item_data: item_data.to_vec(),
        first_sequence_number,
    };
    let mut encoded_ack = BytesMut::new();
    ack.encode(&mut encoded_ack);
    response.extend_from_slice(&encoded_ack);
    Ok(())
}

/// Handle a ReadRange request against standalone object data.
///
/// By Position uses the list's exact one-based order. By Sequence and By Time
/// use aligned resident identities supplied for `LOG_BUFFER` by the object. A
/// built-in log's `LOG_BUFFER`, which ReadProperty refuses, comes from its
/// record store: each item is one encoded record.
/// Like [`handle_read_property`], this low-level helper has no executor
/// context: a built-in Device's COV subscription lists read as the object
/// holds them, empty. Running server reads page the live subscription table.
pub fn handle_read_range(
    db: &ObjectDatabase,
    service_data: &[u8],
    response: &mut BytesMut,
) -> Result<(), Error> {
    let unlimited = |failure| match failure {
        ReadRangeFailure::Service(error) => error,
        ReadRangeFailure::Bytes | ReadRangeFailure::Work => {
            unreachable!("a read with no limit ran past one")
        }
    };
    let request = ReadRangeRequest::decode(service_data).map_err(Error::into_request_reject)?;
    let plan = plan_read_range(db, None, &request).map_err(unlimited)?;
    let selected = prepare_read_range(db, None, request, plan.into_members()).map_err(unlimited)?;
    append_read_range_ack_with(
        &selected.request,
        &selected.items,
        &selected.selection,
        selected.first_sequence_number,
        response,
        encode_property_value,
    )
}

#[cfg(test)]
pub(crate) fn handle_read_range_budgeted(
    db: &ObjectDatabase,
    service_data: &[u8],
    response: &mut BytesMut,
    budget: crate::server::ReadRangeBudget,
) -> Result<(), ReadRangeFailure> {
    let request = ReadRangeRequest::decode(service_data)
        .map_err(|error| ReadRangeFailure::Service(error.into_request_reject()))?;
    let plan = plan_read_range(db, None, &request)?;
    read_range_request_observed(db, None, request, plan, response, budget, |_, _, _, _| {})
}

/// Plan a ReadRange of the object the request names: its row and, for a
/// Group's whole Present_Value, the member rows, charged to the view's work
/// limit before any value is read (#1172, #1213).
pub(crate) fn plan_read_range(
    db: &ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    request: &ReadRangeRequest,
) -> Result<PropertyPlan, ReadRangeFailure> {
    Ok(PropertyPlan::new(
        db,
        view,
        request.object_identifier,
        db.get(&request.object_identifier),
        request.property_identifier,
        request.property_array_index,
    )?)
}

/// ReadRange evaluator over one decoded request and its plan. `view` carries
/// the executor-owned Device definitions and the request-local COV lists that
/// ReadProperty serves, so both services read the same value.
///
/// Observe one request outcome, never a page-budget or work-limit failure.
/// The hook captures identity/result only; delivery must follow guard release.
pub(crate) fn read_range_request_observed(
    db: &ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    request: ReadRangeRequest,
    plan: PropertyPlan,
    response: &mut BytesMut,
    budget: crate::server::ReadRangeBudget,
    completed: impl FnOnce(ObjectIdentifier, PropertyIdentifier, Option<u32>, &Result<(), Error>),
) -> Result<(), ReadRangeFailure> {
    let target = request.object_identifier;
    let property = request.property_identifier;
    let index = request.property_array_index;
    let result = prepare_read_range(db, view, request, plan.into_members()).and_then(|selected| {
        page::append_page_with(&selected, response, budget, encode_property_value)
    });
    let result = match result {
        Ok(()) => Ok(()),
        Err(ReadRangeFailure::Service(error)) => Err(error),
        Err(failure) => return Err(failure),
    };
    completed(target, property, index, &result);
    result.map_err(ReadRangeFailure::Service)
}

struct PreparedReadRange<'a> {
    request: ReadRangeRequest,
    items: RangeItems<'a>,
    selection: SignedRangeSelection,
    first_sequence_number: Option<u64>,
    identities: Option<LogIdentities<'a>>,
}

/// Resolve the ReadRange target to its list items, in the order the service
/// procedure of Clause 15.8 implies and #999 follows for the list services:
/// the object (which the caller has found), the property, a supplied array
/// index, and then whether the target is a BACnetLIST at all. The last check
/// follows the property's datatype, never the shape of the value read: a
/// whole array also reads as a list, and a constructed single value can read
/// as framed bytes.
fn read_range_items(
    db: &ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    object: &dyn BACnetObject,
    request: &ReadRangeRequest,
    members: Option<GroupMembers>,
) -> Result<Vec<PropertyValue>, ReadRangeFailure> {
    let property = request.property_identifier;
    let index = request.property_array_index;
    if index.is_some() && !object.is_array_property(property) {
        // Read the whole property first so an unknown one keeps its error.
        object.read_property(property, None)?;
        return Err(ReadRangeFailure::Service(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
        }));
    }
    let value =
        group_present_value::read_served_property(db, view, object, property, index, members)?;
    // An indexed array element is never a list: 135-2020 defines no
    // BACnetARRAY of BACnetLIST property.
    if index.is_some() || !object.is_list_property(property) {
        return Err(ReadRangeFailure::Service(Error::Protocol {
            class: ErrorClass::SERVICES.to_raw() as u32,
            code: ErrorCode::PROPERTY_IS_NOT_A_LIST.to_raw() as u32,
        }));
    }
    Ok(items::list_items(
        request.object_identifier.object_type(),
        property,
        value,
    )?)
}

/// Select the request's page from one read of the target list. A running
/// server's `view` serves the Device's COV subscription lists from the
/// snapshot it took for this request, so every item, flag and count comes
/// from the same instant. A log's Log_Buffer is paged straight from the
/// stored object's records, which the Device view has no part in.
fn prepare_read_range<'a>(
    db: &'a ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    request: ReadRangeRequest,
    members: Option<GroupMembers>,
) -> Result<PreparedReadRange<'a>, ReadRangeFailure> {
    let stored = db.get(&request.object_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;
    let served = view.map(|view| view.object(stored));
    let object: &dyn BACnetObject = served.as_ref().map_or(stored, |served| served);
    let (items, mut buffer_identities) = match logs::log_buffer(stored, &request)? {
        Some((items, identities)) => (items, Some(identities)),
        None => (
            RangeItems::Values(read_range_items(db, view, object, &request, members)?),
            None,
        ),
    };
    let mut log_identities = || {
        buffer_identities.take().or_else(|| {
            object
                .log_record_identities_internal()
                .map(LogIdentities::Listed)
        })
    };

    let mut resident_identities = None;
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
                return Err(list_item_not_numbered().into());
            }
            let identities = log_identities().ok_or_else(list_item_not_numbered)?;
            if identities.len() != items.len() {
                return Err(list_item_not_numbered().into());
            }
            let reference = identities.position(*reference_seq)?;
            let selection = select_signed_range(items.len(), reference, *count);
            let first_sequence_number = (!selection.range.is_empty())
                .then(|| identities.sequence_number(selection.range.start));
            resident_identities = Some(identities);
            (selection, first_sequence_number)
        }
        Some(RangeSpec::ByTime {
            reference_time,
            count,
        }) => {
            if request.property_identifier != PropertyIdentifier::LOG_BUFFER {
                return Err(list_item_not_timestamped().into());
            }
            resident_identities = log_identities();
            select_time_range(
                items.len(),
                resident_identities.as_ref(),
                *reference_time,
                *count,
            )?
        }
    };

    Ok(PreparedReadRange {
        request,
        items,
        selection,
        first_sequence_number,
        identities: resident_identities,
    })
}
