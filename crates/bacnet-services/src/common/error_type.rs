//! The Error member of the Clause 21 error productions that replace the plain
//! class/code pair (WritePropertyMultiple-Error, ChangeList-Error,
//! CreateObject-Error, SubscribeCOVPropertyMultiple-Error,
//! ConfirmedPrivateTransfer-Error, VTClose-Error): an opening and closing
//! context tag around the application-tagged class and code, `[0]` in every
//! production except inside SubscribeCOVPropertyMultiple's failed subscription.

use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::tagged::{
    decode_app_enumerated, decode_ctx_constructed, decode_ctx_unsigned, expect_end,
};
use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bytes::BytesMut;

/// Encode `[0]` around the error class and code.
pub(crate) fn encode_error_type(buf: &mut BytesMut, class: ErrorClass, code: ErrorCode) {
    encode_error_in(buf, 0, class, code);
}

/// Encode `[tag_number]` around the error class and code.
pub(crate) fn encode_error_in(
    buf: &mut BytesMut,
    tag_number: u8,
    class: ErrorClass,
    code: ErrorCode,
) {
    tags::encode_opening_tag(buf, tag_number);
    primitives::encode_app_enumerated(buf, class.to_raw() as u32);
    primitives::encode_app_enumerated(buf, code.to_raw() as u32);
    tags::encode_closing_tag(buf, tag_number);
}

/// Decode the `[0]` error type at the start of `data`, returning the class and
/// code with the offset just past its closing tag. `what` names the
/// production in errors.
pub(crate) fn decode_error_type(
    data: &[u8],
    what: &str,
) -> Result<((ErrorClass, ErrorCode), usize), Error> {
    decode_error_in(data, 0, 0, what)
}

/// Decode the constructed `[tag_number]` error at `offset`, returning the
/// class and code with the offset just past its closing tag.
pub(crate) fn decode_error_in(
    data: &[u8],
    offset: usize,
    tag_number: u8,
    what: &str,
) -> Result<((ErrorClass, ErrorCode), usize), Error> {
    let (body, end) = decode_ctx_constructed(data, offset, tag_number, what)?;
    let (class, position) = decode_app_enumerated::<u16>(body, 0, what)?;
    let (code, body_end) = decode_app_enumerated::<u16>(body, position, what)?;
    expect_end(body, body_end, offset, what)?;
    Ok((
        (ErrorClass::from_raw(class), ErrorCode::from_raw(code)),
        end,
    ))
}

/// Encode `[0]` Error then `[1]` Unsigned first-failed-element-number, the
/// shape ChangeList-Error and CreateObject-Error share.
pub(crate) fn encode_element_error(
    buf: &mut BytesMut,
    class: ErrorClass,
    code: ErrorCode,
    first_failed_element_number: u32,
) {
    encode_error_type(buf, class, code);
    primitives::encode_ctx_unsigned(buf, 1, u64::from(first_failed_element_number));
}

/// Decode one complete `[0]` Error, `[1]` Unsigned body with no trailing
/// content. Errors call the body `what` and its `[1]` member `number`, which
/// is written out whole (`ChangeList-Error first-failed-element-number`), so
/// a body that decodes builds no label (#1645).
pub(crate) fn decode_element_error(
    data: &[u8],
    what: &str,
    number: &str,
) -> Result<(ErrorClass, ErrorCode, u32), Error> {
    let ((class, code), offset) = decode_error_type(data, what)?;
    let (number, end) = decode_ctx_unsigned::<u32>(data, offset, 1, number)?;
    expect_end(data, end, end, what)?;
    Ok((class, code, number))
}

/// The Error PDU answering `service_choice` with `encode`'s body. The APDU
/// encoder sends it in place of the plain pair only when it is the formal
/// body of that service; otherwise it follows a plain class and code.
pub(crate) fn error_pdu(
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    (error_class, error_code): (ErrorClass, ErrorCode),
    encode: impl FnOnce(&mut BytesMut),
) -> ErrorPdu {
    let mut body = BytesMut::new();
    encode(&mut body);
    ErrorPdu {
        invoke_id,
        service_choice,
        error_class,
        error_code,
        error_data: body.freeze(),
    }
}

/// Decode `pdu`'s body with `decode` when it answers one of `services`, and
/// refuse a body whose class and code disagree with the PDU's. A plain
/// class/code error, which older devices send, fails to decode.
pub(crate) fn decode_error_pdu<T>(
    pdu: &ErrorPdu,
    services: &[ConfirmedServiceChoice],
    what: &str,
    decode: impl FnOnce(&[u8]) -> Result<T, Error>,
    pair: impl FnOnce(&T) -> (ErrorClass, ErrorCode),
) -> Result<T, Error> {
    if !services.contains(&pdu.service_choice) {
        return Err(Error::decoding(
            0,
            format!("ErrorPdu for {:?} is not a {what}", pdu.service_choice),
        ));
    }
    let decoded = decode(&pdu.error_data)?;
    if pair(&decoded) != (pdu.error_class, pdu.error_code) {
        return Err(Error::decoding(
            0,
            format!("{what} body disagrees with ErrorPdu class/code"),
        ));
    }
    Ok(decoded)
}
