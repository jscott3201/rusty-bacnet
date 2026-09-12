//! Hub-only Proprietary-Message local rejection, not vendor dispatch.
//! Registered unicast AND broadcast transit stays opaque via the shared
//! relay mechanics (AB.5.3.2/AB.5.3.3 forwarding).
use super::*;
use crate::sc_frame::{
    encode_sc_message, first_must_understand_destination_option_marker, proprietary_message_error,
    BROADCAST_VMAC, UNKNOWN_VMAC,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::{Bytes, BytesMut};

pub(super) fn local_nak(msg: &ScMessage, wire: &[u8]) -> Option<ScMessage> {
    if msg.function != ScFunction::ProprietaryMessage {
        return None;
    }
    // Proprietary is not a response, but AB.2 still forbids answering
    // broadcasts. Reserved origins stay silent as hub hardening, matching
    // the Unknown, Resolution, and Advertisement fallbacks.
    if msg.destination_vmac == Some(BROADCAST_VMAC)
        || matches!(msg.originating_vmac, Some(UNKNOWN_VMAC | BROADCAST_VMAC))
    {
        return None;
    }
    // Shape faults draw their AB.3.1.5 code. A well-formed hub-local message
    // is unexpected here (no vendor function to serve); the generic
    // proprietary-unknown code applies as the local-matter response noted in
    // AB.2.16, without activity refresh.
    let code =
        proprietary_message_error(msg).unwrap_or(ErrorCode::BVLC_PROPRIETARY_FUNCTION_UNKNOWN);
    let marker = if code == ErrorCode::HEADER_NOT_UNDERSTOOD {
        first_must_understand_destination_option_marker(wire)?
    } else {
        0
    };
    let class = ErrorClass::COMMUNICATION.to_raw().to_be_bytes();
    let code = code.to_raw().to_be_bytes();
    Some(ScMessage {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc_frame::{decode_sc_bvlc_result, decode_sc_message, ScBvlcResult};

    fn decoded(function: u8, flags: u8, body: &[u8]) -> (ScMessage, Vec<u8>) {
        let mut wire = vec![function, flags, 0x22, 0x33];
        wire.extend_from_slice(body);
        let msg = decode_sc_message(&wire).unwrap();
        (msg, wire)
    }

    fn valid() -> Vec<u8> {
        vec![0x00, 0x2B, 0x42, 0xAA]
    }

    #[test]
    fn ignores_other_functions() {
        for function in [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x42] {
            let (msg, wire) = decoded(function, 0, &[]);
            assert!(local_nak(&msg, &wire).is_none());
        }
    }

    #[test]
    fn wellformed_local_draws_proprietary_unknown() {
        let (msg, wire) = decoded(12, 0, &valid());
        let nak = local_nak(&msg, &wire).unwrap();
        assert_eq!(
            decode_sc_bvlc_result(&nak).unwrap(),
            ScBvlcResult::Nak {
                result_for: ScFunction::ProprietaryMessage,
                error_header_marker: 0,
                error_class: 7,
                error_code: 144,
                error_details: String::new(),
            }
        );
        assert_eq!(nak.destination_vmac, None);
    }

    #[test]
    fn shape_faults_draw_shape_codes() {
        let (msg, wire) = decoded(12, 0, &[]);
        assert_eq!(local_nak(&msg, &wire).unwrap().payload[5..7], [0, 149]);
        let (msg, wire) = decoded(12, 0, &[0x00, 0x2B]);
        assert_eq!(local_nak(&msg, &wire).unwrap().payload[5..7], [0, 147]);
        let (msg, wire) = decoded(12, 1, &[0x01, 0x00, 0x2B, 0x42]);
        // Data Options present draws out-of-range, not incomplete.
        assert_eq!(local_nak(&msg, &wire).unwrap().payload[5..7], [0, 80]);
    }

    #[test]
    fn broadcast_and_reserved_origins_are_silent() {
        let mut wire = vec![12, 4, 0x22, 0x33];
        wire.extend_from_slice(&BROADCAST_VMAC);
        let msg = decode_sc_message(&wire).unwrap();
        assert!(local_nak(&msg, &wire).is_none());
        for origin in [UNKNOWN_VMAC, BROADCAST_VMAC] {
            let mut wire = vec![12, 8, 0x22, 0x33];
            wire.extend_from_slice(&origin);
            let msg = decode_sc_message(&wire).unwrap();
            assert!(local_nak(&msg, &wire).is_none());
        }
    }

    #[test]
    fn nak_targets_envelope_source() {
        let mut wire = vec![12, 8, 0x22, 0x33];
        wire.extend_from_slice(&[0x43; 6]);
        let msg = decode_sc_message(&wire).unwrap();
        let nak = local_nak(&msg, &wire).unwrap();
        assert_eq!(nak.destination_vmac, Some([0x43; 6]));
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &nak);
        assert_eq!(decode_sc_message(&buf).unwrap(), nak);
    }
}
