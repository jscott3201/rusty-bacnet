//! Receive-only Address-Resolution / Address-Resolution-ACK admission after
//! generic syntax decoding.
//!
//! The wire shapes paraphrase the local Standard's Annex AB request and
//! response formats: the request carries no defined payload and no Data
//! Options, while the response carries a variable UTF-8 list of direct-
//! connection URIs separated by single spaces, with an empty zero-octet
//! list valid. Error selection and multi-fault precedence interpret the
//! common exchange rules for header options and field faults; envelope
//! routing (broadcast/response silence, hub-connector drops) stays with
//! the hub and node receivers, as for Advertisement and Proprietary.
//!
//! Answering (empty versus populated response versus unsupported-function
//! diagnostic), discovery, and dialing remain later work. This validator
//! only distinguishes well-formed bodies from malformed ones so the node
//! can reject the latter before dispatch while preserving valid consume.

use super::{ScFunction, ScMessage};
use bacnet_types::enums::ErrorCode;

/// Validate an Address-Resolution family message, returning the diagnostic
/// to NAK with (or to silently discard for responses) when malformed.
///
/// Returns `None` for other functions and for well-formed request/response
/// shapes. Data-option presence precedes payload shape, and payload faults
/// precede the Must-Understand fault, matching the Advertisement and
/// Proprietary validators.
pub(crate) fn address_resolution_message_error(msg: &ScMessage) -> Option<ErrorCode> {
    if !matches!(
        msg.function,
        ScFunction::AddressResolution | ScFunction::AddressResolutionAck
    ) {
        return None;
    }
    // Both formats convey no Data Options. The envelope-option fault
    // precedes payload shape, matching the sibling validators.
    if !msg.data_options.is_empty() {
        return Some(ErrorCode::PARAMETER_OUT_OF_RANGE);
    }
    if msg.function == ScFunction::AddressResolution {
        // No payload is defined for the request. Trailing bytes are
        // inconsistent, matching the solicitation policy for unexpected
        // payloads.
        if !msg.payload.is_empty() {
            return Some(ErrorCode::INCONSISTENT_PARAMETERS);
        }
    } else {
        // Empty zero-octet lists are valid responses.
        if !msg.payload.is_empty() {
            // The list is a UTF-8 string. Non-UTF-8 bytes are a data
            // inconsistency, not a header-encoding fault.
            let text = match core::str::from_utf8(&msg.payload) {
                Ok(text) => text,
                Err(_) => return Some(ErrorCode::INCONSISTENT_PARAMETERS),
            };
            if let Some(code) = uri_list_structure_error(text) {
                return Some(code);
            }
        }
    }
    if msg.dest_options.iter().any(|option| option.must_understand) {
        // Known-option shape/placement validation remains separate work.
        return Some(ErrorCode::HEADER_NOT_UNDERSTOOD);
    }
    None
}

/// Structural faults in the response URI list.
///
/// Empty tokens from leading, trailing, or doubled separators are data
/// inconsistencies. Per-token scheme, host, and character faults are field
/// range faults. The caller checks UTF-8 and Must-Understand separately.
fn uri_list_structure_error(text: &str) -> Option<ErrorCode> {
    if text.is_empty() {
        return None;
    }
    // A single space separates entries. Leading, trailing, or doubled
    // separators would create an empty entry.
    if text.starts_with(' ') || text.ends_with(' ') || text.contains("  ") {
        return Some(ErrorCode::INCONSISTENT_PARAMETERS);
    }
    for token in text.split(' ') {
        if !is_valid_wss_uri(token) {
            return Some(ErrorCode::PARAMETER_OUT_OF_RANGE);
        }
    }
    None
}

/// Minimal direct-connection URI shape check.
///
/// Requires the secure WebSocket scheme, a present host, and visible ASCII
/// characters only. This is an endpoint-local range check for the response
/// list; hub forwarding stays opaque and discovery/dial selection remains
/// later work. No length floor or ceiling beyond the BVLC envelope is
/// imposed here.
pub(crate) fn is_valid_wss_uri(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    // Visible ASCII excluding space only. This rejects controls, DEL,
    // and non-ASCII bytes that survived UTF-8 decoding.
    if !token.bytes().all(|b| (0x21..=0x7E).contains(&b)) {
        return false;
    }
    // Scheme is case-insensitive, matching the hub-connector dial policy.
    if token.len() < 7 || !token[..6].eq_ignore_ascii_case("wss://") {
        return false;
    }
    let after_scheme = &token[6..];
    if after_scheme.is_empty() {
        return false;
    }
    // Host runs to the next '/', '?', or '#' or to the end. It must be
    // present; userinfo and port details stay opaque beyond that.
    let host_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let host = &after_scheme[..host_end];
    if host.is_empty() {
        return false;
    }
    // A bare authority terminator with no host (for example `wss:///`)
    // is already caught by the empty-host check above.
    true
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

    #[test]
    fn ignores_other_functions() {
        for raw in [
            0x00, 0x01, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
        ] {
            let msg = message(ScFunction::from_raw(raw), &[0xFF; 9]);
            assert_eq!(address_resolution_message_error(&msg), None);
        }
        for raw in [0x0D, 0x42, 0xFF] {
            let msg = message(ScFunction::from_raw(raw), &[]);
            assert_eq!(address_resolution_message_error(&msg), None);
        }
    }

    #[test]
    fn request_valid_shape_is_empty_only() {
        assert_eq!(
            address_resolution_message_error(&message(ScFunction::AddressResolution, &[])),
            None
        );
        // Non-MU destination options are ignored here.
        let mut ignored = message(ScFunction::AddressResolution, &[]);
        ignored.dest_options.push(ScOption {
            option_type: 31,
            must_understand: false,
            data: vec![0xAA],
        });
        assert_eq!(address_resolution_message_error(&ignored), None);
    }

    #[test]
    fn request_extra_payload_is_inconsistent() {
        for payload in [
            &[0x00][..],
            &[1, 1, 5, 5][..],
            b"wss://one.example/sc".as_slice(),
        ] {
            assert_eq!(
                address_resolution_message_error(&message(ScFunction::AddressResolution, payload)),
                Some(ErrorCode::INCONSISTENT_PARAMETERS)
            );
        }
    }

    #[test]
    fn request_data_options_are_out_of_range() {
        let mut msg = message(ScFunction::AddressResolution, &[]);
        msg.data_options.push(ScOption {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        });
        assert_eq!(
            address_resolution_message_error(&msg),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
        // Data-option fault precedes payload shape.
        let mut both = message(ScFunction::AddressResolution, &[0x42]);
        both.data_options.push(ScOption {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        });
        both.dest_options.push(ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        });
        assert_eq!(
            address_resolution_message_error(&both),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
    }

    #[test]
    fn ack_empty_list_is_valid() {
        assert_eq!(
            address_resolution_message_error(&message(ScFunction::AddressResolutionAck, &[])),
            None
        );
    }

    #[test]
    fn ack_valid_uri_vectors() {
        for payload in [
            b"wss://one.example/sc".as_slice(),
            b"wss://one.example/sc wss://two.example:8443/sc".as_slice(),
            b"wss://hub.example.com:47808/.bacnet/sc?profile=primary".as_slice(),
            b"wss://[::1]:47808/sc".as_slice(),
            b"WSS://UPPER.example/SC".as_slice(),
            b"wss://a.example/1 wss://b.example/2 wss://c.example/3".as_slice(),
        ] {
            assert_eq!(
                address_resolution_message_error(&message(
                    ScFunction::AddressResolutionAck,
                    payload
                )),
                None,
                "payload {payload:?}"
            );
        }
        // Boundary: a long but well-formed list stays valid; length caps
        // belong to the BVLC envelope, not this validator.
        let mut long = b"wss://peer.example/".to_vec();
        long.resize(1400, b'x');
        assert_eq!(
            address_resolution_message_error(&message(ScFunction::AddressResolutionAck, &long)),
            None
        );
    }

    #[test]
    fn ack_non_utf8_is_inconsistent() {
        for payload in [
            &[0xFF, 0x00][..],
            &[0xC3, 0x28][..],
            b"wss://\xFF.example/sc".as_slice(),
        ] {
            assert_eq!(
                address_resolution_message_error(&message(
                    ScFunction::AddressResolutionAck,
                    payload
                )),
                Some(ErrorCode::INCONSISTENT_PARAMETERS),
                "payload {payload:?}"
            );
        }
    }

    #[test]
    fn ack_separator_faults_are_inconsistent() {
        for payload in [
            b" wss://one.example/sc".as_slice(),
            b"wss://one.example/sc ".as_slice(),
            b"wss://one.example/sc  wss://two.example/sc".as_slice(),
            b" ".as_slice(),
            b"  ".as_slice(),
        ] {
            assert_eq!(
                address_resolution_message_error(&message(
                    ScFunction::AddressResolutionAck,
                    payload
                )),
                Some(ErrorCode::INCONSISTENT_PARAMETERS),
                "payload {payload:?}"
            );
        }
    }

    #[test]
    fn ack_scheme_host_and_character_faults_are_out_of_range() {
        for payload in [
            b"ws://one.example/sc".as_slice(),
            b"https://one.example/sc".as_slice(),
            b"wss://".as_slice(),
            b"wss:///sc".as_slice(),
            b"wss://?query".as_slice(),
            b"not-a-uri".as_slice(),
            b"wss://one.example/sc http://two.example/sc".as_slice(),
            b"wss://one.example/sc wss://".as_slice(),
            "wss://one.example/sc\u{00e9}".as_bytes(),
            b"wss://one.example/sc\t".as_slice(),
        ] {
            assert_eq!(
                address_resolution_message_error(&message(
                    ScFunction::AddressResolutionAck,
                    payload
                )),
                Some(ErrorCode::PARAMETER_OUT_OF_RANGE),
                "payload {payload:?}"
            );
        }
    }

    #[test]
    fn ack_data_options_are_out_of_range() {
        let mut msg = message(ScFunction::AddressResolutionAck, b"wss://one.example/sc");
        msg.data_options.push(ScOption {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        });
        assert_eq!(
            address_resolution_message_error(&msg),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
    }

    #[test]
    fn must_understand_is_last_precedence() {
        let mu = ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        };
        // Payload faults win over the MU fault, as for sibling validators.
        let mut request = message(ScFunction::AddressResolution, &[0x42]);
        request.dest_options.push(mu.clone());
        assert_eq!(
            address_resolution_message_error(&request),
            Some(ErrorCode::INCONSISTENT_PARAMETERS)
        );
        let mut ack_structure = message(ScFunction::AddressResolutionAck, b" wss://a.example/");
        ack_structure.dest_options.push(mu.clone());
        assert_eq!(
            address_resolution_message_error(&ack_structure),
            Some(ErrorCode::INCONSISTENT_PARAMETERS)
        );
        let mut ack_range = message(ScFunction::AddressResolutionAck, b"ws://a.example/");
        ack_range.dest_options.push(mu.clone());
        assert_eq!(
            address_resolution_message_error(&ack_range),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
        let mut valid = message(ScFunction::AddressResolutionAck, b"wss://a.example/");
        valid.dest_options.push(mu);
        assert_eq!(
            address_resolution_message_error(&valid),
            Some(ErrorCode::HEADER_NOT_UNDERSTOOD)
        );
    }

    #[test]
    fn generic_syntax_preserves_rejected_shapes() {
        // The generic codec stays permissive; this validator owns the verdict.
        let mut msg = message(ScFunction::AddressResolution, &[0x42]);
        msg.data_options.push(ScOption {
            option_type: 1,
            must_understand: false,
            data: Vec::new(),
        });
        let mut encoded = BytesMut::new();
        encode_sc_message(&mut encoded, &msg);
        let decoded = decode_sc_message(&encoded).unwrap();
        assert_eq!(decoded.payload.as_ref(), &[0x42]);
        assert_eq!(
            address_resolution_message_error(&decoded),
            Some(ErrorCode::PARAMETER_OUT_OF_RANGE)
        );
    }
}
