//! Receive-only Advertisement / Advertisement-Solicitation admission after
//! generic syntax decoding.
//!
//! The wire shapes paraphrase ASHRAE 135-2020 Annex AB.2.8/AB.2.8.1 (unicast
//! Advertisement: no Data Options, six payload octets carrying hub-connection
//! status 0..2, accept-direct 0..1, and two opaque maximum lengths) and
//! AB.2.9/AB.2.9.1 (unicast Advertisement-Solicitation: no Data Options and
//! no defined payload). Error selection and multi-fault precedence interpret
//! AB.3.1.4 (Must Understand) and AB.3.1.5 (common errors); envelope routing
//! (broadcast/response silence, self-echo, hub-connector drops) stays with
//! the hub and node receivers, as for Connect and Control envelopes.

use super::{ScFunction, ScMessage};
use bacnet_types::enums::ErrorCode;

/// Advertisement payload length: hub-status(1) + accept-direct(1) +
/// maximum-BVLC(2) + maximum-NPDU(2).
pub(crate) const ADVERTISEMENT_PAYLOAD_LEN: usize = 6;

pub(crate) fn advertisement_message_error(msg: &ScMessage) -> Option<ErrorCode> {
    if !matches!(
        msg.function,
        ScFunction::Advertisement | ScFunction::AdvertisementSolicitation
    ) {
        return None;
    }
    // AB.2.8.1/AB.2.9.1 convey no Data Options. The envelope-option fault
    // precedes payload shape, matching the Connect and Control validators.
    if !msg.data_options.is_empty() {
        return Some(ErrorCode::PARAMETER_OUT_OF_RANGE);
    }
    if msg.function == ScFunction::AdvertisementSolicitation {
        // AB.2.9.1 defines no payload. Trailing bytes are inconsistent,
        // matching the Control policy for unexpected payloads.
        if !msg.payload.is_empty() {
            return Some(ErrorCode::INCONSISTENT_PARAMETERS);
        }
    } else {
        if msg.payload.is_empty() {
            return Some(ErrorCode::PAYLOAD_EXPECTED);
        }
        if msg.payload.len() < ADVERTISEMENT_PAYLOAD_LEN {
            return Some(ErrorCode::MESSAGE_INCOMPLETE);
        }
        if msg.payload.len() > ADVERTISEMENT_PAYLOAD_LEN {
            return Some(ErrorCode::INCONSISTENT_PARAMETERS);
        }
        // AB.2.8.1 enumerates hub-status 0..2 and accept-direct 0..1.
        // Maximum lengths stay opaque: no positive floor and no
        // BVLC/NPDU relationship is imposed here.
        if msg.payload[0] > 2 || msg.payload[1] > 1 {
            return Some(ErrorCode::PARAMETER_OUT_OF_RANGE);
        }
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

    fn message(function: ScFunction, payload: &[u8]) -> ScMessage {
        ScMessage {
            function,
            message_id: 0x2233,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::copy_from_slice(payload),
        }
    }

    fn valid_advertisement() -> Vec<u8> {
        vec![1, 1, 0x05, 0xC4, 0x05, 0xC4]
    }

    #[test]
    fn ignores_other_functions() {
        for raw in [
            0x00, 0x01, 0x02, 0x03, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
        ] {
            let msg = message(ScFunction::from_raw(raw), &[0xFF; 9]);
            assert_eq!(advertisement_message_error(&msg), None);
        }
        for raw in [0x0D, 0x42, 0xFF] {
            let msg = message(ScFunction::from_raw(raw), &[]);
            assert_eq!(advertisement_message_error(&msg), None);
        }
    }

    #[test]
    fn valid_shapes_pass_without_positive_floors() {
        for status in [0, 1, 2] {
            for direct in [0, 1] {
                for (bvlc, npdu) in [
                    (0u16, 0u16),
                    (1, 1),
                    (300, 1476),
                    (1476, 480),
                    (65535, 65535),
                ] {
                    let mut payload = vec![status, direct];
                    payload.extend_from_slice(&bvlc.to_be_bytes());
                    payload.extend_from_slice(&npdu.to_be_bytes());
                    assert_eq!(
                        advertisement_message_error(&message(ScFunction::Advertisement, &payload)),
                        None
                    );
                }
            }
        }
        assert_eq!(
            advertisement_message_error(&message(ScFunction::AdvertisementSolicitation, &[])),
            None
        );
    }

    #[test]
    fn generic_syntax_preserves_rejected_shapes() {
        // The generic codec stays permissive; this validator owns the verdict.
        let mut msg = message(ScFunction::Advertisement, &[9, 9, 0, 1, 0, 1]);
        msg.data_options.push(ScOption {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        });
        let mut encoded = BytesMut::new();
        encode_sc_message(&mut encoded, &msg);
        let decoded = decode_sc_message(&encoded).unwrap();
        assert_eq!(decoded.payload.as_ref(), &[9, 9, 0, 1, 0, 1]);
        assert_eq!(
            advertisement_message_error(&decoded),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
    }

    #[test]
    fn data_options_are_out_of_range_for_both_functions() {
        for function in [
            ScFunction::Advertisement,
            ScFunction::AdvertisementSolicitation,
        ] {
            let mut msg = if function == ScFunction::Advertisement {
                message(function, &valid_advertisement())
            } else {
                message(function, &[])
            };
            msg.data_options.push(ScOption {
                option_type: 1,
                must_understand: false,
                data: Vec::new(),
            });
            assert_eq!(
                advertisement_message_error(&msg),
                Some(ErrorCode::PARAMETER_OUT_OF_RANGE),
                "{function:?}"
            );
        }
    }

    #[test]
    fn advertisement_payload_length_matrix() {
        assert_eq!(
            advertisement_message_error(&message(ScFunction::Advertisement, &[])),
            Some(ErrorCode::PAYLOAD_EXPECTED)
        );
        for len in 1..6 {
            assert_eq!(
                advertisement_message_error(&message(ScFunction::Advertisement, &vec![1; len])),
                Some(ErrorCode::MESSAGE_INCOMPLETE),
                "len {len}"
            );
        }
        for len in [7, 8, 26, 64] {
            assert_eq!(
                advertisement_message_error(&message(ScFunction::Advertisement, &vec![1; len])),
                Some(ErrorCode::INCONSISTENT_PARAMETERS),
                "len {len}"
            );
        }
    }

    #[test]
    fn advertisement_field_ranges() {
        for status in [3, 4, 0x7F, 0xFF] {
            let mut payload = valid_advertisement();
            payload[0] = status;
            assert_eq!(
                advertisement_message_error(&message(ScFunction::Advertisement, &payload)),
                Some(ErrorCode::PARAMETER_OUT_OF_RANGE),
                "status {status}"
            );
        }
        for direct in [2, 3, 0x7F, 0xFF] {
            let mut payload = valid_advertisement();
            payload[1] = direct;
            assert_eq!(
                advertisement_message_error(&message(ScFunction::Advertisement, &payload)),
                Some(ErrorCode::PARAMETER_OUT_OF_RANGE),
                "direct {direct}"
            );
        }
    }

    #[test]
    fn solicitation_payload_is_inconsistent() {
        for payload in [&[0x00][..], &[1, 1, 5, 5][..], &[0xFF; 6][..]] {
            assert_eq!(
                advertisement_message_error(&message(
                    ScFunction::AdvertisementSolicitation,
                    payload
                )),
                Some(ErrorCode::INCONSISTENT_PARAMETERS)
            );
        }
    }

    #[test]
    fn must_understand_is_last_precedence() {
        // Payload faults win over the MU fault, as for Connect diagnostics.
        let mu = ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        };
        let mut short = message(ScFunction::Advertisement, &[1, 1, 0]);
        short.dest_options.push(mu.clone());
        assert_eq!(
            advertisement_message_error(&short),
            Some(ErrorCode::MESSAGE_INCOMPLETE)
        );
        let mut long = message(ScFunction::AdvertisementSolicitation, &[0x42]);
        long.dest_options.push(mu.clone());
        assert_eq!(
            advertisement_message_error(&long),
            Some(ErrorCode::INCONSISTENT_PARAMETERS)
        );
        let mut valid = message(ScFunction::Advertisement, &valid_advertisement());
        valid.dest_options.push(mu);
        assert_eq!(
            advertisement_message_error(&valid),
            Some(ErrorCode::HEADER_NOT_UNDERSTOOD)
        );
        // Non-MU destination options are ignored here.
        let mut ignored = message(ScFunction::AdvertisementSolicitation, &[]);
        ignored.dest_options.push(ScOption {
            option_type: 31,
            must_understand: false,
            data: vec![0xAA],
        });
        assert_eq!(advertisement_message_error(&ignored), None);
    }
}
