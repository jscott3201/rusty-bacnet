use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::{PropertyIdentifier, RejectReason};
use bacnet_types::error::Error;

use super::{
    extract_property_value, matches_property_boundary, BACnetPropertyValue, PropertyValueBoundary,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropertyValueDecodeStage {
    PropertyIdentifier,
    ArrayIndex,
    Value,
    Priority,
}

#[derive(Debug)]
pub(crate) struct PropertyValueDecodeError {
    pub(crate) error: Error,
    pub(crate) offset: usize,
    pub(crate) stage: PropertyValueDecodeStage,
    pub(crate) reject_reason: RejectReason,
    pub(crate) property_identifier: Option<PropertyIdentifier>,
    pub(crate) property_array_index: Option<u32>,
    pub(crate) reference_complete: bool,
}

impl PropertyValueDecodeError {
    fn new(
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
            reject_reason,
            property_identifier,
            property_array_index,
            reference_complete,
        }
    }
}

fn tag_failure_reason(data: &[u8], offset: usize) -> RejectReason {
    if offset >= data.len() {
        RejectReason::MISSING_REQUIRED_PARAMETER
    } else {
        RejectReason::INVALID_DATA_ENCODING
    }
}

fn wrong_tag_reason(tag: &tags::Tag) -> RejectReason {
    if tag.class == tags::TagClass::Application {
        RejectReason::INVALID_PARAMETER_DATA_TYPE
    } else {
        RejectReason::INVALID_TAG
    }
}

fn error_offset(error: &Error, fallback: usize) -> usize {
    match error {
        Error::Decoding { offset, .. } => *offset,
        _ => fallback,
    }
}

fn value_failure_reason(error: &Error) -> RejectReason {
    match error {
        Error::Decoding { message, .. } if message.contains("missing closing") => {
            RejectReason::MISSING_REQUIRED_PARAMETER
        }
        Error::Decoding { message, .. }
            if message.contains("does not match")
                || message.contains("nesting depth")
                || message.contains("ambiguous") =>
        {
            RejectReason::INVALID_TAG
        }
        Error::InvalidTag(_) => RejectReason::INVALID_TAG,
        _ => RejectReason::INVALID_DATA_ENCODING,
    }
}

impl BACnetPropertyValue {
    pub fn decode(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        Self::decode_with_boundaries(
            data,
            offset,
            &[
                PropertyValueBoundary::End,
                PropertyValueBoundary::Context(0),
                PropertyValueBoundary::ContextToEnd(3),
            ],
        )
    }

    pub(crate) fn decode_in_list(
        data: &[u8],
        offset: usize,
        closing_tag: u8,
    ) -> Result<(Self, usize), Error> {
        Self::decode_in_list_detailed(data, offset, closing_tag).map_err(|error| error.error)
    }

    pub(crate) fn decode_in_list_detailed(
        data: &[u8],
        offset: usize,
        closing_tag: u8,
    ) -> Result<(Self, usize), PropertyValueDecodeError> {
        Self::decode_with_boundaries_detailed(
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
    ) -> Result<(Self, usize), Error> {
        Self::decode_with_boundaries_detailed(data, offset, boundaries).map_err(|error| error.error)
    }

    fn decode_with_boundaries_detailed(
        data: &[u8],
        offset: usize,
        boundaries: &[PropertyValueBoundary],
    ) -> Result<(Self, usize), PropertyValueDecodeError> {
        let start = offset;
        let (tag, content_start) = tags::decode_tag(data, offset).map_err(|error| {
            PropertyValueDecodeError::new(
                error,
                offset,
                PropertyValueDecodeStage::PropertyIdentifier,
                tag_failure_reason(data, offset),
                None,
                None,
                false,
            )
        })?;
        if !tag.is_context(0) {
            return Err(PropertyValueDecodeError::new(
                Error::decoding(
                    offset,
                    "BACnetPropertyValue property-id expected context tag 0",
                ),
                offset,
                PropertyValueDecodeStage::PropertyIdentifier,
                wrong_tag_reason(&tag),
                None,
                None,
                false,
            ));
        }
        let property_end = content_start
            .checked_add(tag.length as usize)
            .ok_or_else(|| {
                PropertyValueDecodeError::new(
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
            return Err(PropertyValueDecodeError::new(
                Error::decoding(content_start, "BACnetPropertyValue property-id truncated"),
                content_start,
                PropertyValueDecodeStage::PropertyIdentifier,
                RejectReason::INVALID_DATA_ENCODING,
                None,
                None,
                false,
            ));
        }
        let prop_id =
            primitives::decode_unsigned(&data[content_start..property_end]).map_err(|error| {
                PropertyValueDecodeError::new(
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
            PropertyValueDecodeError::new(
                Error::decoding(start, "BACnetPropertyValue property-id exceeds u32"),
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
                PropertyValueDecodeError::new(
                    error,
                    offset,
                    PropertyValueDecodeStage::ArrayIndex,
                    tag_failure_reason(data, offset),
                    Some(property_identifier),
                    None,
                    false,
                )
            })?;
            if tag.is_context(1) {
                let end = content_start
                    .checked_add(tag.length as usize)
                    .ok_or_else(|| {
                        PropertyValueDecodeError::new(
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
                    return Err(PropertyValueDecodeError::new(
                        Error::decoding(content_start, "BACnetPropertyValue array-index truncated"),
                        content_start,
                        PropertyValueDecodeStage::ArrayIndex,
                        RejectReason::INVALID_DATA_ENCODING,
                        Some(property_identifier),
                        None,
                        false,
                    ));
                }
                let value =
                    primitives::decode_unsigned(&data[content_start..end]).map_err(|error| {
                        PropertyValueDecodeError::new(
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
                    PropertyValueDecodeError::new(
                        Error::decoding(offset, "BACnetPropertyValue array-index exceeds u32"),
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
            PropertyValueDecodeError::new(
                error,
                offset,
                PropertyValueDecodeStage::Value,
                tag_failure_reason(data, offset),
                Some(property_identifier),
                array_index,
                true,
            )
        })?;
        if !tag.is_opening_tag(2) {
            return Err(PropertyValueDecodeError::new(
                Error::decoding(offset, "BACnetPropertyValue expected opening tag 2"),
                offset,
                PropertyValueDecodeStage::Value,
                wrong_tag_reason(&tag),
                Some(property_identifier),
                array_index,
                true,
            ));
        }
        let (value_bytes, offset) =
            extract_property_value(data, tag_end, 2, property_identifier, boundaries).map_err(
                |error| {
                    let reject_reason = value_failure_reason(&error);
                    let offset = error_offset(&error, tag_end);
                    PropertyValueDecodeError::new(
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
                PropertyValueDecodeError::new(
                    error,
                    offset,
                    PropertyValueDecodeStage::Priority,
                    tag_failure_reason(data, offset),
                    Some(property_identifier),
                    array_index,
                    true,
                )
            })?;
            if tag.is_context(3) {
                let end = new_pos.checked_add(tag.length as usize).ok_or_else(|| {
                    PropertyValueDecodeError::new(
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
                    return Err(PropertyValueDecodeError::new(
                        Error::decoding(new_pos, "BACnetPropertyValue truncated at priority"),
                        new_pos,
                        PropertyValueDecodeStage::Priority,
                        RejectReason::INVALID_DATA_ENCODING,
                        Some(property_identifier),
                        array_index,
                        true,
                    ));
                }
                let prio = primitives::decode_unsigned(&data[new_pos..end]).map_err(|error| {
                    PropertyValueDecodeError::new(
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
                    return Err(PropertyValueDecodeError::new(
                        Error::decoding(
                            new_pos,
                            format!("BACnetPropertyValue priority {prio} out of range 1-16"),
                        ),
                        new_pos,
                        PropertyValueDecodeStage::Priority,
                        RejectReason::PARAMETER_OUT_OF_RANGE,
                        Some(property_identifier),
                        array_index,
                        true,
                    ));
                }
                priority = Some(prio as u8);
                if boundaries
                    .iter()
                    .any(|boundary| matches_property_boundary(data, end, *boundary))
                {
                    return Ok((
                        Self {
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
            Self {
                property_identifier,
                property_array_index: array_index,
                value,
                priority,
            },
            offset,
        ))
    }
}

fn boundary_error(
    data: &[u8],
    offset: usize,
    property_identifier: PropertyIdentifier,
    property_array_index: Option<u32>,
) -> PropertyValueDecodeError {
    let (reject_reason, error) = if offset >= data.len() {
        (
            RejectReason::MISSING_REQUIRED_PARAMETER,
            Error::decoding(offset, "BACnetPropertyValue is missing its list boundary"),
        )
    } else {
        match tags::decode_tag(data, offset) {
            Ok((tag, _)) => (
                if tag.class == tags::TagClass::Application {
                    RejectReason::INVALID_PARAMETER_DATA_TYPE
                } else {
                    RejectReason::INVALID_TAG
                },
                Error::decoding(offset, "BACnetPropertyValue has an unexpected trailing tag"),
            ),
            Err(error) => (RejectReason::INVALID_DATA_ENCODING, error),
        }
    };
    PropertyValueDecodeError::new(
        error,
        offset,
        PropertyValueDecodeStage::Priority,
        reject_reason,
        Some(property_identifier),
        property_array_index,
        true,
    )
}
