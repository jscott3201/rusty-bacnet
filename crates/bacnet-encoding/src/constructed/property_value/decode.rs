//! The `BACnetPropertyValue` decoder, which reports how far it got when a
//! value is malformed so a service can choose its reject reason.

use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::{PropertyIdentifier, RejectReason};
use bacnet_types::error::{DecodeError, DecodingKind, Error};

use super::{extract_property_value, matches_property_boundary, PropertyValueBoundary};
use crate::constructed::tagged::misplaced_kind;
use crate::{primitives, tags};

/// The member a property value decode was reading when it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyValueDecodeStage {
    /// The property identifier, context `[0]`.
    PropertyIdentifier,
    /// The optional array index, context `[1]`.
    ArrayIndex,
    /// The value inside the `[2]` pair.
    Value,
    /// The optional priority, context `[3]`, or what follows the value.
    Priority,
}

/// Why a property value failed to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyValueDecodeFailure {
    /// The encoding is malformed; the reason a service would reject it with.
    Syntax(RejectReason),
    /// The priority is a well-formed Unsigned outside 1-16.
    PriorityOutOfRange,
}

/// A failed property value decode, with what was read before the failure.
#[derive(Debug)]
pub struct PropertyValueDecodeError {
    /// The decode error.
    pub error: Error,
    /// Offset of the failure in the input.
    pub offset: usize,
    /// The member being read.
    pub stage: PropertyValueDecodeStage,
    /// Why the decode failed.
    pub kind: PropertyValueDecodeFailure,
    /// The property identifier, once it was read.
    pub property_identifier: Option<PropertyIdentifier>,
    /// The array index, once it was read.
    pub property_array_index: Option<u32>,
    /// Whether the property reference (identifier and optional index) was
    /// read in full before the failure.
    pub reference_complete: bool,
}

impl PropertyValueDecodeError {
    fn syntax(
        error: Error,
        offset: usize,
        stage: PropertyValueDecodeStage,
        reject_reason: RejectReason,
        property_identifier: Option<PropertyIdentifier>,
        property_array_index: Option<u32>,
        reference_complete: bool,
    ) -> Self {
        Self {
            error,
            offset,
            stage,
            kind: PropertyValueDecodeFailure::Syntax(reject_reason),
            property_identifier,
            property_array_index,
            reference_complete,
        }
    }
}

/// The Reject reason a decoder's refusal draws, the one the server gives
/// every confirmed request (#1446; the table on
/// [`DecodingKind::reject_reason`](bacnet_types::error::DecodingKind::reject_reason)).
fn reason_of(error: &Error) -> RejectReason {
    error
        .reject_reason()
        .unwrap_or(RejectReason::INVALID_DATA_ENCODING)
}

fn error_offset(error: &Error, fallback: usize) -> usize {
    error.decode_fault().map_or(fallback, |(offset, _)| offset)
}

/// Decode a property value at `offset` in `data`; returns it and the offset just past it.
///
/// The value may be followed by the end of the input, the next value's
/// context `[0]`, or a context `[3]` that runs to the end.
pub fn decode_bacnet_property_value(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetPropertyValue, usize), Error> {
    decode_with_boundaries(
        data,
        offset,
        &[
            PropertyValueBoundary::End,
            PropertyValueBoundary::Context(0),
            PropertyValueBoundary::ContextToEnd(3),
        ],
    )
}

/// Decode one property value of a list that ends with the closing tag
/// `closing_tag`; returns it and the offset just past it.
pub fn decode_bacnet_property_value_in_list(
    data: &[u8],
    offset: usize,
    closing_tag: u8,
) -> Result<(BACnetPropertyValue, usize), Error> {
    decode_bacnet_property_value_in_list_detailed(data, offset, closing_tag)
        .map_err(|error| error.error)
}

/// [`decode_bacnet_property_value_in_list`], reporting the stage, the reject
/// reason and the reference read so far when the value is malformed.
pub fn decode_bacnet_property_value_in_list_detailed(
    data: &[u8],
    offset: usize,
    closing_tag: u8,
) -> Result<(BACnetPropertyValue, usize), PropertyValueDecodeError> {
    decode_with_boundaries_detailed(
        data,
        offset,
        &[
            PropertyValueBoundary::Context(3),
            PropertyValueBoundary::Context(0),
            PropertyValueBoundary::Closing(closing_tag),
        ],
    )
}

fn decode_with_boundaries(
    data: &[u8],
    offset: usize,
    boundaries: &[PropertyValueBoundary],
) -> Result<(BACnetPropertyValue, usize), Error> {
    decode_with_boundaries_detailed(data, offset, boundaries).map_err(|error| error.error)
}

fn decode_with_boundaries_detailed(
    data: &[u8],
    offset: usize,
    boundaries: &[PropertyValueBoundary],
) -> Result<(BACnetPropertyValue, usize), PropertyValueDecodeError> {
    let start = offset;
    let (tag, content_start) = tags::decode_tag(data, offset).map_err(|error| {
        let reason = reason_of(&error);
        PropertyValueDecodeError::syntax(
            error,
            offset,
            PropertyValueDecodeStage::PropertyIdentifier,
            reason,
            None,
            None,
            false,
        )
    })?;
    if !tag.is_context(0) {
        let kind = misplaced_kind(data, offset, &tag, Some(0));
        return Err(PropertyValueDecodeError::syntax(
            Error::decoding_kind(
                kind,
                offset,
                "BACnetPropertyValue property-id expected context tag 0",
            ),
            offset,
            PropertyValueDecodeStage::PropertyIdentifier,
            kind.reject_reason(),
            None,
            None,
            false,
        ));
    }
    let property_end = content_start
        .checked_add(tag.length as usize)
        .ok_or_else(|| {
            PropertyValueDecodeError::syntax(
                Error::decoding(
                    content_start,
                    "BACnetPropertyValue property-id length overflow",
                ),
                content_start,
                PropertyValueDecodeStage::PropertyIdentifier,
                RejectReason::INVALID_DATA_ENCODING,
                None,
                None,
                false,
            )
        })?;
    if property_end > data.len() {
        return Err(PropertyValueDecodeError::syntax(
            Error::buffer_too_short(property_end, data.len()),
            content_start,
            PropertyValueDecodeStage::PropertyIdentifier,
            RejectReason::MISSING_REQUIRED_PARAMETER,
            None,
            None,
            false,
        ));
    }
    let prop_id =
        primitives::decode_unsigned(&data[content_start..property_end]).map_err(|error| {
            PropertyValueDecodeError::syntax(
                error,
                content_start,
                PropertyValueDecodeStage::PropertyIdentifier,
                RejectReason::INVALID_DATA_ENCODING,
                None,
                None,
                false,
            )
        })?;
    let prop_id = u32::try_from(prop_id).map_err(|_| {
        PropertyValueDecodeError::syntax(
            Error::out_of_range(start, "BACnetPropertyValue property-id exceeds u32"),
            start,
            PropertyValueDecodeStage::PropertyIdentifier,
            RejectReason::PARAMETER_OUT_OF_RANGE,
            None,
            None,
            false,
        )
    })?;
    let property_identifier = PropertyIdentifier::from_raw(prop_id);
    let mut offset = property_end;

    let mut array_index = None;
    if offset < data.len() {
        let (tag, content_start) = tags::decode_tag(data, offset).map_err(|error| {
            let reason = reason_of(&error);
            PropertyValueDecodeError::syntax(
                error,
                offset,
                PropertyValueDecodeStage::ArrayIndex,
                reason,
                Some(property_identifier),
                None,
                false,
            )
        })?;
        if tag.is_context(1) {
            let end = content_start
                .checked_add(tag.length as usize)
                .ok_or_else(|| {
                    PropertyValueDecodeError::syntax(
                        Error::decoding(
                            content_start,
                            "BACnetPropertyValue array-index length overflow",
                        ),
                        content_start,
                        PropertyValueDecodeStage::ArrayIndex,
                        RejectReason::INVALID_DATA_ENCODING,
                        Some(property_identifier),
                        None,
                        false,
                    )
                })?;
            if end > data.len() {
                return Err(PropertyValueDecodeError::syntax(
                    Error::buffer_too_short(end, data.len()),
                    content_start,
                    PropertyValueDecodeStage::ArrayIndex,
                    RejectReason::MISSING_REQUIRED_PARAMETER,
                    Some(property_identifier),
                    None,
                    false,
                ));
            }
            let value =
                primitives::decode_unsigned(&data[content_start..end]).map_err(|error| {
                    PropertyValueDecodeError::syntax(
                        error,
                        content_start,
                        PropertyValueDecodeStage::ArrayIndex,
                        RejectReason::INVALID_DATA_ENCODING,
                        Some(property_identifier),
                        None,
                        false,
                    )
                })?;
            let value = u32::try_from(value).map_err(|_| {
                PropertyValueDecodeError::syntax(
                    Error::out_of_range(offset, "BACnetPropertyValue array-index exceeds u32"),
                    offset,
                    PropertyValueDecodeStage::ArrayIndex,
                    RejectReason::PARAMETER_OUT_OF_RANGE,
                    Some(property_identifier),
                    None,
                    false,
                )
            })?;
            array_index = Some(value);
            offset = end;
        }
    }

    let (tag, tag_end) = tags::decode_tag(data, offset).map_err(|error| {
        let reason = reason_of(&error);
        PropertyValueDecodeError::syntax(
            error,
            offset,
            PropertyValueDecodeStage::Value,
            reason,
            Some(property_identifier),
            array_index,
            true,
        )
    })?;
    if !tag.is_opening_tag(2) {
        let kind = misplaced_kind(data, offset, &tag, Some(2));
        return Err(PropertyValueDecodeError::syntax(
            Error::decoding_kind(kind, offset, "BACnetPropertyValue expected opening tag 2"),
            offset,
            PropertyValueDecodeStage::Value,
            kind.reject_reason(),
            Some(property_identifier),
            array_index,
            true,
        ));
    }
    let (value_bytes, offset) =
        extract_property_value(data, tag_end, 2, property_identifier, boundaries).map_err(
            |error| {
                let reject_reason = reason_of(&error);
                let offset = error_offset(&error, tag_end);
                PropertyValueDecodeError::syntax(
                    error,
                    offset,
                    PropertyValueDecodeStage::Value,
                    reject_reason,
                    Some(property_identifier),
                    array_index,
                    true,
                )
            },
        )?;
    let value = value_bytes.to_vec();

    let mut priority = None;
    if offset < data.len() {
        let (tag, new_pos) = tags::decode_tag(data, offset).map_err(|error| {
            let reason = reason_of(&error);
            PropertyValueDecodeError::syntax(
                error,
                offset,
                PropertyValueDecodeStage::Priority,
                reason,
                Some(property_identifier),
                array_index,
                true,
            )
        })?;
        if tag.is_context(3) {
            let end = new_pos.checked_add(tag.length as usize).ok_or_else(|| {
                PropertyValueDecodeError::syntax(
                    Error::decoding(new_pos, "BACnetPropertyValue priority length overflow"),
                    new_pos,
                    PropertyValueDecodeStage::Priority,
                    RejectReason::INVALID_DATA_ENCODING,
                    Some(property_identifier),
                    array_index,
                    true,
                )
            })?;
            if end > data.len() {
                return Err(PropertyValueDecodeError::syntax(
                    Error::buffer_too_short(end, data.len()),
                    new_pos,
                    PropertyValueDecodeStage::Priority,
                    RejectReason::MISSING_REQUIRED_PARAMETER,
                    Some(property_identifier),
                    array_index,
                    true,
                ));
            }
            let prio = primitives::decode_unsigned(&data[new_pos..end]).map_err(|error| {
                PropertyValueDecodeError::syntax(
                    error,
                    new_pos,
                    PropertyValueDecodeStage::Priority,
                    RejectReason::INVALID_DATA_ENCODING,
                    Some(property_identifier),
                    array_index,
                    true,
                )
            })?;
            if !(1..=16).contains(&prio) {
                return Err(PropertyValueDecodeError {
                    error: DecodeError::new(
                        DecodingKind::OutOfRange,
                        new_pos,
                        "BACnetPropertyValue priority {} out of range 1-16",
                    )
                    .with_value(prio)
                    .into(),
                    offset: new_pos,
                    stage: PropertyValueDecodeStage::Priority,
                    kind: PropertyValueDecodeFailure::PriorityOutOfRange,
                    property_identifier: Some(property_identifier),
                    property_array_index: array_index,
                    reference_complete: true,
                });
            }
            priority = Some(prio as u8);
            if boundaries
                .iter()
                .any(|boundary| matches_property_boundary(data, end, *boundary))
            {
                return Ok((
                    BACnetPropertyValue {
                        property_identifier,
                        property_array_index: array_index,
                        value,
                        priority,
                    },
                    end,
                ));
            }
            return Err(boundary_error(data, end, property_identifier, array_index));
        }
    }

    if !boundaries
        .iter()
        .any(|boundary| matches_property_boundary(data, offset, *boundary))
    {
        return Err(boundary_error(
            data,
            offset,
            property_identifier,
            array_index,
        ));
    }

    Ok((
        BACnetPropertyValue {
            property_identifier,
            property_array_index: array_index,
            value,
            priority,
        },
        offset,
    ))
}

fn boundary_error(
    data: &[u8],
    offset: usize,
    property_identifier: PropertyIdentifier,
    property_array_index: Option<u32>,
) -> PropertyValueDecodeError {
    let error = if offset >= data.len() {
        Error::missing(offset, "BACnetPropertyValue is missing its list boundary")
    } else {
        match tags::decode_tag(data, offset) {
            Ok(_) => {
                Error::invalid_tag(offset, "BACnetPropertyValue has an unexpected trailing tag")
            }
            Err(error) => error,
        }
    };
    let reject_reason = reason_of(&error);
    PropertyValueDecodeError::syntax(
        error,
        offset,
        PropertyValueDecodeStage::Priority,
        reject_reason,
        Some(property_identifier),
        property_array_index,
        true,
    )
}
