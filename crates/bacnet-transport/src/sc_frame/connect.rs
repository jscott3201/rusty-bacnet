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
    } else if msg.payload[6..22] == [0; 16] {
        // Local peer-admission policy, not UUID version/variant validation.
        // For Connect-Accept this is only a local diagnostic: AB.2 forbids
        // responding to response messages, so the handshake discards silently.
        Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
    } else if msg.payload[22..24] == [0; 2] || msg.payload[24..26] == [0; 2] {
        // Local zero-only receive-admission policy, not a universal positive
        // capacity floor or a relationship between Max-BVLC and Max-NPDU.
        // As with identity errors, Connect-Accept is discarded without reply.
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
    fn zero_limits_rejection_is_receive_admission_not_codec_policy() {
        for function in [6, 7] {
            for limits in [[0, 0, 0x10, 0], [0x20, 0, 0, 0], [0; 4]] {
                let mut wire = valid_connect(function, [0x22; 6]);
                wire[26..30].copy_from_slice(&limits);
                let message = decode_sc_message(&wire).unwrap();
                let mut encoded = BytesMut::new();
                encode_sc_message(&mut encoded, &message);
                assert_eq!(&encoded[..], wire, "generic syntax preserves zero limits");
                assert_eq!(
                    connect_message_error(&message),
                    Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
                );
                #[cfg(feature = "sc-tls")]
                if function == 7 {
                    assert_eq!(validate_connect_request(&message, &wire), Ok(()));
                }
            }
        }
    }

    #[test]
    fn positive_limits_remain_independent_without_a_serviceability_floor() {
        for function in [6, 7] {
            for (bvlc, npdu) in [
                (1u16, 1u16),
                (1, 65535),
                (65535, 1),
                (65535, 65535),
                (1200, 480),
                (300, 1476),
                (1476, 1476),
            ] {
                let mut wire = valid_connect(function, [0x22; 6]);
                wire[26..28].copy_from_slice(&bvlc.to_be_bytes());
                wire[28..30].copy_from_slice(&npdu.to_be_bytes());
                assert_eq!(
                    connect_message_error(&decode_sc_message(&wire).unwrap()),
                    None
                );
            }
        }
    }

    #[test]
    fn zero_uuid_rejection_is_receive_admission_not_codec_policy() {
        for function in [6, 7] {
            let mut wire = valid_connect(function, [0x22; 6]);
            wire[10..26].fill(0);
            let message = decode_sc_message(&wire).unwrap();
            let mut encoded = BytesMut::new();
            encode_sc_message(&mut encoded, &message);
            assert_eq!(&encoded[..], wire, "generic syntax must preserve nil UUID");
            assert_eq!(
                connect_message_error(&message),
                Some(ErrorCode::PARAMETER_OUT_OF_RANGE),
                "function {function}"
            );
        }
    }

    #[test]
    fn nonzero_uuid_bits_remain_opaque() {
        for function in [6, 7] {
            for position in 0..16 {
                for value in [1, 2, 4, 8, 0x10, 0x20, 0x40, 0x80, 0xff] {
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
    fn zero_uuid_precedes_mu_for_local_connect_diagnostics() {
        for function in [6, 7] {
            let mut wire = valid_connect(function, [0x22; 6]);
            wire[10..26].fill(0);
            wire[1] = 2;
            wire.insert(4, 0x5e);
            assert_eq!(
                connect_message_error(&decode_sc_message(&wire).unwrap()),
                Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
            );
        }
    }
}
