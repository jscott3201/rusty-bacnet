//! Detailed confirmed-COV decoding and Reject classification.

use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::RejectReason;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::common::{BACnetPropertyValue, MAX_DECODED_ITEMS};
use crate::cov::COVNotificationRequest;

/// Structured failure returned when decoding a confirmed COV notification.
///
/// Confirmed-service responders need the Clause 18.9 Reject reason in
/// addition to the ordinary decoder error retained by
/// [`COVNotificationRequest::decode`].
#[derive(Debug)]
pub struct COVNotificationDecodeError {
    error: Error,
    reject_reason: RejectReason,
}

impl COVNotificationDecodeError {
    fn new(error: Error, reject_reason: RejectReason) -> Self {
        Self {
            error,
            reject_reason,
        }
    }

    /// Reject reason appropriate for this confirmed-service syntax failure.
    pub fn reject_reason(&self) -> RejectReason {
        self.reject_reason
    }

    /// Recover the ordinary decoder error used by the compatibility API.
    pub fn into_error(self) -> Error {
        self.error
    }
}

type COVDecodeResult<T> = Result<T, COVNotificationDecodeError>;

fn failure(error: Error, reject_reason: RejectReason) -> COVNotificationDecodeError {
    COVNotificationDecodeError::new(error, reject_reason)
}

fn decode_required_context<'a>(
    data: &'a [u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> COVDecodeResult<(&'a [u8], usize)> {
    if offset >= data.len() {
        return Err(failure(
            Error::decoding(offset, format!("{field} is missing")),
            RejectReason::MISSING_REQUIRED_PARAMETER,
        ));
    }
    let (tag, pos) = tags::decode_tag(data, offset)
        .map_err(|error| failure(error, RejectReason::INVALID_TAG))?;
    if !tag.is_context(expected_tag) {
        return Err(failure(
            Error::decoding(
                offset,
                format!("{field} expected context tag {expected_tag}"),
            ),
            RejectReason::INVALID_TAG,
        ));
    }
    let end = pos.checked_add(tag.length as usize).ok_or_else(|| {
        failure(
            Error::decoding(pos, format!("{field} length overflow")),
            RejectReason::INVALID_DATA_ENCODING,
        )
    })?;
    if end > data.len() {
        return Err(failure(
            Error::decoding(pos, format!("{field} has invalid data encoding")),
            RejectReason::INVALID_DATA_ENCODING,
        ));
    }
    Ok((&data[pos..end], end))
}

fn decode_required_u32(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> COVDecodeResult<(u32, usize)> {
    let (content, end) = decode_required_context(data, offset, expected_tag, field)?;
    if content.len() > 1 && content.first() == Some(&0) {
        return Err(failure(
            Error::decoding(offset, format!("{field} is not minimally encoded")),
            RejectReason::INVALID_DATA_ENCODING,
        ));
    }
    let value = primitives::decode_unsigned(content)
        .map_err(|error| failure(error, RejectReason::INVALID_DATA_ENCODING))?;
    let value = u32::try_from(value).map_err(|_| {
        failure(
            Error::decoding(offset, format!("{field} exceeds u32")),
            RejectReason::PARAMETER_OUT_OF_RANGE,
        )
    })?;
    Ok((value, end))
}

fn property_value_reject_reason(error: &Error) -> RejectReason {
    match error {
        Error::InvalidTag(_) => RejectReason::INVALID_TAG,
        Error::OutOfRange(_) => RejectReason::PARAMETER_OUT_OF_RANGE,
        Error::BufferTooShort { .. } => RejectReason::INVALID_DATA_ENCODING,
        Error::Decoding { message, .. }
            if message.contains("expected context tag")
                || message.contains("expected opening tag")
                || message.contains("expected closing tag") =>
        {
            RejectReason::INVALID_TAG
        }
        Error::Decoding { message, .. } if message.contains("out of range") => {
            RejectReason::PARAMETER_OUT_OF_RANGE
        }
        Error::Decoding { .. } => RejectReason::INVALID_DATA_ENCODING,
        _ => RejectReason::OTHER,
    }
}

impl COVNotificationRequest {
    /// Decode a confirmed COV notification and retain its Clause 18.9 Reject
    /// classification if the service parameters are malformed.
    pub fn decode_detailed(data: &[u8]) -> COVDecodeResult<Self> {
        let mut offset = 0;

        let (subscriber_process_identifier, end) =
            decode_required_u32(data, offset, 0, "COVNotification process-id")?;
        offset = end;

        let (content, end) = decode_required_context(data, offset, 1, "COVNotification device-id")?;
        let initiating_device_identifier = ObjectIdentifier::decode(content)
            .map_err(|error| failure(error, RejectReason::INVALID_DATA_ENCODING))?;
        offset = end;

        let (content, end) =
            decode_required_context(data, offset, 2, "COVNotification monitored-id")?;
        let monitored_object_identifier = ObjectIdentifier::decode(content)
            .map_err(|error| failure(error, RejectReason::INVALID_DATA_ENCODING))?;
        offset = end;

        let (time_remaining, end) =
            decode_required_u32(data, offset, 3, "COVNotification time-remaining")?;
        offset = end;

        if offset >= data.len() {
            return Err(failure(
                Error::decoding(offset, "COVNotification list-of-values is missing"),
                RejectReason::MISSING_REQUIRED_PARAMETER,
            ));
        }
        let (tag, tag_end) = tags::decode_tag(data, offset)
            .map_err(|error| failure(error, RejectReason::INVALID_TAG))?;
        if !tag.is_opening_tag(4) {
            return Err(failure(
                Error::decoding(offset, "COVNotification expected opening tag 4"),
                RejectReason::INVALID_TAG,
            ));
        }
        offset = tag_end;

        let mut values = Vec::new();
        loop {
            if offset >= data.len() {
                return Err(failure(
                    Error::decoding(offset, "COVNotification missing closing tag 4"),
                    RejectReason::INVALID_TAG,
                ));
            }
            let (tag, tag_end) = tags::decode_tag(data, offset)
                .map_err(|error| failure(error, RejectReason::INVALID_TAG))?;
            if tag.is_closing_tag(4) {
                offset = tag_end;
                break;
            }
            if values.len() >= MAX_DECODED_ITEMS {
                return Err(failure(
                    Error::decoding(offset, "COVNotification values exceeds max"),
                    RejectReason::BUFFER_OVERFLOW,
                ));
            }
            let (pv, new_offset) =
                BACnetPropertyValue::decode_in_list(data, offset, 4).map_err(|error| {
                    let reject_reason = property_value_reject_reason(&error);
                    failure(error, reject_reason)
                })?;
            values.push(pv);
            offset = new_offset;
        }
        if values.is_empty() {
            return Err(failure(
                Error::decoding(
                    offset,
                    "COVNotification list-of-values must contain at least one value",
                ),
                RejectReason::PARAMETER_OUT_OF_RANGE,
            ));
        }
        if offset != data.len() {
            let reject_reason = match tags::decode_tag(data, offset) {
                Ok((tag, _)) if !tag.is_opening && !tag.is_closing => {
                    RejectReason::TOO_MANY_ARGUMENTS
                }
                _ => RejectReason::INVALID_TAG,
            };
            return Err(failure(
                Error::decoding(offset, "COVNotification has trailing data"),
                reject_reason,
            ));
        }

        Ok(Self {
            subscriber_process_identifier,
            initiating_device_identifier,
            monitored_object_identifier,
            time_remaining,
            list_of_values: values,
        })
    }
}
