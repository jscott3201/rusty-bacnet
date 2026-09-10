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
    } else if msg.function == ScFunction::ConnectRequest && msg.payload[6..22] == [0; 16] {
        // Local peer-admission policy, not UUID version/variant validation.
        // Connect-Accept response policy is deliberately unchanged.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc_frame::{
        connect_test_support::valid_connect, decode_sc_message, encode_sc_message,
    };
    use bytes::BytesMut;

    #[test]
    fn zero_uuid_rejection_is_request_only_not_codec_policy() {
        for function in [6, 7] {
            let mut wire = valid_connect(function, [0x22; 6]);
            wire[10..26].fill(0);
            let message = decode_sc_message(&wire).unwrap();
            let mut encoded = BytesMut::new();
            encode_sc_message(&mut encoded, &message);
            assert_eq!(&encoded[..], wire, "generic syntax must preserve nil UUID");
            assert_eq!(
                connect_message_error(&message),
                if function == 6 {
                    Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
                } else {
                    None
                },
                "function {function}"
            );
        }
    }

    #[test]
    fn nonzero_uuid_bits_remain_opaque() {
        for function in [6, 7] {
            for position in 0..16 {
                for value in [1, 0x80, 0xff] {
                    let mut wire = valid_connect(function, [0x22; 6]);
                    wire[10..26].fill(0);
                    wire[10 + position] = value;
                    assert_eq!(
                        connect_message_error(&decode_sc_message(&wire).unwrap()),
                        None
                    );
                }
            }
            let mut wire = valid_connect(function, [0x22; 6]);
            wire[10..26].fill(0xff);
            assert_eq!(
                connect_message_error(&decode_sc_message(&wire).unwrap()),
                None
            );
        }
    }

    #[test]
    fn zero_uuid_precedence_keeps_accept_mu_error_and_request_range_error() {
        for function in [6, 7] {
            let mut wire = valid_connect(function, [0x22; 6]);
            wire[10..26].fill(0);
            wire[1] = 2;
            wire.insert(4, 0x5e);
            assert_eq!(
                connect_message_error(&decode_sc_message(&wire).unwrap()),
                Some(if function == 6 {
                    ErrorCode::PARAMETER_OUT_OF_RANGE
                } else {
                    ErrorCode::HEADER_NOT_UNDERSTOOD
                })
            );
        }
    }
}
