//! Structural recognition of the Clause 21 error productions that replace the
//! plain class/code pair. Each opens with the error class and code inside
//! `[0]`, then adds its own members:
//!
//! - WritePropertyMultiple-Error (service 16): a `[1]` frame around the
//!   BACnetObjectPropertyReference of the first failed write attempt;
//! - ChangeList-Error (AddListElement 8, RemoveListElement 9) and
//!   CreateObject-Error (10): a `[1]` Unsigned first failed element number;
//! - ConfirmedPrivateTransfer-Error (18): `[1]` vendor identifier, `[2]`
//!   service number and an optional `[3]` frame of error parameters;
//! - VTClose-Error (22): an optional `[1]` frame of application Unsigned8
//!   session identifiers.
//!
//! SubscribeCOVPropertyMultiple-Error (30) is a CHOICE instead: the `[0]`
//! error alone, or a `[1]` frame holding the failed subscription's `[0]`
//! object identifier, `[1]` property reference and `[2]` error.
//!
//! The members are read with the constructed codecs' shared tagged-field
//! helpers, so a refusal has the kind and wording it has there.

use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::{DecodeError, DecodingKind, Error};

use crate::constructed::tagged::{
    decode_app_enumerated, decode_app_unsigned, decode_ctx_constructed, decode_ctx_object_id,
    decode_ctx_unsigned, expect_end,
};
use crate::constructed::{decode_object_property_reference, decode_property_reference};
use crate::tags;

/// Decodes the members after `[0]` from an offset, returning the offset past
/// them.
type Members = fn(&[u8], usize, &'static str) -> Result<usize, Error>;

/// The class and code of `data` when it is the complete formal error body of
/// `service`; `None` when the service has no formal body or `data` does not
/// open with an opening tag the production starts with. A body that opens
/// with one but is malformed is an error: [`Error::BufferTooShort`] when a
/// member's contents run past the end of `data`, [`Error::Decoding`]
/// otherwise.
pub(super) fn decode_formal_body(
    service: ConfirmedServiceChoice,
    data: &[u8],
) -> Result<Option<(ErrorClass, ErrorCode)>, Error> {
    let (what, members): (&'static str, Members) = match service {
        s if s == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE => {
            ("formal WPM Error", first_failed_write_attempt)
        }
        s if s == ConfirmedServiceChoice::ADD_LIST_ELEMENT
            || s == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT =>
        {
            ("ChangeList-Error", first_failed_element_number)
        }
        s if s == ConfirmedServiceChoice::CREATE_OBJECT => {
            ("CreateObject-Error", first_failed_element_number)
        }
        s if s == ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER => {
            ("ConfirmedPrivateTransfer-Error", private_transfer_members)
        }
        s if s == ConfirmedServiceChoice::VT_CLOSE => ("VTClose-Error", vt_session_identifiers),
        s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE => {
            return subscribe_cov_property_multiple(data);
        }
        _ => return Ok(None),
    };
    if !opens_with(data, 0) {
        return Ok(None);
    }
    let (pair, offset) = error_type(data, 0, 0, what)?;
    let end = members(data, offset, what)?;
    expect_end(data, end, end, what)?;
    Ok(Some(pair))
}

/// Whether `data` starts with opening tag `number`.
fn opens_with(data: &[u8], number: u8) -> bool {
    tags::decode_tag(data, 0).is_ok_and(|(tag, _)| tag.is_opening_tag(number))
}

/// The CHOICE: `[0]` Error alone, or `[1]` first-failed-subscription, whose
/// `[2]` Error supplies the class and code.
fn subscribe_cov_property_multiple(data: &[u8]) -> Result<Option<(ErrorClass, ErrorCode)>, Error> {
    const WHAT: &str = "SubscribeCOVPropertyMultiple-Error";
    if opens_with(data, 0) {
        let (pair, end) = error_type(data, 0, 0, WHAT)?;
        expect_end(data, end, end, WHAT)?;
        return Ok(Some(pair));
    }
    if !opens_with(data, 1) {
        return Ok(None);
    }
    let (subscription, end) = decode_ctx_constructed(data, 0, 1, WHAT)?;
    expect_end(data, end, end, WHAT)?;

    // Offsets from here on are into the `[1]` body; its frame opens at 0.
    let (_, offset) = decode_ctx_object_id(subscription, 0, 0, WHAT)?;
    let (reference, after_reference) = decode_ctx_constructed(subscription, offset, 1, WHAT)?;
    let (_, reference_end) = decode_property_reference(reference, 0)?;
    expect_end(reference, reference_end, offset, WHAT)?;
    let (pair, end) = error_type(subscription, after_reference, 2, WHAT)?;
    expect_end(subscription, end, 0, WHAT)?;
    Ok(Some(pair))
}

/// WPM's `[1]` frame around the first failed write attempt.
fn first_failed_write_attempt(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<usize, Error> {
    let (reference, end) = decode_ctx_constructed(data, offset, 1, what)?;
    decode_object_property_reference(reference)?;
    Ok(end)
}

/// ChangeList's and CreateObject's `[1]` Unsigned first failed element number.
fn first_failed_element_number(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<usize, Error> {
    let (_, end) = decode_ctx_unsigned::<u32>(data, offset, 1, what)?;
    Ok(end)
}

/// ConfirmedPrivateTransfer's `[1]` vendor identifier and `[2]` service
/// number, then its optional `[3]` error parameters.
fn private_transfer_members(
    data: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<usize, Error> {
    let (_, offset) = decode_ctx_unsigned::<u32>(data, offset, 1, what)?;
    let (_, offset) = decode_ctx_unsigned::<u32>(data, offset, 2, what)?;
    if offset == data.len() {
        return Ok(offset);
    }
    let (_, end) = decode_ctx_constructed(data, offset, 3, what)?;
    Ok(end)
}

/// VTClose's optional `[1]` frame of application Unsigned8 identifiers.
fn vt_session_identifiers(data: &[u8], offset: usize, what: &'static str) -> Result<usize, Error> {
    if offset == data.len() {
        return Ok(offset);
    }
    let (sessions, end) = decode_ctx_constructed(data, offset, 1, what)?;
    let mut position = 0;
    while position < sessions.len() {
        let (session, next) = decode_app_unsigned::<u64>(sessions, position, what)?;
        if u8::try_from(session).is_err() {
            return Err(DecodeError::new(
                DecodingKind::OutOfRange,
                position,
                "session identifier {} exceeds u8",
            )
            .within(what)
            .with_value(session)
            .into());
        }
        position = next;
    }
    Ok(end)
}

/// The class and code inside the constructed `[number]` at `offset`, with the
/// offset past its closing tag.
fn error_type(
    data: &[u8],
    offset: usize,
    number: u8,
    what: &'static str,
) -> Result<((ErrorClass, ErrorCode), usize), Error> {
    let (error_body, end) = decode_ctx_constructed(data, offset, number, what)?;
    let (class, next) = decode_app_enumerated(error_body, 0, what)?;
    let (code, body_end) = decode_app_enumerated(error_body, next, what)?;
    expect_end(error_body, body_end, offset, what)?;
    Ok((
        (ErrorClass::from_raw(class), ErrorCode::from_raw(code)),
        end,
    ))
}
