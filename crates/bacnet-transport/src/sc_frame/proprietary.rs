//! Receive-only Proprietary-Message admission after generic syntax decoding.
//!
//! The wire shapes paraphrase ASHRAE 135-2020 Annex AB.2.16/AB.2.16.1
//! (unicast-or-broadcast Proprietary-Message: no Data Options, payload of
//! at least vendor identifier plus vendor function, optional vendor data)
//! Error selection and multi-fault precedence interpret AB.3.1.4 (Must
//! Understand) and AB.3.1.5 (common errors); envelope routing (broadcast
//! silence, self-echo, hub-connector drops) stays with the hub and node
//! receivers, as for Advertisement and Unknown envelopes.

use super::{ScFunction, ScMessage};
use bacnet_types::enums::ErrorCode;

/// Minimum Proprietary-Message payload: vendor identifier (2) + function (1).
pub(crate) const PROPRIETARY_MIN_PAYLOAD_LEN: usize = 3;

pub(crate) fn proprietary_message_error(msg: &ScMessage) -> Option<ErrorCode> {
    if msg.function != ScFunction::ProprietaryMessage {
        return None;
    }
    // AB.2.16.1 conveys no Data Options. The envelope-option fault
    // precedes payload shape, matching the Advertisement validator.
    if !msg.data_options.is_empty() {
        return Some(ErrorCode::PARAMETER_OUT_OF_RANGE);
    }
    // AB.2.16.1 requires at least vendor identifier and function octet.
    // Vendor values stay opaque: no identifier floor and no function range
    // is imposed here.
    if msg.payload.is_empty() {
        return Some(ErrorCode::PAYLOAD_EXPECTED);
    }
    if msg.payload.len() < PROPRIETARY_MIN_PAYLOAD_LEN {
        return Some(ErrorCode::MESSAGE_INCOMPLETE);
    }
    if msg.dest_options.iter().any(|option| option.must_understand) {
        // Known-option shape/placement validation remains separate work.
        return Some(ErrorCode::HEADER_NOT_UNDERSTOOD);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc_frame::{decode_sc_message, encode_sc_message, ScOption};
    use bytes::{Bytes, BytesMut};

    fn message(payload: &[u8]) -> ScMessage {
        ScMessage {
            function: ScFunction::ProprietaryMessage,
            message_id: 0x2233,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::copy_from_slice(payload),
        }
    }

    fn valid_proprietary() -> Vec<u8> {
        vec![0x00, 0x2B, 0x42, 0xAA, 0xBB]
    }

    #[test]
    fn ignores_other_functions() {
        for raw in [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
        ] {
            let msg = ScMessage {
                function: ScFunction::from_raw(raw),
                message_id: 0x2233,
                originating_vmac: None,
                destination_vmac: None,
                dest_options: Vec::new(),
                data_options: Vec::new(),
                payload: Bytes::copy_from_slice(&[0xFF; 9]),
            };
            assert_eq!(proprietary_message_error(&msg), None);
        }
        for raw in [0x0D, 0x42, 0xFF] {
            let msg = ScMessage {
                function: ScFunction::from_raw(raw),
                message_id: 0x2233,
                originating_vmac: None,
                destination_vmac: None,
                dest_options: Vec::new(),
                data_options: Vec::new(),
                payload: Bytes::new(),
            };
            assert_eq!(proprietary_message_error(&msg), None);
        }
    }

    #[test]
    fn valid_shapes_pass_without_vendor_floors() {
        for payload in [
            vec![0x00, 0x00, 0x00],
            vec![0xFF, 0xFF, 0xFF],
            vec![0x00, 0x2B, 0x42],
            valid_proprietary(),
            vec![0x12, 0x34, 0x56, 0x00, 0x01, 0x02, 0x03, 0xFF],
        ] {
            assert_eq!(proprietary_message_error(&message(&payload)), None);
        }
        // Non-MU destination options are ignored here.
        let mut ignored = message(&valid_proprietary());
        ignored.dest_options.push(ScOption {
            option_type: 31,
            must_understand: false,
            data: vec![0xAA],
        });
        assert_eq!(proprietary_message_error(&ignored), None);
    }

    #[test]
    fn generic_syntax_preserves_rejected_shapes() {
        // The generic codec stays permissive; this validator owns the verdict.
        let mut msg = message(&[0x00, 0x2B]);
        msg.data_options.push(ScOption {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        });
        let mut encoded = BytesMut::new();
        encode_sc_message(&mut encoded, &msg);
        let decoded = decode_sc_message(&encoded).unwrap();
        assert_eq!(decoded.payload.as_ref(), &[0x00, 0x2B]);
        assert_eq!(
            proprietary_message_error(&decoded),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
    }

    #[test]
    fn data_options_are_out_of_range() {
        let mut msg = message(&valid_proprietary());
        msg.data_options.push(ScOption {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        });
        assert_eq!(
            proprietary_message_error(&msg),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
    }

    #[test]
    fn payload_length_matrix() {
        assert_eq!(
            proprietary_message_error(&message(&[])),
            Some(ErrorCode::PAYLOAD_EXPECTED)
        );
        for len in [1, 2] {
            assert_eq!(
                proprietary_message_error(&message(&vec![0x2B; len])),
                Some(ErrorCode::MESSAGE_INCOMPLETE),
                "len {len}"
            );
        }
        for len in [3, 4, 26, 64] {
            assert_eq!(
                proprietary_message_error(&message(&vec![0x2B; len])),
                None,
                "len {len}"
            );
        }
    }

    #[test]
    fn must_understand_is_last_precedence() {
        // Payload faults win over the MU fault, as for Advertisement diagnostics.
        let mu = ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        };
        let mut empty = message(&[]);
        empty.dest_options.push(mu.clone());
        assert_eq!(
            proprietary_message_error(&empty),
            Some(ErrorCode::PAYLOAD_EXPECTED)
        );
        let mut short = message(&[0x00, 0x2B]);
        short.dest_options.push(mu.clone());
        assert_eq!(
            proprietary_message_error(&short),
            Some(ErrorCode::MESSAGE_INCOMPLETE)
        );
        let mut data_optioned = message(&valid_proprietary());
        data_optioned.data_options.push(ScOption {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        });
        data_optioned.dest_options.push(mu.clone());
        assert_eq!(
            proprietary_message_error(&data_optioned),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
        let mut valid = message(&valid_proprietary());
        valid.dest_options.push(mu);
        assert_eq!(
            proprietary_message_error(&valid),
            Some(ErrorCode::HEADER_NOT_UNDERSTOOD)
        );
    }
}
