//! Established-node Address-Resolution admission, not answering, discovery,
//! or dialing.
//!
//! Well-formed request and response bodies are consumed silently with
//! accepted activity (no NPDU, no state change, no response yet). Malformed
//! request bodies draw the validator's diagnostic as a connection-local NAK
//! for locally-addressed unicast; malformed response bodies are always
//! silent because the response rule forbids answering responses. Envelope
//! routing (explicit destinations, reserved origins, broadcast silence)
//! matches the Advertisement and Proprietary gates.

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tracing::warn;

use super::data_attributes::build_bvlc_result_nak;
use super::rejection::{RejectionBudget, RejectionExpired};
use super::WebSocketPort;
use crate::sc_frame::{
    address_resolution_message_error, encode_sc_message,
    first_must_understand_destination_option_marker, ScFunction, ScMessage,
};

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    if !matches!(
        msg.function,
        ScFunction::AddressResolution | ScFunction::AddressResolutionAck
    ) {
        return Ok(false);
    }
    // Hub-connector AB.5.4: explicit destinations are not for the local
    // BVLL. Reserved origins are also silent; neither decision spends a
    // budget. Broadcast destinations arrive with an explicit address, so
    // the broadcast-silence rule needs no separate branch.
    if msg.destination_vmac.is_some()
        || matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == [0xFF; 6])
    {
        return Ok(true);
    }
    // Valid shapes are consumed silently with accepted activity. Answering
    // (empty versus populated response versus unsupported-function
    // diagnostic), discovery, and dialing remain later work; this gate
    // only rejects malformed bodies before dispatch.
    let Some(code) = address_resolution_message_error(msg) else {
        return Ok(false);
    };
    // Responses never draw a response, even when malformed. The malformed
    // response is discarded without activity, probe, or NPDU effects.
    if msg.function == ScFunction::AddressResolutionAck {
        return Ok(true);
    }
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
        warn!("BACnet/SC address-resolution NAK send error: {}", e);
    }
    Ok(true)
}
