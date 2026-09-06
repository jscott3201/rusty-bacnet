use bacnet_encoding::tags;
use bacnet_types::enums::{PropertyIdentifier, RejectReason};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;

use crate::common::decode_context_bool;

pub(super) fn reject(reason: RejectReason, _message: &'static str) -> Error {
    Error::Reject {
        reason: reason.to_raw(),
    }
}

pub(super) fn decode_required_bool(
    data: &[u8],
    offset: usize,
    context_tag: u8,
    field: &'static str,
) -> Result<(bool, usize), Error> {
    if offset >= data.len() {
        return Err(reject(
            RejectReason::MISSING_REQUIRED_PARAMETER,
            "required Boolean is missing",
        ));
    }
    let (tag, _) = tags::decode_tag(data, offset).map_err(|_| {
        reject(
            RejectReason::INVALID_DATA_ENCODING,
            "required Boolean tag is malformed",
        )
    })?;
    if !tag.is_context(context_tag) {
        return Err(reject(
            RejectReason::MISSING_REQUIRED_PARAMETER,
            "required Boolean is missing",
        ));
    }
    decode_context_bool(data, offset, context_tag, field).map_err(|_| {
        reject(
            RejectReason::INVALID_DATA_ENCODING,
            "required Boolean value is malformed",
        )
    })
}

fn actual_date_is_valid(date: &Date) -> bool {
    if date.year == Date::UNSPECIFIED
        || !(1..=12).contains(&date.month)
        || !(1..=7).contains(&date.day_of_week)
    {
        return false;
    }

    let year = 1900 + u16::from(date.year);
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days_in_month = match date.month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days_in_month).contains(&date.day)
}

pub(super) fn actual_time_is_valid(time: &Time) -> bool {
    time.hour <= 23 && time.minute <= 59 && time.second <= 59 && time.hundredths <= 99
}

pub(super) fn validate_actual_date_time(date: &Date, time: &Time) -> Result<(), Error> {
    if !actual_date_is_valid(date) || !actual_time_is_valid(time) {
        return Err(Error::Encoding(
            "COVNotificationMultiple requires a specific actual DateTime".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_actual_time(time: &Time) -> Result<(), Error> {
    if !actual_time_is_valid(time) {
        return Err(Error::Encoding(
            "COVNotificationMultiple time-of-change must be a specific actual Time".into(),
        ));
    }
    Ok(())
}

fn property_identifier_is_forbidden(property_identifier: PropertyIdentifier) -> bool {
    matches!(
        property_identifier,
        PropertyIdentifier::ALL | PropertyIdentifier::OPTIONAL | PropertyIdentifier::REQUIRED
    )
}

pub(super) fn validate_property_identifier(
    property_identifier: PropertyIdentifier,
    field: &str,
) -> Result<(), Error> {
    if property_identifier_is_forbidden(property_identifier) {
        return Err(Error::Encoding(format!(
            "{field} cannot be ALL, OPTIONAL, or REQUIRED"
        )));
    }
    Ok(())
}

pub(super) fn validate_decoded_property_identifier(
    property_identifier: PropertyIdentifier,
    _field: &str,
) -> Result<(), Error> {
    if property_identifier_is_forbidden(property_identifier) {
        return Err(reject(
            RejectReason::PARAMETER_OUT_OF_RANGE,
            "property identifier is forbidden in COV Multiple",
        ));
    }
    Ok(())
}

pub(super) fn validate_raw_property_value(value: &[u8]) -> Result<(), Error> {
    if value.is_empty() {
        return Err(Error::Encoding(
            "COVNotificationMultiple property value must not be empty".into(),
        ));
    }
    let mut framed = BytesMut::new();
    tags::encode_opening_tag(&mut framed, 2);
    let content_start = framed.len();
    framed.extend_from_slice(value);
    tags::encode_closing_tag(&mut framed, 2);
    let (_, end) = tags::extract_context_value(&framed, content_start, 2)
        .map_err(|error| Error::Encoding(format!("invalid property value: {error}")))?;
    if end != framed.len() {
        return Err(Error::Encoding(
            "COVNotificationMultiple property value has unbalanced tags".into(),
        ));
    }
    Ok(())
}

pub(super) fn decode_date_time(
    data: &[u8],
    offset: usize,
    context_tag: u8,
) -> Result<((Date, Time), usize), Error> {
    let (opening, mut pos) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(context_tag) {
        return Err(reject(
            RejectReason::INVALID_TAG,
            "invalid DateTime opening tag",
        ));
    }

    let (date_tag, content_start) = tags::decode_tag(data, pos)?;
    if date_tag.class != tags::TagClass::Application
        || date_tag.number != tags::app_tag::DATE
        || date_tag.length != 4
    {
        return Err(reject(
            RejectReason::INVALID_DATA_ENCODING,
            "invalid DateTime Date",
        ));
    }
    let content_end = content_start.checked_add(4).ok_or_else(|| {
        reject(
            RejectReason::INVALID_DATA_ENCODING,
            "DateTime Date length overflow",
        )
    })?;
    let date = Date::decode(data.get(content_start..content_end).ok_or_else(|| {
        reject(
            RejectReason::INVALID_DATA_ENCODING,
            "truncated DateTime Date",
        )
    })?)
    .map_err(|_| reject(RejectReason::INVALID_DATA_ENCODING, "invalid DateTime Date"))?;
    pos = content_end;

    let (time_tag, content_start) = tags::decode_tag(data, pos)?;
    if time_tag.class != tags::TagClass::Application
        || time_tag.number != tags::app_tag::TIME
        || time_tag.length != 4
    {
        return Err(reject(
            RejectReason::INVALID_DATA_ENCODING,
            "invalid DateTime Time",
        ));
    }
    let content_end = content_start.checked_add(4).ok_or_else(|| {
        reject(
            RejectReason::INVALID_DATA_ENCODING,
            "DateTime Time length overflow",
        )
    })?;
    let time = Time::decode(data.get(content_start..content_end).ok_or_else(|| {
        reject(
            RejectReason::INVALID_DATA_ENCODING,
            "truncated DateTime Time",
        )
    })?)
    .map_err(|_| reject(RejectReason::INVALID_DATA_ENCODING, "invalid DateTime Time"))?;
    if !actual_date_is_valid(&date) || !actual_time_is_valid(&time) {
        return Err(reject(
            RejectReason::INVALID_DATA_ENCODING,
            "DateTime must contain a specific actual Date and Time",
        ));
    }
    pos = content_end;

    let (closing, end) = tags::decode_tag(data, pos)?;
    if !closing.is_closing_tag(context_tag) {
        return Err(reject(
            RejectReason::INVALID_TAG,
            "invalid DateTime closing tag",
        ));
    }
    Ok(((date, time), end))
}
