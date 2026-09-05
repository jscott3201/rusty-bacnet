//! Receive-only Connect admission after generic syntax decoding.

use super::{ScFunction, ScMessage};
use bacnet_types::enums::ErrorCode;

pub(crate) fn connect_message_error(msg: &ScMessage) -> Option<ErrorCode> {
    if !matches!(
        msg.function,
        ScFunction::ConnectRequest | ScFunction::ConnectAccept
    ) {
        return None;
    }
    // AB.2.10–11 omit VMACs and Data Options and define exactly 26 payload
    // octets. Error selection and multi-fault precedence interpret AB.3.1.5.
    if msg.originating_vmac.is_some()
        || msg.destination_vmac.is_some()
        || !msg.data_options.is_empty()
    {
        Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
    } else if msg.payload.is_empty() {
        Some(ErrorCode::PAYLOAD_EXPECTED)
    } else if msg.payload.len() < 26 {
        Some(ErrorCode::MESSAGE_INCOMPLETE)
    } else if msg.payload.len() > 26 {
        Some(ErrorCode::INCONSISTENT_PARAMETERS)
    } else if msg.payload[..6] == [0; 6] || msg.payload[..6] == [0xff; 6] {
        Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
    } else if msg.dest_options.iter().any(|option| option.must_understand) {
        // Known-option shape/placement validation remains separate work.
        Some(ErrorCode::HEADER_NOT_UNDERSTOOD)
    } else {
        None
    }
}

/// Accepting-hub rejection. Response addressing uses the envelope source,
/// never the proposed identity inside the Connect payload.
#[cfg(feature = "sc-tls")]
pub(crate) fn validate_connect_request(
    msg: &ScMessage,
    wire: &[u8],
) -> Result<(), Option<bytes::Bytes>> {
    use super::{
        encode_sc_message, first_must_understand_destination_option_marker, BROADCAST_VMAC,
        UNKNOWN_VMAC,
    };
    use bacnet_types::enums::ErrorClass;
    use bytes::{Bytes, BytesMut};

    if msg.function != ScFunction::ConnectRequest {
        return Ok(());
    }
    let Some(code) = connect_message_error(msg) else {
        return Ok(());
    };
    // AB.2 suppresses broadcast responses. Reserved envelope-source
    // suppression is conservative local hardening, as for control messages.
    if msg.destination_vmac == Some(BROADCAST_VMAC)
        || matches!(msg.originating_vmac, Some(UNKNOWN_VMAC | BROADCAST_VMAC))
    {
        return Err(None);
    }
    let marker = if code == ErrorCode::HEADER_NOT_UNDERSTOOD {
        first_must_understand_destination_option_marker(wire).ok_or(None)?
    } else {
        0
    };
    let class = ErrorClass::COMMUNICATION.to_raw().to_be_bytes();
    let code = code.to_raw().to_be_bytes();
    let nak = ScMessage {
        function: ScFunction::Result,
        message_id: msg.message_id,
        originating_vmac: None,
        destination_vmac: msg.originating_vmac,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![6, 1, marker, class[0], class[1], code[0], code[1]]),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &nak);
    Err(Some(buf.freeze()))
}
