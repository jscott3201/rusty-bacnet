//! Receive-only heartbeat admission after generic syntax decoding.

use super::{
    encode_sc_message, first_must_understand_destination_option_marker, ScFunction, ScMessage,
    BROADCAST_VMAC, UNKNOWN_VMAC,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::{Bytes, BytesMut};

/// An error always discards the envelope; its optional NAK is connection-local.
/// Supply a successfully decoded message and its original wire bytes.
pub(crate) fn validate_heartbeat(msg: &ScMessage, wire: &[u8]) -> Result<(), Option<Bytes>> {
    if !matches!(
        msg.function,
        ScFunction::HeartbeatRequest | ScFunction::HeartbeatAck
    ) {
        return Ok(());
    }
    // AB.2.14–15 omit VMACs, Data Options, and payload. The error mapping
    // and multi-fault order are local interpretations of AB.3.1.5.
    let (marker, code) = if msg.originating_vmac.is_some()
        || msg.destination_vmac.is_some()
        || !msg.data_options.is_empty()
    {
        (0, ErrorCode::PARAMETER_OUT_OF_RANGE)
    } else if !msg.payload.is_empty() {
        (0, ErrorCode::INCONSISTENT_PARAMETERS)
    } else if msg.dest_options.iter().any(|option| option.must_understand) {
        // No Destination Options are understood here. Known-option shape and
        // placement validation across functions remains separate work.
        // The decoded option loses More Options and empty Header Data bits.
        // Fail closed if their original marker cannot be recovered.
        let marker = first_must_understand_destination_option_marker(wire).ok_or(None)?;
        (marker, ErrorCode::HEADER_NOT_UNDERSTOOD)
    } else {
        return Ok(());
    };

    // Responses and broadcasts must not generate responses. Reserved sources
    // are suppressed as local hardening: neither provides a unicast reply target.
    if msg.function == ScFunction::HeartbeatAck
        || msg.destination_vmac == Some(BROADCAST_VMAC)
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
            0x0A, 0x01, marker, class[0], class[1], code[0], code[1],
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
    fn heartbeat_missing_raw_marker_fails_closed_without_fabricated_nak() {
        let wire = [0x0A, 2, 0x22, 0x33, 0xFE, 0, 0, 0x1E];
        let msg = crate::sc_frame::decode_sc_message(&wire).unwrap();
        // Defensive fallback for a caller that cannot supply the original
        // marker, which cannot arise from the receive loops' paired inputs.
        assert_eq!(validate_heartbeat(&msg, &[]), Err(None));
    }
}
