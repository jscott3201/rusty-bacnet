//! Receive-only heartbeat and Disconnect admission after generic syntax decoding.

use super::{
    encode_sc_message, first_must_understand_destination_option_marker, ScFunction, ScMessage,
    BROADCAST_VMAC, UNKNOWN_VMAC,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::{Bytes, BytesMut};

#[derive(Clone, Copy)]
pub(crate) enum ControlRecipient {
    HubConnector,
    #[cfg(feature = "sc-tls")]
    AcceptingHub,
}

/// Shared semantic admission for wire receivers and direct connection callers.
pub(crate) fn control_envelope_error(msg: &ScMessage) -> Option<ErrorCode> {
    if !matches!(
        msg.function,
        ScFunction::HeartbeatRequest
            | ScFunction::HeartbeatAck
            | ScFunction::DisconnectRequest
            | ScFunction::DisconnectAck
    ) {
        return None;
    }
    // AB.2.12–15 omit VMACs, Data Options, and payload. The error mapping
    // and multi-fault order are local interpretations of AB.3.1.5.
    if msg.originating_vmac.is_some()
        || msg.destination_vmac.is_some()
        || !msg.data_options.is_empty()
    {
        Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
    } else if !msg.payload.is_empty() {
        Some(ErrorCode::INCONSISTENT_PARAMETERS)
    } else if msg.dest_options.iter().any(|option| option.must_understand) {
        // No Destination Options are understood here. Known-option shape and
        // placement validation across functions remains separate work.
        Some(ErrorCode::HEADER_NOT_UNDERSTOOD)
    } else {
        None
    }
}

/// An error always discards the envelope; its optional NAK is connection-local.
/// Supply a successfully decoded message and its original wire bytes.
pub(crate) fn validate_control(
    msg: &ScMessage,
    wire: &[u8],
    recipient: ControlRecipient,
) -> Result<(), Option<Bytes>> {
    let Some(code) = control_envelope_error(msg) else {
        return Ok(());
    };
    // AB.5.4 drops addressed local-BVLL messages at the hub connector. With
    // AB.2's broadcast rule, every explicit destination is silent there.
    if matches!(recipient, ControlRecipient::HubConnector) && msg.destination_vmac.is_some() {
        return Err(None);
    }
    let marker = if code == ErrorCode::HEADER_NOT_UNDERSTOOD {
        // The decoded option loses More Options and empty Header Data bits.
        // Fail closed if their original marker cannot be recovered.
        first_must_understand_destination_option_marker(wire).ok_or(None)?
    } else {
        0
    };

    // Responses and broadcasts must not generate responses. Reserved sources
    // are suppressed as local hardening: neither provides a unicast reply target.
    if matches!(
        msg.function,
        ScFunction::HeartbeatAck | ScFunction::DisconnectAck
    ) || msg.destination_vmac == Some(BROADCAST_VMAC)
        || matches!(msg.originating_vmac, Some(UNKNOWN_VMAC | BROADCAST_VMAC))
    {
        return Err(None);
    }
    let class = ErrorClass::COMMUNICATION.to_raw().to_be_bytes();
    let code = code.to_raw().to_be_bytes();
    let nak = ScMessage {
        function: ScFunction::Result,
        message_id: msg.message_id,
        originating_vmac: None,
        destination_vmac: msg.originating_vmac,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![
            msg.function.to_raw(),
            0x01,
            marker,
            class[0],
            class[1],
            code[0],
            code[1],
        ]),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &nak);
    Err(Some(buf.freeze()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_missing_raw_marker_fails_closed_without_fabricated_nak() {
        // Defensive fallback for a caller that cannot supply the original
        // marker, which cannot arise from the receive loops' paired inputs.
        for function in [0x08, 0x0A] {
            let wire = [function, 2, 0x22, 0x33, 0xFE, 0, 0, 0x1E];
            let msg = crate::sc_frame::decode_sc_message(&wire).unwrap();
            assert_eq!(
                validate_control(&msg, &[], ControlRecipient::HubConnector),
                Err(None)
            );
        }
    }
}
