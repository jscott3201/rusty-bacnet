//! Established-node Advertisement/Solicitation admission plus the accepted
//! solicitation predicate and solicited-reply rate policy. Generic codec
//! validation lives in `sc_frame`; received-Advertisement peer status
//! tracking is not kept (no local peer store); solicited replies are
//! originated by the transport loop, keeping `ScConnection::handle_received`
//! pure.

use std::time::Duration;

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tracing::warn;

use super::data_attributes::build_bvlc_result_nak;
use super::rejection::{RejectionBudget, RejectionExpired};
use super::WebSocketPort;
use crate::sc_frame::{
    advertisement_message_error, encode_sc_message,
    first_must_understand_destination_option_marker, ScFunction, ScMessage, Vmac, BROADCAST_VMAC,
};

/// Minimum spacing between solicited Advertisements answered by this node.
///
/// Local anti-storm policy (not a wire deadline): floods of solicitations
/// collapse to at most one Advertisement per interval. Chosen as a plain
/// constant rather than a RejectionBudget because the reply is a solicited
/// positive transmission, not a rejection NAK; the send itself stays
/// best-effort like Heartbeat-ACK.
pub(super) const SOLICITED_ADVERTISEMENT_MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Accepted-solicitation predicate for AB.3.2 solicited replies.
///
/// Returns the reply destination (the solicitation origin, or `None` for a
/// hub-peer solicitation so the reply stays peer-addressed) when the frame
/// is a well-formed, locally-addressed solicitation. Malformed shapes keep
/// the existing NAK path; addressed or reserved-origin envelopes stay
/// silent; other functions are not solicitations.
pub(super) fn solicited_advertisement_destination(msg: &ScMessage) -> Option<Option<Vmac>> {
    if msg.function != ScFunction::AdvertisementSolicitation {
        return None;
    }
    if advertisement_message_error(msg).is_some() {
        return None;
    }
    if msg.destination_vmac.is_some() {
        return None;
    }
    if matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == BROADCAST_VMAC) {
        return None;
    }
    Some(msg.originating_vmac)
}

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    if !matches!(
        msg.function,
        ScFunction::Advertisement | ScFunction::AdvertisementSolicitation
    ) {
        return Ok(false);
    }
    // Hub-connector AB.5.4: explicit destinations are not for the local BVLL.
    // Reserved origins are also silent; neither decision spends a budget.
    if msg.destination_vmac.is_some()
        || matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == BROADCAST_VMAC)
    {
        return Ok(true);
    }
    // Valid shapes are consumed silently with accepted activity (no NPDU, no
    // state change). Solicited Advertisement transmissions for accepted
    // solicitations are originated by the transport loop, not admission.
    let Some(code) = advertisement_message_error(msg) else {
        return Ok(false);
    };
    let marker = if code == ErrorCode::HEADER_NOT_UNDERSTOOD {
        match first_must_understand_destination_option_marker(wire) {
            Some(marker) => marker,
            None => return Ok(true),
        }
    } else {
        0
    };
    let nak = build_bvlc_result_nak(
        msg.message_id,
        msg.function,
        marker,
        msg.originating_vmac,
        ErrorClass::COMMUNICATION,
        code,
    );
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &nak);
    if let Err(e) = budget.send(ws, &bytes).await? {
        warn!("BACnet/SC advertisement NAK send error: {}", e);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc_frame::decode_sc_message;

    fn solicitation(origin: Option<Vmac>, dest: Option<Vmac>, payload: &[u8]) -> ScMessage {
        ScMessage {
            function: ScFunction::AdvertisementSolicitation,
            message_id: 0x2233,
            originating_vmac: origin,
            destination_vmac: dest,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: bytes::Bytes::copy_from_slice(payload),
        }
    }

    #[test]
    fn predicate_accepts_only_locally_addressed_valid_solicitations() {
        assert_eq!(
            solicited_advertisement_destination(&solicitation(None, None, &[])),
            Some(None)
        );
        assert_eq!(
            solicited_advertisement_destination(&solicitation(Some([0x22; 6]), None, &[])),
            Some(Some([0x22; 6]))
        );
    }

    #[test]
    fn predicate_rejects_malformed_addressed_and_reserved_solicitations() {
        // Malformed shapes keep the NAK path (no reply destination).
        assert_eq!(
            solicited_advertisement_destination(&solicitation(None, None, &[0x00])),
            None
        );
        // Explicit destinations are not for the local BVLL.
        assert_eq!(
            solicited_advertisement_destination(&solicitation(None, Some([0x44; 6]), &[])),
            None
        );
        assert_eq!(
            solicited_advertisement_destination(&solicitation(
                Some([0x22; 6]),
                Some([0x44; 6]),
                &[]
            )),
            None
        );
        // Reserved origins stay silent.
        for origin in [Some([0; 6]), Some(BROADCAST_VMAC)] {
            assert_eq!(
                solicited_advertisement_destination(&solicitation(origin, None, &[])),
                None
            );
        }
        // Must-Understand destination options are faults, not replies.
        let mut mu = solicitation(None, None, &[]);
        mu.dest_options.push(crate::sc_frame::ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        });
        assert_eq!(solicited_advertisement_destination(&mu), None);
    }

    #[test]
    fn predicate_ignores_other_functions() {
        for function in [
            ScFunction::Advertisement,
            ScFunction::EncapsulatedNpdu,
            ScFunction::HeartbeatRequest,
        ] {
            let mut msg = solicitation(None, None, &[]);
            msg.function = function;
            assert_eq!(solicited_advertisement_destination(&msg), None);
        }
    }

    #[test]
    fn predicate_agrees_with_wire_codec() {
        // The predicate accepts exactly the wire shapes the rejection gate
        // lets through as valid solicitations.
        let wire_valid = [0x05u8, 0x00, 0x22, 0x33];
        let msg = decode_sc_message(&wire_valid).unwrap();
        assert_eq!(solicited_advertisement_destination(&msg), Some(None));
        let wire_payload = [0x05u8, 0x00, 0x22, 0x33, 0x00];
        let msg = decode_sc_message(&wire_payload).unwrap();
        assert_eq!(solicited_advertisement_destination(&msg), None);
    }
}
