//! Detailed confirmed-COV decoding and Reject classification.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::constructed::tagged::{
    decode_canonical_unsigned, decode_ctx_primitive, expect_end, misplaced_tag,
};
use bacnet_encoding::constructed::{
    decode_bacnet_property_value_in_list_detailed, PropertyValueDecodeFailure,
};
use bacnet_encoding::tags;
use bacnet_types::enums::RejectReason;
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bacnet_types::primitives::ObjectIdentifier;

use crate::common::MAX_DECODED_ITEMS;
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

/// A decoder's refusal with the reason the server gives the same fault in
/// any confirmed request (#1446; the table on
/// [`DecodingKind::reject_reason`](bacnet_types::error::DecodingKind::reject_reason)).
fn syntax(error: Error) -> COVNotificationDecodeError {
    let reject_reason = error
        .reject_reason()
        .unwrap_or(RejectReason::INVALID_DATA_ENCODING);
    failure(error, reject_reason)
}

/// The contents of the required primitive context tag `expected_tag` at
/// `offset`, with [`syntax`]'s reasons for a missing member, one cut short
/// or a tag that doesn't fit.
fn decode_required_context<'a>(
    data: &'a [u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> COVDecodeResult<(&'a [u8], usize)> {
    if offset >= data.len() {
        return Err(syntax(
            DecodeError::new(DecodingKind::Missing, offset, "is missing")
                .within(field)
                .into(),
        ));
    }
    decode_ctx_primitive(data, offset, expected_tag, field).map_err(syntax)
}

fn decode_required_u32(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> COVDecodeResult<(u32, usize)> {
    let (content, end) = decode_required_context(data, offset, expected_tag, field)?;
    let value = decode_canonical_unsigned(content, offset, field).map_err(syntax)?;
    let value = u32::try_from(value).map_err(|_| {
        syntax(
            DecodeError::new(DecodingKind::OutOfRange, offset, "exceeds u32")
                .within(field)
                .into(),
        )
    })?;
    Ok((value, end))
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
        let initiating_device_identifier = ObjectIdentifier::decode(content).map_err(syntax)?;
        offset = end;

        let (content, end) =
            decode_required_context(data, offset, 2, "COVNotification monitored-id")?;
        let monitored_object_identifier = ObjectIdentifier::decode(content).map_err(syntax)?;
        offset = end;

        let (time_remaining, end) =
            decode_required_u32(data, offset, 3, "COVNotification time-remaining")?;
        offset = end;

        if offset >= data.len() {
            return Err(syntax(Error::missing(
                offset,
                "COVNotification list-of-values is missing",
            )));
        }
        let (tag, tag_end) = tags::decode_tag(data, offset).map_err(syntax)?;
        if !tag.is_opening_tag(4) {
            return Err(syntax(misplaced_tag(
                data,
                &tag,
                Some(4),
                offset,
                "COVNotification expected opening tag 4",
            )));
        }
        offset = tag_end;

        let mut values = Vec::new();
        loop {
            if offset >= data.len() {
                return Err(syntax(Error::missing(
                    offset,
                    "COVNotification missing closing tag 4",
                )));
            }
            let (tag, tag_end) = tags::decode_tag(data, offset).map_err(syntax)?;
            if tag.is_closing_tag(4) {
                offset = tag_end;
                break;
            }
            if values.len() >= MAX_DECODED_ITEMS {
                return Err(syntax(Error::overflow(
                    offset,
                    "COVNotification values exceeds max",
                )));
            }
            let (pv, new_offset) = decode_bacnet_property_value_in_list_detailed(data, offset, 4)
                .map_err(|failed| {
                let reject_reason = match failed.kind {
                    PropertyValueDecodeFailure::Syntax(reason) => reason,
                    PropertyValueDecodeFailure::PriorityOutOfRange => {
                        RejectReason::PARAMETER_OUT_OF_RANGE
                    }
                };
                failure(failed.error, reject_reason)
            })?;
            values.push(pv);
            offset = new_offset;
        }
        if values.is_empty() {
            return Err(syntax(Error::out_of_range(
                offset,
                "COVNotification list-of-values must contain at least one value",
            )));
        }
        expect_end(data, offset, offset, "COVNotification").map_err(syntax)?;

        Ok(Self {
            subscriber_process_identifier,
            initiating_device_identifier,
            monitored_object_identifier,
            time_remaining,
            list_of_values: values,
        })
    }
}
